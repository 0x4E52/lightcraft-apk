//! An in-app file chooser for hosts without system file dialogs (Android, where they need Java).
//!
//! It browses only the folders the host names in [`Services::pick_roots`] (Android: the Inbox
//! that shares fill, and the library), and [`crate::pick::ask`] opens it when the host has no
//! [`crate::pick::Picker`] and names roots; the desktop and the web name none and are unchanged.
//! Its answer goes through the same channel as a system dialog's, so the command it was opened
//! for runs again with the chosen paths ([`crate::pick::poll`]).
//!
//! Every path it shows or answers stays inside a root: a listing leaves out symbolic links and
//! hidden entries, and [`inside_roots`] refuses `..` and any link on the way down, so a link
//! planted in the Inbox can't lead out of it.
//!
//! A command whose file can't live under those roots is refused with why ([`unavailable`]): a
//! saved file would stay inside LightCraft where other apps can't open it, and a Lightroom
//! catalog points at originals on the computer it came from.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{RichText, vec2};

use crate::pick::{PickKind, PickRequest};
use crate::{LightcraftApp, Services};

/// The most entries one folder lists (a folder with more says so).
pub const MAX_ENTRIES: usize = 5000;
/// The longest file name a save takes, in bytes.
const MAX_NAME_BYTES: usize = 120;

/// What a disabled command says where the in-app chooser stands in for file dialogs.
pub const SAVE_REASON: &str =
    "Not available on Android: LightCraft can't save this file yet where other apps can open it; a file saved here would stay inside LightCraft";
/// File ▸ Import Lightroom Catalog… on Android.
pub const LIGHTROOM_REASON: &str = "Not available on Android: a Lightroom Classic catalog points to originals on the computer it came from; import it in LightCraft on that computer";
/// File ▸ New Library… and Open Library… on Android.
pub const ONE_LIBRARY_REASON: &str = "Not available on Android: LightCraft keeps one library, inside the app";
/// Settings ▸ Import ▸ Auto Import ▸ Watched folder Choose… on Android.
pub const AUTO_IMPORT_REASON: &str = "Not available on Android: other apps can't save photos into a folder LightCraft can watch; share them to LightCraft or use Import from Photo Library…";
/// Settings ▸ Display ▸ Choose Profile… on Android.
pub const DISPLAY_PROFILE_REASON: &str = "Not available on Android: a monitor profile file can't be chosen here; previews are shown as sRGB";
/// Settings ▸ Denoise ▸ Install from file… on Android.
pub const DENOISE_FILE_REASON: &str = "Not available on Android yet: install a denoise model from the list below";
/// Said where a folder is empty: how files get into the Inbox.
pub const INBOX_HINT: &str = "Share a file to LightCraft from Files or the browser, then pick it from Inbox.";

/// The in-app chooser stands in for file dialogs: the host has no system dialog for commands
/// ([`Services::picker`]) and names the folders it may browse.
pub fn in_use(services: &Services) -> bool {
    services.picker.is_none() && !services.pick_roots.is_empty()
}

/// Why `command` can't run where the in-app chooser stands in for file dialogs (`None`: it can,
/// or the host has its own dialogs). Its menu item is disabled with this as the tooltip, and the
/// command (from the control channel or MCP too) is refused with it.
pub fn unavailable(services: &Services, command: &str) -> Option<&'static str> {
    if !in_use(services) {
        return None;
    }
    match command {
        "file.exportPresets"
        | "file.exportMaskEffects"
        | "file.exportCurvePresets"
        | "file.exportKeywords"
        | "app.print"
        | "dialog.print"
        | "app.contactSheet"
        | "dialog.contactSheet"
        | "app.slideshowExport"
        | "dialog.slideshowExport" => Some(SAVE_REASON),
        "file.importLightroom" => Some(LIGHTROOM_REASON),
        "app.newLibrary" | "app.openLibrary" => Some(ONE_LIBRARY_REASON),
        _ => None,
    }
}

/// [`unavailable`] for `app`: the library problem window's Choose Another Library… still runs
/// `app.openLibrary`, with [`check_library_choice`] limiting it to an existing library.
pub fn refusal(app: &LightcraftApp, command: &str) -> Option<&'static str> {
    if command == "app.openLibrary" && app.library_problem.is_some() {
        return None;
    }
    unavailable(&app.services, command)
}

/// Where the in-app chooser stands in for file dialogs, a folder chosen to open as the library
/// must already be a LightCraft library (it holds a catalog), not inside another one, and not
/// `current` (the library that couldn't be opened: Try Again opens it again), so Choose never
/// creates a library in the Inbox or inside the library.
pub fn check_library_choice(path: &Path, current: Option<&Path>) -> Result<(), String> {
    let shown = path.display();
    if !lightcraft_engine::library::is_library(path) {
        return Err(format!("{shown}: no LightCraft library here; choose a folder that holds one"));
    }
    if path.ancestors().skip(1).any(lightcraft_engine::library::is_library) {
        return Err(format!("{shown}: inside another library; choose the library's own folder"));
    }
    let same = |c: &Path| c == path || c.canonicalize().ok().zip(path.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    if current.is_some_and(same) {
        return Err(format!("{shown}: this is the library that couldn't be opened; Try Again opens it again"));
    }
    Ok(())
}

/// `path` as the real path under one of `roots`, when it is one: every component below the root
/// must be a plain name of an entry that exists and is not a symbolic link (no `..`). A
/// root that doesn't exist matches nothing.
pub fn inside_roots(roots: &[(String, PathBuf)], path: &Path) -> Result<PathBuf, String> {
    for (_, root) in roots {
        let Ok(real) = root.canonicalize() else { continue };
        let Some(rest) = path.strip_prefix(root).ok().or_else(|| path.strip_prefix(&real).ok()) else { continue };
        let mut at = real;
        for c in rest.components() {
            let Component::Normal(name) = c else {
                return Err(format!("{}: only folders inside the listed places can be opened", path.display()));
            };
            at.push(name);
            let meta = std::fs::symlink_metadata(&at).map_err(|e| format!("{}: {e}", at.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{}: links are not followed here", path.display()));
            }
        }
        return Ok(at);
    }
    Err(format!("{}: outside the places LightCraft can open here", path.display()))
}

/// One row of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

/// Whether `name` ends in one of `extensions` (any case; no extensions = any file).
fn has_extension(name: &str, extensions: &[&str]) -> bool {
    if extensions.is_empty() {
        return true;
    }
    let lower = name.to_lowercase();
    extensions.iter().any(|e| lower.rsplit_once('.').is_some_and(|(_, x)| x == e.to_lowercase()))
}

/// The folders and the files `request` can choose in `dir`, folders first, by name; symbolic
/// links, hidden entries and names that aren't text are left out. The flag says the folder had
/// more than [`MAX_ENTRIES`].
pub fn list(dir: &Path, request: &PickRequest) -> Result<(Vec<Entry>, bool), String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let extensions = request.filter.as_ref().map_or(&[][..], |f| f.1);
    let mut out = Vec::new();
    let mut more = false;
    for entry in read.flatten() {
        if out.len() >= MAX_ENTRIES {
            more = true;
            break;
        }
        // (the entry's own type: a link is a link, not what it points to)
        let Ok(kind) = entry.file_type() else { continue };
        let Ok(name) = entry.file_name().into_string() else { continue };
        if kind.is_symlink() || name.starts_with('.') {
            continue;
        }
        let dir = kind.is_dir();
        let wanted = dir || (kind.is_file() && request.kind != PickKind::Folder && has_extension(&name, extensions));
        if wanted {
            out.push(Entry { name, dir });
        }
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok((out, more))
}

/// A file name typed for a save, checked: one plain name (no folder, `..`, control characters or
/// leading dot), at most [`MAX_NAME_BYTES`], with the filter's first extension added when it has
/// none of them.
pub fn save_name(typed: &str, extensions: &[&str]) -> Result<String, String> {
    let name = typed.trim();
    if name.is_empty() {
        return Err("Type a file name".into());
    }
    if name.starts_with('.') || name.chars().any(|c| matches!(c, '/' | '\\' | '\0') || c.is_control()) {
        return Err(format!("“{name}” can't be a file name here"));
    }
    let mut name = name.to_string();
    if let Some(first) = extensions.first()
        && !has_extension(&name, extensions)
    {
        name = format!("{name}.{first}");
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(format!("The name is longer than {MAX_NAME_BYTES} bytes"));
    }
    Ok(name)
}

/// The chooser while it is up.
pub struct Browser {
    request: PickRequest,
    roots: Vec<(String, PathBuf)>,
    /// The folder shown (a real path inside a root); `None` = the list of roots.
    dir: Option<PathBuf>,
    entries: Vec<Entry>,
    /// The folder had more entries than it lists.
    more: bool,
    /// Files chosen in `dir`.
    chosen: Vec<String>,
    /// [`PickKind::Save`]: the name typed.
    name: String,
    error: Option<String>,
    answer: Sender<Vec<String>>,
}

impl Browser {
    /// The folder shown, `None` for the list of places (`ui.inspect`, tests).
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The names listed in the folder shown.
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Show `path` (a folder inside a root); a path outside them, or one that can't be listed,
    /// leaves the folder shown as it was and says why.
    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        let result = inside_roots(&self.roots, path).and_then(|real| {
            if !real.is_dir() {
                return Err(format!("{}: not a folder", path.display()));
            }
            let (entries, more) = list(&real, &self.request)?;
            Ok((real, entries, more))
        });
        match result {
            Ok((real, entries, more)) => {
                self.dir = Some(real);
                self.entries = entries;
                self.more = more;
                self.chosen.clear();
                self.error = None;
                Ok(())
            }
            Err(e) => {
                self.error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Back to the list of places.
    pub fn show_places(&mut self) {
        self.dir = None;
        self.entries.clear();
        self.chosen.clear();
        self.error = None;
    }

    /// The folder above the one shown, or the places from a root.
    fn up(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        let at_root = self.roots.iter().any(|(_, r)| r.canonicalize().is_ok_and(|r| r == dir));
        match dir.parent() {
            Some(parent) if !at_root => {
                let _ = self.open(parent);
            }
            _ => self.show_places(),
        }
    }

    /// Choose (or un-choose) the file `name` of the folder shown.
    pub fn toggle(&mut self, name: &str) {
        if let Some(i) = self.chosen.iter().position(|c| c == name) {
            self.chosen.remove(i);
        } else if self.request.kind == PickKind::Files {
            self.chosen.push(name.to_string());
        } else {
            self.chosen = vec![name.to_string()];
        }
        if self.request.kind == PickKind::Save {
            self.name = name.to_string();
            self.chosen.clear();
        }
    }

    /// Set the name a save takes.
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    /// The paths the confirm button answers, each checked again to be inside a root (`Err`: why
    /// nothing can be answered yet).
    pub fn answer(&self) -> Result<Vec<String>, String> {
        let Some(dir) = &self.dir else { return Err("Choose a place first".into()) };
        let text = |p: &Path| p.to_string_lossy().into_owned();
        match self.request.kind {
            PickKind::Folder => Ok(vec![text(&inside_roots(&self.roots, dir)?)]),
            PickKind::Save => {
                let extensions = self.request.filter.as_ref().map_or(&[][..], |f| f.1);
                let name = save_name(&self.name, extensions)?;
                let dir = inside_roots(&self.roots, dir)?;
                let target = dir.join(&name);
                if std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink() || m.is_dir()) {
                    return Err(format!("{name}: can't be replaced"));
                }
                Ok(vec![text(&target)])
            }
            PickKind::Files | PickKind::File => {
                if self.chosen.is_empty() {
                    return Err("Choose a file".into());
                }
                self.chosen.iter().map(|n| inside_roots(&self.roots, &dir.join(n)).map(|p| text(&p))).collect()
            }
        }
    }

    /// Whether the confirm button is on: [`Browser::answer`] without touching the disk (it runs
    /// every frame; the full check, with every path checked to be inside a root, runs on
    /// [`Browser::confirm`]).
    pub fn ready(&self) -> bool {
        if self.dir.is_none() {
            return false;
        }
        match self.request.kind {
            PickKind::Folder => true,
            PickKind::Save => save_name(&self.name, self.request.filter.as_ref().map_or(&[][..], |f| f.1)).is_ok(),
            PickKind::Files | PickKind::File => !self.chosen.is_empty(),
        }
    }

    /// Answer and close: the command runs again with the paths.
    pub fn confirm(&mut self) -> Result<Vec<String>, String> {
        let paths = self.answer().inspect_err(|e| self.error = Some(e.clone()))?;
        let _ = self.answer.send(paths.clone());
        Ok(paths)
    }
}

/// Open the chooser for `request` and answer through the returned channel (`None` = the host
/// names no roots). An open chooser for another command is closed unanswered.
pub fn start(app: &mut LightcraftApp, request: PickRequest) -> Option<Receiver<Vec<String>>> {
    if app.services.pick_roots.is_empty() {
        return None;
    }
    let (tx, rx) = channel();
    let name = request.file_name.clone().unwrap_or_default();
    let mut browser = Browser {
        request,
        roots: app.services.pick_roots.clone(),
        dir: None,
        entries: vec![],
        more: false,
        chosen: vec![],
        name,
        error: None,
        answer: tx,
    };
    // with one place, it opens there
    if let [(_, only)] = browser.roots.as_slice() {
        let only = only.clone();
        let _ = browser.open(&only);
    }
    app.pick_browser = Some(browser);
    Some(rx)
}

/// Close the chooser unanswered (Cancel, Back): the command it was for doesn't run.
pub fn cancel(app: &mut LightcraftApp) {
    if let Some(b) = app.pick_browser.take() {
        let _ = b.answer.send(vec![]);
    }
}

/// What the user did this frame.
enum Action {
    Places,
    Root(PathBuf),
    Up,
    Enter(String),
    Toggle(String),
    Confirm,
    Cancel,
}

/// The chooser's window, over everything else (a modal layer: nothing behind it takes clicks).
pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(b) = app.pick_browser.as_mut() else { return };
    let t = crate::theme::Tokens::get(ctx);
    let title = b.request.title.clone().unwrap_or_else(|| {
        crate::i18n::tr(match b.request.kind {
            PickKind::Files => "Choose Files",
            PickKind::File => "Choose a File",
            PickKind::Folder => "Choose a Folder",
            PickKind::Save => "Save As",
        })
        .to_string()
    });
    let screen = ctx.content_rect();
    let width = (screen.width() - 24.0).clamp(200.0, 460.0);
    let row_h = if crate::theme::density(ctx) { crate::theme::TOUCH_HIT } else { 22.0 };
    let mut action = None;
    let shown = egui::Window::new(title)
        .id(egui::Id::new("pick-browser"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(width)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal_wrapped(|ui| {
                let r = ui.add(egui::Button::new(crate::i18n::tr("Places")).min_size(vec2(0.0, row_h)));
                crate::widgets::register(ui.ctx(), "pickBrowser:places", r.rect);
                if r.clicked() {
                    action = Some(Action::Places);
                }
                if let Some(dir) = &b.dir {
                    let r = ui.add(egui::Button::new(crate::i18n::tr("Up")).min_size(vec2(0.0, row_h)));
                    crate::widgets::register(ui.ctx(), "pickBrowser:up", r.rect);
                    if r.clicked() {
                        action = Some(Action::Up);
                    }
                    ui.label(RichText::new(where_label(&b.roots, dir)).color(t.text_label));
                }
            });
            ui.separator();
            let list_h = (screen.height() * 0.5).clamp(row_h * 3.0, 360.0);
            egui::ScrollArea::vertical().id_salt("pick-browser-list").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| match &b.dir {
                None => {
                    for (label, root) in &b.roots {
                        let r = ui.add(egui::Button::new(format!("{}  ›", crate::i18n::tr(label))).min_size(vec2(ui.available_width(), row_h)));
                        crate::widgets::register(ui.ctx(), format!("pickBrowser:root:{label}"), r.rect);
                        if r.clicked() {
                            action = Some(Action::Root(root.clone()));
                        }
                    }
                }
                Some(_) => {
                    if b.entries.is_empty() {
                        ui.label(RichText::new(crate::i18n::tr("Nothing to choose here.")).color(t.text_dim));
                        ui.add(egui::Label::new(RichText::new(crate::i18n::tr(INBOX_HINT)).color(t.text_dim)).wrap());
                    }
                    for e in &b.entries {
                        let chosen = b.chosen.contains(&e.name);
                        let text = if e.dir { format!("{}  ›", e.name) } else { e.name.clone() };
                        let r = ui.add(egui::Button::selectable(chosen, text).min_size(vec2(ui.available_width(), row_h)));
                        crate::widgets::register(ui.ctx(), format!("pickBrowser:entry:{}", e.name), r.rect);
                        if r.clicked() {
                            action = Some(if e.dir { Action::Enter(e.name.clone()) } else { Action::Toggle(e.name.clone()) });
                        }
                    }
                    if b.more {
                        ui.label(
                            RichText::new(format!("{} ({MAX_ENTRIES})", crate::i18n::tr("Only the first entries are listed."))).color(t.text_dim),
                        );
                    }
                }
            });
            if b.request.kind == PickKind::Save && b.dir.is_some() {
                let r = crate::text_field::TextField::singleline("pickBrowser:name", &mut b.name).width(width).show(ui);
                if r.ending == Some(crate::text_field::Ending::Return) {
                    action = Some(Action::Confirm);
                }
            }
            if let Some(e) = &b.error {
                ui.add(egui::Label::new(RichText::new(e).color(t.caution)).wrap());
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let cancel = ui.add(egui::Button::new(crate::i18n::tr("Cancel")).min_size(vec2(0.0, row_h)));
                crate::widgets::register(ui.ctx(), "pickBrowser:cancel", cancel.rect);
                if cancel.clicked() {
                    action = Some(Action::Cancel);
                }
                let label = match b.request.kind {
                    PickKind::Folder => "Choose This Folder",
                    PickKind::Save => "Save",
                    PickKind::Files | PickKind::File => "Open",
                };
                let ok = ui.add_enabled(b.ready(), egui::Button::new(crate::i18n::tr(label)).min_size(vec2(0.0, row_h)));
                crate::widgets::register(ui.ctx(), "pickBrowser:confirm", ok.rect);
                if ok.clicked() {
                    action = Some(Action::Confirm);
                }
            });
        });
    if let Some(w) = shown {
        ctx.memory_mut(|m| m.set_modal_layer(w.response.layer_id));
        ctx.move_to_top(w.response.layer_id);
        crate::widgets::register(ctx, "pickBrowser:window", w.response.rect);
    }
    let Some(action) = action else { return };
    act(app, action);
}

/// Carry out one of the window's actions.
fn act(app: &mut LightcraftApp, action: Action) {
    let Some(b) = app.pick_browser.as_mut() else { return };
    match action {
        Action::Places => b.show_places(),
        Action::Root(root) => {
            let _ = b.open(&root);
        }
        Action::Up => b.up(),
        Action::Enter(name) => {
            if let Some(dir) = b.dir.clone() {
                let _ = b.open(&dir.join(name));
            }
        }
        Action::Toggle(name) => b.toggle(&name),
        Action::Confirm => {
            if b.confirm().is_ok() {
                app.pick_browser = None;
            }
        }
        Action::Cancel => cancel(app),
    }
}

/// "Inbox › presets": the root's label and the folders below it.
fn where_label(roots: &[(String, PathBuf)], dir: &Path) -> String {
    for (label, root) in roots {
        let Ok(real) = root.canonicalize() else { continue };
        if let Ok(rest) = dir.strip_prefix(&real) {
            let mut parts = vec![crate::i18n::tr(label).to_string()];
            parts.extend(rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()));
            return parts.join(" › ");
        }
    }
    dir.display().to_string()
}
