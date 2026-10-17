//! The app settings file, `<settings>/ui.json`: the saved UI state and app settings, the library
//! to open at launch included.
//!
//! The settings folder is the host's to name ([`prefs_path`]): the desktop app's
//! (`lightcraft_engine::config::config_dir`, nothing with `LIGHTCRAFT_NO_PREFS`) or the Android
//! host's internal storage. `web_time::Instant` (not `std::time::Instant`) keeps the module
//! buildable for the browser, should a web host ever use it.

use std::path::{Path, PathBuf};

use crate::LightcraftApp;
use crate::state::UiState;
use crate::window_place::{Seen, WindowPlace, WindowWatch};

/// The settings file's name inside the settings folder.
pub const FILE_NAME: &str = "ui.json";

/// `<config>/ui.json`; `None` (nothing read or written) without a settings folder.
pub fn prefs_path(config: Option<&Path>) -> Option<PathBuf> {
    config.map(|d| d.join(FILE_NAME))
}

/// The saved UI state and app settings (`<config>/ui.json`), if any, and a warning for the user
/// when the file exists but can't be used (issue #103): a damaged file is kept as
/// `ui.json.corrupt-<unix time>` first, so the next save can't lose the library location in it;
/// one that can't be read at all is not written this session (`keep_file`, the third value).
///
/// A `--memory` session reads the settings (so it looks like the user's app) but writes nothing
/// (issue #164), so a damaged file is left as it is there, not set aside.
pub fn load_prefs(config: Option<&Path>, in_memory: bool) -> (Option<UiState>, Option<String>, bool) {
    let Some(path) = prefs_path(config) else { return (None, None, false) };
    load_prefs_at(&path, !in_memory)
}

/// Compatibility recovery is limited to persisted docking, not runtime UiState commands.
pub fn decode_saved_ui(bytes: &[u8]) -> Result<(UiState, bool), String> {
    let mut saved: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let fields = saved.as_object_mut().ok_or("settings must be an object")?;
    let recovered = fields.get("docking").is_some_and(|value| match serde_json::from_value::<Option<crate::docking::Workspace>>(value.clone()) {
        Ok(Some(workspace)) => workspace.validate().is_err(),
        Ok(None) => false,
        Err(_) => true,
    });
    if recovered {
        fields.remove("docking");
    }
    let ui = serde_json::from_value(saved).map_err(|error| error.to_string())?;
    Ok((ui, recovered))
}

/// [`load_prefs`] for the file at `path`; `set_aside_damaged` is false for a session that writes
/// nothing.
pub fn load_prefs_at(path: &Path, set_aside_damaged: bool) -> (Option<UiState>, Option<String>, bool) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None, false),
        Err(e) => {
            let msg = format!("The app settings ({}) couldn't be read: {e}. Defaults are used, and the file isn't overwritten.", path.display());
            return (None, Some(msg), true);
        }
    };
    match decode_saved_ui(&bytes) {
        Ok((ui, false)) => (Some(ui.sanitized()), None, false),
        Ok((ui, true)) => {
            let msg = format!(
                "The saved panel layout in {} couldn't be restored. Other settings were loaded; the original file is preserved and settings won't be saved this session.",
                path.display()
            );
            (Some(ui.sanitized()), Some(msg), true)
        }
        Err(e) if !set_aside_damaged => {
            let msg = format!("The app settings ({}) are damaged ({e}). Defaults are used; nothing is saved in this session.", path.display());
            (None, Some(msg), true)
        }
        Err(e) => {
            let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let keep = path.with_file_name(format!("{FILE_NAME}.corrupt-{secs}"));
            let kept = std::fs::rename(path, &keep);
            let what = match &kept {
                Ok(()) => format!("it was kept as {}", keep.display()),
                Err(r) => format!("it couldn't be set aside ({r})"),
            };
            let msg = format!(
                "The app settings ({}) are damaged ({e}); {what}. Defaults are used — reopen your library with Settings → Open Library… if it isn't shown.",
                path.display()
            );
            (None, Some(msg), kept.is_err())
        }
    }
}

/// Write `bytes` to `path` atomically: a temp file, fsynced, renamed over it (a crash or a full
/// disk leaves the old file or the new one, never a truncated one).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent()
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Saves `ui.json` (app settings, incl. the library to open at launch): as soon as the library
/// changes, every few seconds when anything else changed ([`PrefsWriter::tick`]), and when the app
/// is left or quits ([`PrefsWriter::save`], through [`super::flush`]).
///
/// Nothing is written while the session is one that promises to save nothing: `--memory`
/// (`path` is `None`; issue #164), or the one running because the library couldn't be opened — the
/// question window and Continue Without Saving (issue #100; `app.openLibrary` ends it, and the
/// library it opened is saved then).
#[derive(Default)]
pub struct PrefsWriter {
    /// Where to write; `None` writes nothing (`--memory`, `LIGHTCRAFT_NO_PREFS`, no settings folder).
    path: Option<PathBuf>,
    written: Vec<u8>,
    library: String,
    checked: f64,
    /// The last write failed (reported once until a write works).
    failing: bool,
    /// `ui.json` couldn't be read at launch: never overwrite it this session.
    keep_file: bool,
    /// Follows the window, to remember the size it rests at (`UiState::window`).
    window: WindowWatch,
    /// When the window was first looked at: its watch goes by the clock from then on, not by
    /// the interface's time, which stands still while the window is hidden.
    window_since: Option<web_time::Instant>,
}

impl PrefsWriter {
    /// A writer to `path` (`None`: write nothing) that takes `app`'s settings as what is on disk
    /// now, so only changes are written; `keep_file` (from [`load_prefs`]) never overwrites the file.
    pub fn new(path: Option<PathBuf>, app: &LightcraftApp, keep_file: bool) -> PrefsWriter {
        PrefsWriter {
            path,
            written: serde_json::to_vec_pretty(&app.ui).unwrap_or_default(),
            library: app.ui.settings.library_path.clone(),
            keep_file,
            ..Default::default()
        }
    }

    /// Write `app`'s settings if they changed since the last write.
    pub fn save(&mut self, app: &LightcraftApp) -> Result<(), String> {
        self.save_ui(&app.ui, app.library_problem.is_some())
    }

    /// Write `ui` if it changed since the last write; `temporary` (the library couldn't be
    /// opened) saves nothing.
    pub fn save_ui(&mut self, ui: &UiState, temporary: bool) -> Result<(), String> {
        if self.keep_file || temporary {
            return Ok(());
        }
        let Some(path) = self.path.clone() else { return Ok(()) };
        if let Some(workspace) = &ui.docking {
            workspace.validate()?;
        }
        let bytes = serde_json::to_vec_pretty(ui).map_err(|e| e.to_string())?;
        if bytes == self.written {
            return Ok(());
        }
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| write_atomic(&path, &bytes))
            .map_err(|e| format!("saving the app settings failed: {e}"))?;
        self.written = bytes;
        self.library = ui.settings.library_path.clone();
        Ok(())
    }

    /// What to remember of the window as the app quits ([`WindowWatch::at_exit`]).
    pub fn window_at_exit(&mut self, remembered: Option<WindowPlace>) -> Option<WindowPlace> {
        match self.window_since {
            Some(since) => self.window.at_exit(remembered, since.elapsed().as_secs_f64()),
            None => remembered,
        }
    }

    /// Once per frame: follow the window and write the settings when the library changed, else
    /// at most every three seconds. A failed write is reported once (a notice) and retried.
    pub fn tick(&mut self, app: &mut LightcraftApp, ctx: &egui::Context) {
        // (asked for before the input is read: the context is not asked from inside its own lock)
        let zoom = ctx.zoom_factor();
        let (now, seen) = ctx.input(|i| (i.time, Seen::of(i.viewport(), i.content_rect(), zoom)));
        // how the window is, to open like that next time (written with the settings below). What
        // the settings hold is checked first: a script may have set it (`ui.set`)
        let remembered = app.ui.window.and_then(WindowPlace::checked);
        let at = self.window_since.get_or_insert_with(web_time::Instant::now).elapsed().as_secs_f64();
        app.ui.window = self.window.note(remembered, &seen, at);
        if let Some(wait) = self.window.waits(at) {
            // (a size is only remembered once the window has rested at it: look again then, though
            // nothing else asks for a frame)
            ctx.request_repaint_after(wait);
        }
        let moved = app.ui.settings.library_path != self.library;
        if !moved && now - self.checked < 3.0 {
            return;
        }
        self.checked = now;
        match self.save(app) {
            Ok(()) => self.failing = false,
            Err(e) => {
                if !self.failing {
                    log::warn!("{e}");
                    app.notices.push(format!("{e}. LightCraft keeps trying."));
                }
                self.failing = true;
                // don't retry every frame
                self.library = app.ui.settings.library_path.clone();
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::Services;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-app-prefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Issue #103: a damaged ui.json (which holds the library location) is kept aside and
    /// reported, not silently replaced by the defaults.
    #[test]
    fn damaged_ui_json_is_kept_and_reported() {
        let d = dir("damaged");
        let path = d.join("ui.json");
        std::fs::write(&path, br#"{"settings": {"library_path": "/Volumes/Photos/Lib"#).unwrap();
        let (ui, warning, keep) = load_prefs_at(&path, true);
        assert!(ui.is_none() && !keep);
        let warning = warning.unwrap();
        assert!(warning.contains("damaged") && warning.contains("ui.json.corrupt-"), "{warning}");
        assert!(!path.exists());
        let kept: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert!(kept.len() == 1 && kept[0].starts_with("ui.json.corrupt-"), "{kept:?}");
        // a missing file is just "no settings yet"
        assert_eq!(load_prefs_at(&path, true).1, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Issue #164: a `--memory` session reads the settings but writes nothing — a damaged file is
    /// reported and left where it is, not set aside.
    #[test]
    fn memory_session_leaves_a_damaged_ui_json_alone() {
        let d = dir("damaged-memory");
        let path = d.join("ui.json");
        let bytes = br#"{"settings": {"library_path": "/Volumes/Photos/Lib"#;
        std::fs::write(&path, bytes).unwrap();
        let (ui, warning, keep) = load_prefs_at(&path, false);
        assert!(ui.is_none() && keep);
        let warning = warning.unwrap();
        assert!(warning.contains("damaged") && warning.contains("nothing is saved"), "{warning}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        // the same through the settings folder, as a host calls it
        let (ui, _, keep) = load_prefs(Some(&d), true);
        assert!(ui.is_none() && keep);
        let (ui, warning, keep) = load_prefs(None, false);
        assert!(ui.is_none() && warning.is_none() && !keep, "no settings folder: nothing read");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn docking_survives_native_settings_file_reload() {
        use crate::docking::Panel;
        let d = dir("docking-reload");
        let path = d.join("ui.json");
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        app.run("ui.dock", serde_json::json!({"operation":"float", "panel":"tools", "rect":[70,80,310,500]})).unwrap();
        app.run("ui.dock", serde_json::json!({"operation":"close", "panel":"tools"})).unwrap();
        let mut writer = PrefsWriter { path: Some(path.clone()), ..Default::default() };
        writer.save_ui(&app.ui, false).unwrap();
        let (restored, warning, keep) = load_prefs_at(&path, false);
        assert!(warning.is_none());
        assert!(!keep);
        let mut restored_app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        restored_app.ui = restored.unwrap();
        restored_app.run("ui.dock", serde_json::json!({"operation":"open", "panel":"tools"})).unwrap();
        assert_eq!(
            restored_app.ui.docking.as_ref().unwrap().layout.floating.iter().find(|g| g.panels.contains(&Panel::Tools)).unwrap().rect,
            [70.0, 80.0, 310.0, 500.0]
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn future_docking_preserves_library_settings_and_the_original_preferences_file() {
        let d = dir("future-docking");
        let path = d.join("ui.json");
        let mut ui = UiState { thumb_size: 210.0, filmstrip: false, ..Default::default() };
        ui.settings.library_path = "/Volumes/Photos/Synthetic Library".into();
        for docking in [serde_json::json!({"layout":{"root":{"FutureLayout":{}}}}), serde_json::json!(false)] {
            let mut saved = serde_json::to_value(&ui).unwrap();
            saved["docking"] = docking;
            assert!(serde_json::from_value::<UiState>(saved.clone()).is_err(), "runtime state parsing stays strict");
            let bytes = serde_json::to_vec(&saved).unwrap();
            for set_aside in [false, true] {
                std::fs::write(&path, &bytes).unwrap();
                let (loaded, warning, keep) = load_prefs_at(&path, set_aside);
                let loaded = loaded.unwrap();
                assert_eq!(loaded.settings.library_path, ui.settings.library_path);
                assert_eq!(loaded.thumb_size, ui.thumb_size);
                assert!(!loaded.filmstrip);
                assert!(loaded.docking.is_none());
                assert!(warning.unwrap().contains("Other settings were loaded"));
                assert!(keep);
                let mut writer = PrefsWriter { path: Some(path.clone()), keep_file: keep, ..Default::default() };
                writer.save_ui(&loaded, false).unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
                assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "a future layout does not rename or replace the settings file");
            }
        }
        std::fs::remove_dir_all(d).unwrap();
    }

    /// Issue #164: the writer saves changed settings to its file — and nothing at all without one
    /// (`--memory`) or while the session is the temporary one offered when the library can't open.
    #[test]
    fn prefs_writer_writes_nothing_in_a_memory_or_temporary_session() {
        let d = dir("writer");
        let path = d.join("sub").join("ui.json");
        let mut ui = UiState::default();
        // an ordinary session: a change is written, the folder created on the way
        let mut w = PrefsWriter { path: Some(path.clone()), ..Default::default() };
        ui.thumb_size += 50.0;
        w.save_ui(&ui, false).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(String::from_utf8_lossy(&saved).contains("thumbSize"), "{}", String::from_utf8_lossy(&saved));
        // the temporary session (library problem): a further change is not written…
        ui.thumb_size += 50.0;
        w.save_ui(&ui, true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        // …until a library opens
        w.save_ui(&ui, false).unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), saved);
        // --memory: no path, so no file and no folder, whatever changes
        let d2 = dir("writer-memory");
        let _ = std::fs::remove_dir_all(&d2);
        let mut w = PrefsWriter { path: None, ..Default::default() };
        ui.thumb_size += 50.0;
        w.save_ui(&ui, false).unwrap();
        assert!(w.written.is_empty() && !d2.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A new writer takes the app's settings as written: nothing changed, nothing is written.
    #[test]
    fn a_new_writer_writes_only_changes() {
        let d = dir("writer-new");
        let path = d.join("ui.json");
        let mut app = LightcraftApp::new(lightcraft_engine::Session::new(), Services::default());
        let mut w = PrefsWriter::new(Some(path.clone()), &app, false);
        w.save(&app).unwrap();
        assert!(!path.exists(), "unchanged settings are not written");
        app.ui.thumb_size += 50.0;
        w.save(&app).unwrap();
        assert!(path.exists());
        // keep_file: the settings file that couldn't be read is never overwritten
        let kept = d.join("kept.json");
        let mut w = PrefsWriter::new(Some(kept.clone()), &app, true);
        app.ui.thumb_size += 50.0;
        w.save(&app).unwrap();
        assert!(!kept.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ui_json_is_written_atomically() {
        let d = dir("atomic");
        let path = d.join("ui.json");
        std::fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"{\"new\": true}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"new\": true}");
        assert!(!d.join("ui.json.tmp").exists());
        // a failed write leaves the old file
        let missing = d.join("gone").join("ui.json");
        assert!(write_atomic(&missing, b"x").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"new\": true}");
        let ui = UiState::default();
        write_atomic(&path, &serde_json::to_vec_pretty(&ui).unwrap()).unwrap();
        let (loaded, warning, _) = load_prefs_at(&path, true);
        assert!(loaded.is_some() && warning.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Given a window that rests at a size, when the settings are written and read again, then
    /// they hold how it was. A window in the file that is not one (a hand edit, another version's)
    /// reads as nothing remembered and costs no other setting; and a window resized just before
    /// the app quits is remembered at that size.
    #[test]
    fn the_settings_keep_how_the_window_was() {
        let d = dir("window");
        let path = d.join("ui.json");
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        app.ui.settings.library_path = "/pictures/library".into();
        let mut writer = PrefsWriter { path: Some(path.clone()), ..Default::default() };
        writer.save(&app).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("\"window\""), "nothing to remember yet");
        let window = WindowPlace { size: [1300.0, 800.0], zoomed: true };
        app.ui.window = Some(window);
        writer.save(&app).unwrap();
        let (restored, warning, _) = load_prefs_at(&path, false);
        assert_eq!((restored.unwrap().window, warning), (Some(window), None));

        // a window that is no window: the file is read all the same, with its other settings
        let text = std::fs::read_to_string(&path).unwrap();
        for odd in [r#""window": {}"#, r#""window": "wide""#, r#""window": {"size": [1, 2, 3]}"#, r#""window": {"size": [null, 800.0]}"#] {
            let start = text.find("\"window\"").unwrap();
            let end = start + text[start..].find('}').unwrap() + 1;
            std::fs::write(&path, format!("{}{odd}{}", &text[..start], &text[end..])).unwrap();
            let (restored, warning, keep) = load_prefs_at(&path, true);
            let restored = restored.unwrap_or_else(|| panic!("{odd}: {warning:?}"));
            assert_eq!((restored.window, restored.settings.library_path.as_str(), warning, keep), (None, "/pictures/library", None, false), "{odd}");
        }

        // resized, and the app quits a moment later: the new size is what is written
        let mut writer = PrefsWriter { path: Some(path.clone()), ..Default::default() };
        let rest = Some(WindowPlace { size: [1300.0, 800.0], zoomed: false });
        writer.window.note(rest, &Seen { size: Some([1500.0, 900.0]), ..Default::default() }, 0.0);
        writer.window_since = web_time::Instant::now().checked_sub(std::time::Duration::from_millis(400));
        assert!(writer.window_since.is_some());
        assert_eq!(writer.window_at_exit(rest), Some(WindowPlace { size: [1500.0, 900.0], zoomed: false }));
        assert_eq!(PrefsWriter::default().window_at_exit(rest), rest, "a window that was never seen");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Given the app's window at a size, when the settings writer looks at it frame after frame,
    /// then it asks to be woken while the size has still to rest, remembers the size once it has,
    /// and asks for nothing more. A window in the settings that is not one (a script set it) is
    /// put right before it is gone by.
    #[test]
    fn the_writer_follows_the_window_from_frame_to_frame() {
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
        let mut writer = PrefsWriter::default();
        let ctx = egui::Context::default();
        let pass = |look: &mut dyn FnMut(&egui::Context)| {
            let raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1300.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| look(ui.ctx()));
            out.textures_delta.clear();
            out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay)
        };
        // (egui's own first frames ask for another: let them pass)
        for _ in 0..3 {
            pass(&mut |_| {});
        }
        app.ui.window = Some(WindowPlace { size: [f32::NAN, 800.0], zoomed: true });
        let asked = pass(&mut |ctx| writer.tick(&mut app, ctx));
        assert_eq!(app.ui.window, None, "no window, and not yet at rest");
        assert!(asked.is_some_and(|wait| wait <= std::time::Duration::from_secs(1)), "{asked:?}");
        // a second later
        writer.window_since = writer.window_since.and_then(|since| since.checked_sub(std::time::Duration::from_secs(1)));
        pass(&mut |ctx| writer.tick(&mut app, ctx));
        assert_eq!(app.ui.window, Some(WindowPlace { size: [1300.0, 800.0], zoomed: false }));
        let asked = pass(&mut |ctx| writer.tick(&mut app, ctx));
        assert!(asked.is_none_or(|wait| wait > std::time::Duration::from_secs(3600)), "at rest and remembered: {asked:?}");
    }
}
