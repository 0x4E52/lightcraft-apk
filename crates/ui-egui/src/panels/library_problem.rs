//! The library couldn't be opened at launch (issue #100): never a silent in-memory session. A
//! blocking window says which library and why, and offers Try Again, Choose Another Library…,
//! Continue Without Saving and Quit (not where the host can't quit: Android). Continuing shows a
//! banner under the top bar for as long as the session is temporary.

use egui::{Align2, RichText, vec2};
use serde_json::json;

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// A library that failed to open (set by the host at launch).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryProblem {
    /// The library folder ("" when there was none to try, e.g. no home folder).
    pub path: String,
    /// Why it didn't open.
    pub error: String,
    /// Continue Without Saving was chosen: the banner shows instead of the window.
    pub dismissed: bool,
    /// Files from the command line, imported once a library opens.
    pub pending_import: Vec<String>,
    /// Try Again reopens `path` as a library folder (`false` in the browser, whose storage isn't
    /// a folder: reloading the page tries again).
    pub can_retry: bool,
}

impl LibraryProblem {
    pub fn new(path: impl Into<String>, error: impl Into<String>) -> LibraryProblem {
        LibraryProblem { path: path.into(), error: error.into(), can_retry: true, ..Default::default() }
    }

    /// `ui.inspect` → `libraryProblem`.
    pub fn to_json(&self) -> serde_json::Value {
        json!({"path": self.path, "error": self.error, "temporarySession": self.dismissed, "pendingImport": self.pending_import.len()})
    }
}

/// What the user chose.
enum Choice {
    Retry,
    Choose,
    Temporary,
    Quit,
}

/// Open `path` (`None`: ask for a folder) through `app.openLibrary`; on success the problem is
/// over (and command-line files are imported), else it shows the new error.
fn open(app: &mut LightcraftApp, path: Option<String>) {
    // a host that opens libraries off the UI thread (Android) does it behind its loading frame
    if let (Some(p), Some(reopen)) = (&path, app.services.reopen_library.as_mut()) {
        if let Err(e) = reopen(p)
            && let Some(problem) = app.library_problem.as_mut()
        {
            problem.error = e;
            problem.dismissed = false;
        }
        return;
    }
    let params = match &path {
        Some(p) => json!({"path": p}),
        None => json!({}),
    };
    match app.run("app.openLibrary", params) {
        Ok(serde_json::Value::Null) => {} // folder dialog cancelled
        Ok(_) => {
            let files = app.library_problem.take().map(|p| p.pending_import).unwrap_or_default();
            // in the background, like files dropped on the window (issue #374)
            if !files.is_empty()
                && let Err(e) = crate::import::start_paths(app, files)
            {
                log::warn!("import: {e}");
            }
        }
        Err(e) => {
            if let Some(p) = app.library_problem.as_mut() {
                if let Some(path) = path {
                    p.path = path;
                }
                p.error = e;
                p.dismissed = false;
            }
        }
    }
}

/// Clear the problem once a library is open (e.g. through Settings → Open Library…).
pub fn logic(app: &mut LightcraftApp) {
    if app.library_problem.is_some() && app.session.library.is_some() {
        app.library_problem = None;
    }
}

/// The blocking window (until a choice is made).
pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(problem) = app.library_problem.clone() else { return };
    if problem.dismissed {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("library-problem-dim")).order(egui::Order::Middle).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        // swallows clicks: nothing behind the window can be used meanwhile
        ui.allocate_rect(screen, egui::Sense::click_and_drag());
        ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
    });
    let mut choice = None;
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::symmetric(18, 14));
    egui::Window::new(crate::i18n::tr("Your library couldn't be opened"))
        .id(egui::Id::new("library-problem"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .frame(frame)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(460.0)
        .show(ctx, |ui| {
            // a fixed width: a wrapped label in an auto-sized, centred window would move it every frame
            ui.set_width(440.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            if problem.path.is_empty() {
                ui.label(RichText::new(crate::i18n::tr("LightCraft couldn't find where to keep your library.")).color(t.text));
            } else {
                ui.label(RichText::new(crate::i18n::tr("LightCraft couldn't open the library at")).color(t.text_label));
                ui.label(RichText::new(&problem.path).font(t.semibold(12.5)).color(t.text));
            }
            ui.add(egui::Label::new(RichText::new(&problem.error).color(t.caution)).wrap());
            ui.label(
                RichText::new(
                    "Your photos and edits in it are untouched. You can try again (e.g. after closing another program that has it open \
                     or reconnecting its drive), open a different library or create a new one, or continue without a library — \
                     then nothing you do is saved.",
                )
                .color(t.text_dim),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let mut button = |ui: &mut egui::Ui, id: &str, label: &str, enabled: bool, c: Choice| {
                    let r = ui.add_enabled(enabled, egui::Button::new(crate::i18n::tr(label)).min_size(vec2(0.0, 26.0)));
                    register(ui.ctx(), format!("button:{id}"), r.rect);
                    if r.clicked() {
                        choice = Some(c);
                    }
                };
                button(ui, "libraryRetry", "Try Again", problem.can_retry && !problem.path.is_empty(), Choice::Retry);
                button(ui, "libraryChoose", "Choose Another Library…", app.services.pick_folder.is_some(), Choice::Choose);
                button(ui, "libraryTemporary", "Continue Without Saving", true, Choice::Temporary);
                // a host that can't quit (Android) offers no Quit that would do nothing
                if !app.services.quit_disabled {
                    button(ui, "libraryQuit", "Quit", true, Choice::Quit);
                }
            });
        });
    match choice {
        Some(Choice::Retry) => open(app, Some(problem.path)),
        Some(Choice::Choose) => open(app, None),
        Some(Choice::Temporary) => {
            if let Some(p) = app.library_problem.as_mut() {
                p.dismissed = true;
            }
            let files = app.library_problem.as_mut().map(|p| std::mem::take(&mut p.pending_import)).unwrap_or_default();
            // in the background, like files dropped on the window (issue #374)
            if !files.is_empty()
                && let Err(e) = crate::import::start_paths(app, files)
            {
                log::warn!("import: {e}");
            }
        }
        Some(Choice::Quit) => app.ui.quit = true,
        None => {}
    }
}

/// The banner under the top bar while the session is temporary (Continue Without Saving).
pub fn banner(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    if !app.library_problem.as_ref().is_some_and(|p| p.dismissed) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut reopen = false;
    egui::Panel::top("library-problem-banner")
        .exact_size(30.0)
        .frame(egui::Frame::NONE.fill(t.caution).inner_margin(egui::Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let r = ui.label(
                    RichText::new(crate::i18n::tr("Temporary session — your library isn't open, and nothing you do here is saved."))
                        .font(t.semibold(12.5))
                        .color(t.canvas),
                );
                register(ui.ctx(), "indicator:temporarySession", r.rect);
                let b = ui.add(
                    egui::Button::new(RichText::new(crate::i18n::tr("Open Library…")).color(t.canvas)).fill(egui::Color32::from_black_alpha(40)),
                );
                register(ui.ctx(), "button:libraryReopen", b.rect);
                reopen = b.clicked();
            });
        });
    if reopen && let Some(p) = app.library_problem.as_mut() {
        p.dismissed = false;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::Services;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn problem_app(services: Services) -> LightcraftApp {
        let mut app = LightcraftApp::new(lightcraft_engine::Session::new(), services);
        app.library_problem = Some(LibraryProblem::new("/data/user/0/x/files/library", "replay failed"));
        app
    }

    /// Try Again goes through the host's reopen hook (a phone opens the library behind its loading
    /// frame, not on the UI thread): the hook gets the library's path, `app.openLibrary` is not
    /// run, and the window stays until the host's new session replaces this one. A hook that
    /// can't start shows why.
    #[test]
    fn try_again_uses_reopen_hook() {
        let dir = std::env::temp_dir().join(format!("lc-problem-hook-{}", std::process::id()));
        let path = dir.join("library").to_string_lossy().into_owned();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let log = calls.clone();
        let mut app = problem_app(Services {
            reopen_library: Some(Box::new(move |path| {
                log.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        });
        open(&mut app, Some(path.clone()));
        assert_eq!(*calls.borrow(), vec![path.clone()]);
        let problem = app.library_problem.as_ref().unwrap();
        assert_eq!(problem.error, "replay failed", "app.openLibrary did not run (it would have changed the error)");
        assert!(app.session.library.is_none());

        app.services.reopen_library = Some(Box::new(|_| Err("the library is already being opened".into())));
        open(&mut app, Some(path));
        assert_eq!(app.library_problem.as_ref().unwrap().error, "the library is already being opened");
        assert_eq!(calls.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without a hook (the desktop) Try Again runs `app.openLibrary` as before: here a path that
    /// is a file, so it fails with its own error.
    #[test]
    fn try_again_without_hook_opens_the_library() {
        let dir = std::env::temp_dir().join(format!("lc-problem-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not-a-folder");
        std::fs::write(&file, b"x").unwrap();
        let mut app = problem_app(Services::default());
        open(&mut app, Some(file.to_string_lossy().into_owned()));
        let problem = app.library_problem.as_ref().unwrap();
        assert_ne!(problem.error, "replay failed", "app.openLibrary ran");
        assert_eq!(problem.path, file.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
