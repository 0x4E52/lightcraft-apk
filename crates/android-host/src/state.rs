//! What the Android host holds once the library is open: the app and its settings writer.
//!
//! Built on the UI thread from the session the [`crate::loader::Loader`] opened, and kept in the
//! slot the eframe app and the lifecycle wrapper share; what the lifecycle asks of it is the
//! [`Host`] trait. The saving rules are the desktop's (`lightcraft_ui_egui::host::flush`): while
//! the app runs only what changed is written and an open slider drag stays open; when the app is
//! left everything is written, the drag committed first.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use lightcraft_ui_egui::LightcraftApp;
use lightcraft_ui_egui::host::flush;
use lightcraft_ui_egui::host::prefs::PrefsWriter;
use lightcraft_ui_egui::host::session_open::open_session;
use lightcraft_ui_egui::panels::library_problem::LibraryProblem;
use lightcraft_ui_egui::state::UiState;

use crate::lifecycle_core::Host;
use crate::loader::Opened;

/// A request to open the library again, from the library problem window's Try Again (the
/// UI thread of the app asks, the eframe wrapper that owns the loader answers on its next frame).
/// Clones share the request.
#[derive(Clone, Default)]
pub struct Reopen(Rc<RefCell<Option<String>>>);

impl Reopen {
    /// Ask for the library at `path` to be opened again (a later request replaces an earlier one that
    /// was not taken yet); `Err` when the request cell is in use.
    pub fn request(&self, path: &str) -> Result<(), String> {
        let mut slot = self.0.try_borrow_mut().map_err(|_| "the library is already being opened".to_owned())?;
        *slot = Some(path.to_owned());
        Ok(())
    }

    /// The path asked for, once (`None` when nothing was asked).
    pub fn take(&self) -> Option<String> {
        self.0.try_borrow_mut().ok().and_then(|mut slot| slot.take())
    }

    /// Whether a request waits.
    pub fn is_pending(&self) -> bool {
        self.0.try_borrow().is_ok_and(|slot| slot.is_some())
    }
}

/// What `run` prepared before the event loop, for building the app once the session is open.
#[derive(Clone, Default)]
pub struct Start {
    /// The saved UI state and settings (`ui.json`), if any.
    pub prefs: Option<UiState>,
    /// Why `ui.json` couldn't be used, for the notices panel.
    pub prefs_warning: Option<String>,
    /// The last start died creating the GPU device, or stopped after the GPU was lost
    /// (`host::gpu_markers`): GPU rendering is off and this says why (once).
    pub gpu_notice: Option<String>,
    /// The window's renderer failed to start last time, so this start uses another (once).
    pub renderer_notice: Option<String>,
    /// `ui.json` couldn't be read: never overwrite it this session.
    pub keep_prefs_file: bool,
    /// Where the settings are written (`<config>/ui.json`).
    pub prefs_path: Option<PathBuf>,
    /// The log file being written (Help ▸ Open Log Folder).
    pub log_file: Option<PathBuf>,
    /// The library folder the loader opened, for the problem window when it failed.
    pub library: PathBuf,
    /// Where Try Again in the problem window asks to open the library again (`None`: the
    /// desktop's command runs it on the UI thread).
    pub reopen: Option<Reopen>,
    /// What Back does when nothing in the app is left to close (`None` in tests: nothing).
    pub move_to_back: Option<crate::services::MoveToBack>,
    /// The private folders Settings ▸ Storage counts.
    pub storage: crate::services::StorageDirs,
    /// Access to the photo library (`None` in tests: no device to ask). The eframe app drives it;
    /// the commands that read photos ask it.
    pub photos: Option<crate::services::PhotoAccess>,
    /// The MediaStore query behind File ▸ Import from Photo Library (`None` in tests: no device);
    /// offered only with [`Start::photos`], whose permission it needs.
    pub media_query: Option<crate::services::MediaQuery>,
    /// Starts the browser on a link (`None` in tests: Help and About links are unavailable).
    pub open_link: Option<crate::services::OpenLink>,
    /// The device's shared folders (`None` in tests: no export folder, Local section from `HOME`).
    pub shared: Option<lightcraft_android_sys::storage::Folders>,
    /// File ▸ Share's service (`None` in tests: no share sheet).
    pub share: Option<lightcraft_ui_egui::share::ShareFn>,
    /// The system clipboard (`None` in tests: egui's own, in-process).
    pub clipboard: Option<crate::services::Clip>,
    /// The SD cards and USB drives (`None` in tests: the desktop's mount folders, none on a phone).
    pub volumes: Option<crate::services::VolumeQuery>,
    /// The raw types this phone hides from apps, asked on the loader's worker (default in tests:
    /// no query, nothing hidden).
    pub hidden_types: crate::services::HiddenTypes,
}

impl Start {
    /// The start for opening the library again after `old` (a session over the library problem)
    /// is gone: the settings as they are now, nothing to warn about twice. Files waiting in the
    /// problem window (`LibraryProblem::pending_import`, the desktop's command-line files) are not
    /// carried: the phone has none; once share or inbox imports reach this path they go here.
    pub fn restarted(&self, old: &HostState, library: PathBuf) -> Start {
        Start { prefs: Some(old.app.ui.clone()), prefs_warning: None, gpu_notice: None, renderer_notice: None, library, ..self.clone() }
    }

    /// Prepare for opening `library` again (Try Again): the state over the problem, if there still
    /// is one, is closed after its settings are taken over; without one only the folder changes.
    pub fn reopening(&mut self, old: Option<HostState>, library: PathBuf) {
        match old {
            Some(mut old) => {
                *self = self.restarted(&old, library);
                old.close(Duration::from_millis(500));
            }
            None => self.library = library,
        }
    }
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
        let mut services =
            crate::services::base_services(start.log_file.as_deref(), start.reopen.as_ref(), start.move_to_back.as_ref(), &start.storage);
        // no system file dialogs: the in-app chooser over the Inbox and the library
        services.pick_roots = crate::services::pick_roots(start.storage.inbox.as_deref(), &start.library);
        if let (Some(photos), Some(query)) = (&start.photos, &start.media_query) {
            services.import_media = Some(crate::services::import_media(photos.gate(), query.clone()));
        }
        services.open_url = start.open_link.clone().map(crate::services::open_url);
        if let Some(folders) = &start.shared {
            crate::services::shared_storage(&mut services, folders);
        }
        services.share = start.share.clone();
        services.clipboard = start.clipboard.clone().map(crate::services::clipboard);
        services.volumes = start.volumes.clone().map(crate::services::volumes);
        services.hidden_types = start.hidden_types.known();
        let mut app = LightcraftApp::new(session, services);
        if let Some(ui) = start.prefs {
            app.ui = ui;
        }
        lightcraft_ui_egui::i18n::set_language(app.ui.language);
        app.notices.extend(start.prefs_warning);
        app.notices.extend(start.renderer_notice);
        app.gpu_notice = true;
        if problem.is_none() {
            lightcraft_ui_egui::panels::empty_library::open_on_empty_library(&mut app);
        }
        // (after the launch-time view switch above, so it isn't saved for its own sake)
        let prefs = PrefsWriter::new(start.prefs_path, &app, start.keep_prefs_file);
        // after the snapshot above, so the writer saves the switched-off preference (as the desktop)
        if let Some(notice) = start.gpu_notice {
            app.ui.settings.gpu = false;
            app.notices.push(notice);
        }
        app.library_problem = problem;
        HostState { app, prefs }
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

impl Host for HostState {
    /// Everything to disk now, an open slider drag committed first (the app is being left: focus
    /// lost, closed). Needs no frame. Each failure is logged; the first is returned.
    fn flush_leaving(&mut self) -> Result<(), String> {
        flush::flush_leaving(&mut self.app, &mut self.prefs)
    }

    /// What changed to disk, an open slider drag left open (a timer while the app runs; the
    /// surface going away without a focus loss first).
    fn flush_dirty(&mut self) -> Result<(), String> {
        flush::flush_periodic(&mut self.app, &mut self.prefs)
    }

    /// The session's memory caches ([`lightcraft_engine::Session::trim_caches`]), the GPU's
    /// recycled buffers, and the freed pages back to the system. Memory only: no file is read or
    /// written.
    fn trim(&mut self) {
        let freed = self.app.session.trim_caches();
        lightcraft_engine::gpu::trim_pool(0);
        lightcraft_engine::memory::release();
        log::info!("trimmed: {} MiB of caches let go", freed >> 20);
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
        assert!(state.app.services.import_media.is_none(), "no device: no photo library to import from");
        assert_eq!(state.app.services.pick_roots, [("Library".to_owned(), lib.clone())], "the in-app chooser browses the library");
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

    /// The lifecycle's trim (backgrounded, low memory) lets the decoded sources go without a frame
    /// and without touching the library: the edits stay, nothing is written.
    #[test]
    fn state_trim_lets_caches_go() {
        let mut session = Session::with_demo();
        let id = session.active().unwrap();
        let image = std::sync::Arc::new(lightcraft_raster::Rgb32f::new(64, 64));
        session.media.insert(id, lightcraft_engine::media::SourceLevel::Preview, image);
        let mut state = HostState::new(Ok((session, None)), Start::default());
        assert!(state.app.session.media.held() > 0);
        let photos = state.app.session.catalog.len();
        state.trim();
        assert_eq!(state.app.session.media.held(), 0);
        assert_eq!(state.app.session.catalog.len(), photos);
    }

    /// Try Again in the problem window asks the host instead of reopening on the UI thread: the
    /// request reaches the shared cell, the library problem stays up until the host replaces the
    /// state, and the next start carries the settings as they are.
    #[test]
    fn try_again_asks_the_host_to_reopen() {
        let reopen = Reopen::default();
        let start = Start { library: PathBuf::from("/data/user/0/x/files/library"), reopen: Some(reopen.clone()), ..Start::default() };
        let mut state = HostState::new(Err("replay failed".into()), start.clone());
        assert!(!reopen.is_pending());
        let hook = state.app.services.reopen_library.as_mut().unwrap();
        hook("/data/user/0/x/files/library").unwrap();
        assert!(reopen.is_pending());
        hook("/other").unwrap();
        assert_eq!(reopen.take().as_deref(), Some("/other"), "the latest request wins");
        assert!(reopen.take().is_none() && !reopen.is_pending());
        assert!(state.app.library_problem.is_some(), "the window stays until the library is open");

        state.app.ui.thumb_size += 30.0;
        let next = start.restarted(&state, PathBuf::from("/data/user/0/x/files/library"));
        assert_eq!(next.prefs.unwrap().thumb_size, state.app.ui.thumb_size);
        assert!(next.prefs_warning.is_none() && next.reopen.is_some());

        // the swap the wrapper does: settings taken over, the old state closed, the folder asked for
        let thumb = state.app.ui.thumb_size;
        let mut again = Start { prefs_warning: Some("old".into()), ..start.clone() };
        again.reopening(Some(state), PathBuf::from("/data/user/0/x/files/other"));
        assert_eq!(again.library, PathBuf::from("/data/user/0/x/files/other"));
        assert_eq!(again.prefs.as_ref().map(|p| p.thumb_size), Some(thumb));
        assert!(again.prefs_warning.is_none() && again.reopen.is_some());
        again.reopening(None, PathBuf::from("/x"));
        assert_eq!(again.library, PathBuf::from("/x"));
    }

    /// A GPU that died at the last start: the app starts with GPU rendering off and says why, and
    /// that the window uses another renderer; both once (not again when the library reopens), and
    /// the switched-off preference is saved.
    #[test]
    fn gpu_notices_reach_the_app_once() {
        let dir = temp("gpu-notice");
        let lib = dir.join("library");
        let mut session = Session::new().with_fs();
        session.open_library(&lib, true).unwrap();
        let start = Start {
            gpu_notice: Some("LightCraft closed unexpectedly while starting the GPU last time".into()),
            renderer_notice: Some("The window could not start with its vulkan renderer last time.".into()),
            prefs_path: Some(dir.join("ui.json")),
            library: lib.clone(),
            ..Start::default()
        };
        let mut state = HostState::new(Ok((session, None)), start.clone());
        assert!(!state.app.ui.settings.gpu, "GPU rendering starts off");
        let said = |state: &HostState, what: &str| state.app.notices.iter().filter(|n| n.contains(what)).count();
        assert_eq!(said(&state, "starting the GPU last time"), 1);
        assert_eq!(said(&state, "vulkan renderer last time"), 1);
        state.prefs.save(&state.app).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("ui.json")).unwrap()).unwrap();
        assert_eq!(saved["settings"]["gpu"], json!(false), "{saved}");
        let next = start.restarted(&state, lib);
        assert!(next.gpu_notice.is_none() && next.renderer_notice.is_none());
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

    /// With a device's photo access and query, File ▸ Import from Photo Library is offered.
    #[test]
    fn photo_library_import_needs_access_and_query() {
        let t0 = std::time::Instant::now();
        let photos = crate::services::PhotoAccess::new(34, Box::new(|| Ok(Default::default())), Box::new(|_| {}), t0);
        let query: crate::services::MediaQuery = std::sync::Arc::new(|| Ok(Vec::new()));
        let only_query = Start { media_query: Some(query.clone()), ..Start::default() };
        let state = HostState::new(Err("in memory".into()), only_query);
        assert!(state.app.services.import_media.is_none(), "no access to ask: not offered");
        let start = Start { photos: Some(photos), media_query: Some(query), ..Start::default() };
        let state = HostState::new(Err("in memory".into()), start);
        assert!(state.app.services.import_media.is_some());
        assert!(lightcraft_ui_egui::menus::ui_enabled(&state.app, "file.importMediaLibrary"));
    }

    /// The host's link closure is what Help ▸ LightCraft Help calls.
    #[test]
    fn help_links_reach_the_host() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = seen.clone();
        let open_link: crate::services::OpenLink = std::rc::Rc::new(move |u: &str| {
            log.borrow_mut().push(u.to_owned());
            Ok(())
        });
        let mut state = HostState::new(Err("in memory".into()), Start { open_link: Some(open_link), ..Start::default() });
        let r = lightcraft_ui_egui::menus::run_ui_command(&mut state.app, "app.help", &serde_json::Value::Null);
        assert!(matches!(r, Some(Ok(_))), "{r:?}");
        assert_eq!(*seen.borrow(), [lightcraft_ui_egui::links::HELP]);
    }

    /// The host's clipboard is the app's: a copy from the UI reaches the host's `set`.
    #[test]
    fn clipboard_reaches_the_app() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = seen.clone();
        let clip = crate::services::Clip {
            set: std::rc::Rc::new(move |t: &str| {
                log.borrow_mut().push(t.to_owned());
                Ok(())
            }),
            get: std::rc::Rc::new(|| Ok(None)),
        };
        let mut state = HostState::new(Err("in memory".into()), Start { clipboard: Some(clip), ..Start::default() });
        let services = state.app.services.clipboard.as_mut().unwrap();
        (services.set)("Lisbon").unwrap();
        assert_eq!(*seen.borrow(), ["Lisbon"]);
        assert!(HostState::new(Err("in memory".into()), Start::default()).app.services.clipboard.is_none());
    }

    /// The device's SD cards and USB drives are the session's cameras and cards (APK-51).
    #[test]
    fn volumes_reach_the_device_scan() {
        let base = std::env::temp_dir().join(format!("lc-android-volumes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("1234-ABCD/DCIM")).unwrap();
        let root = base.join("1234-ABCD");
        let query: crate::services::VolumeQuery =
            std::sync::Arc::new(move || Ok(vec![lightcraft_android_sys::volumes::Volume { name: "SanDisk SD card".into(), root: root.clone() }]));
        let state = HostState::new(Err("in memory".into()), Start { volumes: Some(query), ..Start::default() });
        assert!(state.app.services.volumes.is_some());
        let found = state.app.session.devices.scan_now();
        assert_eq!(found.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["SanDisk SD card"]);
        assert_eq!(found[0].path, base.join("1234-ABCD/DCIM").to_string_lossy());
        assert!(HostState::new(Err("in memory".into()), Start::default()).app.services.volumes.is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The raw types the loader's worker found hidden reach the import review's and Local's note
    /// (APK-52); a start without the query names none.
    #[test]
    fn hidden_types_reach_the_services() {
        let hidden = crate::services::HiddenTypes::new(std::sync::Arc::new(|| Ok(vec!["CR3".into()])));
        let mut loader = crate::loader::Loader::start({
            let hidden = hidden.clone();
            move || {
                hidden.ask();
                (lightcraft_engine::Session::new(), None)
            }
        });
        let t0 = std::time::Instant::now();
        let opened = loop {
            if let Some(opened) = loader.poll() {
                break opened;
            }
            assert!(t0.elapsed() < Duration::from_secs(10), "the loader answers");
            std::thread::sleep(Duration::from_millis(5));
        };
        let state = HostState::new(opened, Start { hidden_types: hidden, ..Start::default() });
        assert_eq!(state.app.services.hidden_types, ["CR3"]);
        assert!(HostState::new(Err("in memory".into()), Start::default()).app.services.hidden_types.is_empty());
    }
}
