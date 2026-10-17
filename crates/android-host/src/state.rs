//! What the Android host holds once the library is open: the app and its settings writer.
//!
//! Built on the UI thread from the session the [`crate::loader::Loader`] opened, and kept in the
//! slot the eframe app (and, from APK-18, the lifecycle wrapper) share. The saving rules are the
//! desktop's (`lightcraft_ui_egui::host::flush`): while the app runs only what changed is written
//! and an open slider drag stays open; when the app is left everything is written, the drag
//! committed first.

use std::path::PathBuf;
use std::time::Duration;

use lightcraft_ui_egui::LightcraftApp;
use lightcraft_ui_egui::host::flush;
use lightcraft_ui_egui::host::prefs::PrefsWriter;
use lightcraft_ui_egui::host::session_open::open_session;
use lightcraft_ui_egui::panels::library_problem::LibraryProblem;
use lightcraft_ui_egui::state::UiState;

use crate::loader::Opened;

/// What `run` prepared before the event loop, for building the app once the session is open.
#[derive(Default)]
pub struct Start {
    /// The saved UI state and settings (`ui.json`), if any.
    pub prefs: Option<UiState>,
    /// Why `ui.json` couldn't be used, for the notices panel.
    pub prefs_warning: Option<String>,
    /// `ui.json` couldn't be read: never overwrite it this session.
    pub keep_prefs_file: bool,
    /// Where the settings are written (`<config>/ui.json`).
    pub prefs_path: Option<PathBuf>,
    /// The log file being written (Help ▸ Open Log Folder).
    pub log_file: Option<PathBuf>,
    /// The library folder the loader opened, for the problem window when it failed.
    pub library: PathBuf,
}

/// The running app and the writer of its settings file.
pub struct HostState {
    pub app: LightcraftApp,
    pub prefs: PrefsWriter,
}

impl HostState {
    /// The app around the session the loader opened (or, when opening failed, an empty session in
    /// memory and the problem window asking what to do), with the saved settings applied.
    pub fn new(opened: Result<Opened, String>, start: Start) -> HostState {
        let (session, problem) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                log::error!("can't open library {}: {e}", start.library.display());
                // as the desktop's problem path: the folders and the SAM model of `open_session`
                let (session, _) = open_session(true, None, false);
                (session.with_default_sam3_model(), Some(LibraryProblem::new(start.library.to_string_lossy(), e)))
            }
        };
        let mut app = LightcraftApp::new(session, crate::services::base_services(start.log_file.as_deref()));
        if let Some(ui) = start.prefs {
            app.ui = ui;
        }
        lightcraft_ui_egui::i18n::set_language(app.ui.language);
        app.notices.extend(start.prefs_warning);
        app.gpu_notice = true;
        if problem.is_none() {
            lightcraft_ui_egui::panels::empty_library::open_on_empty_library(&mut app);
        }
        // (after the launch-time view switch above, so it isn't saved for its own sake)
        let prefs = PrefsWriter::new(start.prefs_path, &app, start.keep_prefs_file);
        app.library_problem = problem;
        HostState { app, prefs }
    }

    /// Everything to disk now, an open slider drag committed first (the app is being left: focus
    /// lost, closed). Needs no frame. Each failure is logged; the first is returned.
    pub fn flush_leaving(&mut self) -> Result<(), String> {
        flush::flush_leaving(&mut self.app, &mut self.prefs)
    }

    /// What changed to disk, an open slider drag left open (a timer while the app runs).
    pub fn flush_dirty(&mut self) -> Result<(), String> {
        flush::flush_periodic(&mut self.app, &mut self.prefs)
    }

    /// The app ends: everything saved, the library closed, then up to `wait` for renders in
    /// flight, so none is inside the GPU driver when the process exits (as the desktop's
    /// `on_exit`).
    pub fn close(&mut self, wait: Duration) {
        let _ = self.flush_leaving();
        if let Err(e) = self.app.session.close_library() {
            log::error!("saving the library failed: {e}");
        }
        if !self.app.shutdown(wait) {
            log::warn!("quit: background work was still running after {wait:?}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lightcraft_engine::Session;
    use serde_json::json;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-android-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn exposure(s: &Session, id: lightcraft_engine::catalog::PhotoId) -> Option<f64> {
        s.develop_of(id).and_then(|d| lightcraft_engine::develop::controls::get(&d, "light.exposure"))
    }

    /// The host state built from an opened library saves a slider drag when the app is left, with
    /// no frame in between (Android's focus loss): reopened, the edit, the view and the settings
    /// are there. A periodic save before it leaves the drag open.
    #[test]
    fn state_flush_without_a_frame() {
        let dir = temp("flush");
        let lib = dir.join("library");
        let mut session = Session::new().with_fs();
        session.open_library(&lib, true).unwrap();
        let start = Start { prefs_path: Some(dir.join("config").join("ui.json")), library: lib.clone(), ..Start::default() };
        let mut state = HostState::new(Ok((session, None)), start);
        assert!(state.app.library_problem.is_none());
        assert!(state.app.services.write.is_some(), "the Android services are in place");
        let id = *state.app.session.visible().get(1).unwrap();
        state.app.run("library.select", json!({"ids": [id.0]})).unwrap();
        state.app.run("develop.beginInteraction", json!({"label": "Exposure"})).unwrap();
        state.app.run("develop.set", json!({"control": "light.exposure", "value": 1.5})).unwrap();
        state.app.ui.thumb_size += 40.0;

        state.flush_dirty().unwrap();
        assert!(state.app.session.interaction.is_some(), "a periodic save leaves the drag open");

        state.flush_leaving().unwrap();
        assert!(state.app.session.interaction.is_none(), "leaving commits the drag");
        assert!(dir.join("config").join("ui.json").exists());
        assert!(lib.join("view.json").exists());
        // the process is killed here: what was flushed is what the next start sees
        drop(state);
        let mut reopened = Session::new().with_fs();
        reopened.open_library(&lib, false).unwrap();
        assert_eq!(exposure(&reopened, id), Some(1.5));
        assert_eq!(reopened.selection.active, Some(id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A library the loader couldn't open is never replaced: the session is empty and in memory
    /// and the problem window says why, with the library's folder.
    #[test]
    fn a_failed_open_shows_the_library_problem() {
        let start = Start { library: PathBuf::from("/data/user/0/x/files/library"), ..Start::default() };
        let state = HostState::new(Err(crate::loader::STOPPED.to_owned()), start);
        let problem = state.app.library_problem.as_ref().unwrap();
        assert_eq!(problem.path, "/data/user/0/x/files/library");
        assert_eq!(problem.error, crate::loader::STOPPED);
        assert!(state.app.session.library.is_none() && state.app.session.catalog.is_empty());
    }
}
