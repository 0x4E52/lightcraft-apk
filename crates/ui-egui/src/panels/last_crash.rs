//! The last crash (APK-53): when the previous run recorded a panic (`engine::guard::install_hook`
//! appends each to `<settings>/panic.log`), the next start opens a small window with what happened
//! and three choices: Copy Report, Share Report (the previous run's log, through `file.share`) and
//! Dismiss. Nothing is sent anywhere: the user decides where the text goes.
//!
//! The host reads the panic log before it installs the hook again ([`take_previous`]), which
//! empties it, so a crash is offered once; it hands the text over in
//! [`crate::Services::previous_crash`].

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use egui::{Align2, RichText};
use serde_json::json;

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// The most of the panic log the report keeps (its end: the newest panics).
pub const MAX_REPORT_BYTES: usize = 8 * 1024;

/// What the previous run left behind, as the host found it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviousCrash {
    /// The panic lines of the panic log.
    pub panics: String,
    /// The log of that run (`lightcraft.1.log` once this run's log was started), the file Share
    /// Report sends. `None` = nothing to share.
    pub log: Option<String>,
}

/// Read and empty the panic log at `path`: its text (the last [`MAX_REPORT_BYTES`], from a line
/// start), or `None` when there is none, it is blank or unreadable. Removing the file is what
/// makes a crash show once; a file that can't be removed is truncated instead, and when neither
/// works the text is still returned (a window twice beats a crash report lost).
pub fn take_previous(path: &Path) -> Option<String> {
    let (bytes, cut) = read_tail(path, MAX_REPORT_BYTES as u64 + 4)?;
    if std::fs::remove_file(path).is_err() {
        let _ = std::fs::write(path, b"");
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut start = text.len().saturating_sub(MAX_REPORT_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut tail = text.get(start..).unwrap_or_default();
    if start > 0 || cut {
        // cut inside a line: begin at the next one
        tail = tail.split_once('\n').map_or(tail, |(_, rest)| rest);
    }
    let tail = tail.trim();
    (!tail.is_empty()).then(|| tail.to_owned())
}

/// The last `max` bytes of the file at `path` (all of it when shorter) and whether the start was
/// cut off; only that tail is read, so a panic log that grew without bound costs a bounded read.
pub fn read_tail(path: &Path, max: u64) -> Option<(Vec<u8>, bool)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let cut = len > max;
    if cut {
        file.seek(SeekFrom::Start(len.saturating_sub(max))).ok()?;
    }
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes).ok()?;
    Some((bytes, cut))
}

/// Where Share Report puts its copy: a file of the report in the export folder for documents
/// (`Documents/LightCraft` on Android), because the share sheet only takes files in shared storage.
const REPORT_FILE: &str = "LightCraft crash report.txt";
/// The most of the previous run's log the shared copy keeps (its end).
const MAX_LOG_BYTES: u64 = 256 * 1024;

/// The text of the shared report: the panics, then the end of the previous run's log.
fn shared_text(crash: &LastCrash) -> String {
    let mut text = crash.report();
    if let Some(log) = &crash.previous.log
        && let Some((bytes, cut)) = read_tail(Path::new(log), MAX_LOG_BYTES)
    {
        text.push_str(if cut { "\n--- the end of the previous run's log ---\n" } else { "\n--- the previous run's log ---\n" });
        text.push_str(&String::from_utf8_lossy(&bytes));
    }
    text
}

/// Write the shared copy into the export folder; its path.
fn write_shared_copy(app: &LightcraftApp, crash: &LastCrash) -> Result<String, String> {
    let dir = std::path::PathBuf::from(crate::control::export_folder(&app.services, crate::ExportKind::Document));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(REPORT_FILE);
    lightcraft_engine::catalog::safe_file::write_atomic(&path, shared_text(crash).as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// The window's state while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastCrash {
    pub previous: PreviousCrash,
}

impl LastCrash {
    /// The text Copy Report puts on the clipboard.
    pub fn report(&self) -> String {
        format!("LightCraft {} crashed on its last run.\n\n{}\n", env!("CARGO_PKG_VERSION"), self.previous.panics)
    }

    /// `ui.inspect` → `lastCrash`.
    pub fn to_json(&self) -> serde_json::Value {
        json!({"panics": self.previous.panics, "log": self.previous.log})
    }
}

/// Open the window for the crash the host found (once, at the first frame).
pub fn logic(app: &mut LightcraftApp) {
    if app.last_crash.is_none()
        && let Some(previous) = app.services.previous_crash.take()
    {
        app.last_crash = Some(LastCrash { previous });
    }
}

enum Choice {
    Copy,
    Share,
    Dismiss,
}

/// The window, until Dismiss.
pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(crash) = app.last_crash.clone() else { return };
    let t = Tokens::get(ctx);
    let can_share = app.services.share.is_some() && crash.previous.log.is_some() && app.share.is_none();
    let mut choice = None;
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::symmetric(18, 14));
    egui::Window::new(crate::i18n::tr("LightCraft closed unexpectedly"))
        .id(egui::Id::new("last-crash"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .frame(frame)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            // a fixed width: a wrapped label in an auto-sized, centred window would move it every frame
            let width = (ctx.content_rect().width() - 48.0).clamp(240.0, 440.0);
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Last time LightCraft hit an internal error. Your photos and edits are saved. Nothing has been sent anywhere: \
                         copy or share the report if you want to tell us what happened.",
                    )
                    .color(t.text_label),
                )
                .wrap(),
            );
            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&crash.previous.panics).monospace().size(11.0).color(t.text_dim)).wrap());
            });
            ui.horizontal_wrapped(|ui| {
                let mut button = |ui: &mut egui::Ui, id: &str, label: &str, enabled: bool, c: Choice| {
                    let r = ui.add_enabled(enabled, egui::Button::new(crate::i18n::tr(label)).min_size(egui::vec2(0.0, 32.0)));
                    register(ui.ctx(), format!("button:{id}"), r.rect);
                    if r.clicked() {
                        choice = Some(c);
                    }
                };
                button(ui, "lastCrashCopy", "Copy Report", true, Choice::Copy);
                button(ui, "lastCrashShare", "Share Report", can_share, Choice::Share);
                button(ui, "lastCrashDismiss", "Dismiss", true, Choice::Dismiss);
            });
        });
    match choice {
        Some(Choice::Copy) => ctx.copy_text(crash.report()),
        Some(Choice::Share) => {
            let shared =
                write_shared_copy(app, &crash).and_then(|path| app.run("file.share", json!({"paths": [path]})).map(drop).map_err(|e| e.to_string()));
            if let Err(e) = shared {
                log::warn!("share crash report: {e}");
                app.toast_error(ctx, format!("Can't share the report: {e}"));
            }
        }
        Some(Choice::Dismiss) => app.last_crash = None,
        None => {}
    }
}
