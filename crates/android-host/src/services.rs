//! The platform services the Android host gives the UI (`lightcraft_ui_egui::Services`).
//!
//! Writing files (exports, `ui.screenshot`, presets), encoding PNGs, naming the log file, the
//! first-start library notice, the memory a SAM-based mask needs, reopening the library behind the
//! loading frame, and Back and Quit: Back at the root moves the app to the background (`run` passes
//! the JNI call in), and Quit is never offered, since the system ends the Activity and the process's
//! one event loop can't start again. Commands that need a file dialog get the in-app chooser over
//! [`pick_roots`] (APK-24); sharing and the clipboard come with their cards, each as a `Services`
//! field.
//!
//! Access to the photo library ([`PhotoAccess`], APK-22) is the host's: the eframe app's `logic`
//! feeds it the window's focus changes and a tick while a request is pending, and the commands
//! that read photos ask it before they start. The MediaStore import (APK-23,
//! [`import_media`]) runs on a worker, so it asks through the [`PhotoGate`] the access keeps up
//! to date: readable → the query; denied → where to allow it; not asked yet → the request is
//! made on the UI thread's next frame and the command says to try again once allowed. Imports
//! default to Copy (`prefer_copy_import`): LightCraft can't write next to other apps' photos.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lightcraft_android_sys::permissions::{Access, Event, Grants, PhotoPermission};

use lightcraft_ui_egui::Services;
use lightcraft_ui_egui::media_import::{ImportMedia, MediaBucket};

use crate::state::Reopen;

/// Told once at the first start (in the notices panel): the library lives in the app's private
/// storage, which Android deletes with the app.
pub const LIBRARY_NOTICE: &str =
    "Your library is stored inside LightCraft. Uninstalling it or Clear data deletes the library; back it up with File ▸ Export…";

/// The thumbnail cache limit a phone starts with (the desktop's is 2 GB); the user can raise it in
/// Settings ▸ Storage.
pub const PHONE_CACHE_BYTES: u64 = 512 << 20;

/// What Settings ▸ Storage counts: the app's private folders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageDirs {
    /// `files/` and `cache/`, which App info adds up.
    pub roots: Vec<std::path::PathBuf>,
    /// Shared files waiting to be imported.
    pub inbox: Option<std::path::PathBuf>,
}

impl StorageDirs {
    /// From the internal storage folder (`…/files`): it and its sibling `cache`.
    pub fn from_internal(files: &Path, inbox: &Path) -> StorageDirs {
        let mut roots = vec![files.to_path_buf()];
        roots.extend(files.parent().map(|app| app.join("cache")));
        StorageDirs { roots, inbox: Some(inbox.to_path_buf()) }
    }
}

/// The places the in-app file chooser browses (`Services::pick_roots`): the Inbox, where shared
/// presets, track logs, keyword lists and models wait, then the library. Not `Download` or
/// `Documents`: under scoped storage the photo permission opens only other apps' *media* files,
/// so the non-media files a browser saved there are hidden from listings and can't be opened by
/// path. The Inbox folder is created here, so it can be listed before anything was shared.
pub fn pick_roots(inbox: Option<&Path>, library: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut roots = Vec::new();
    if let Some(inbox) = inbox {
        if let Err(e) = std::fs::create_dir_all(inbox) {
            log::warn!("can't create the Inbox {}: {e}", inbox.display());
        }
        roots.push(("Inbox".to_owned(), inbox.to_path_buf()));
    }
    roots.push(("Library".to_owned(), library.to_path_buf()));
    roots
}

/// Back's last step: moves the app to the background without waiting for it (Android: JNI
/// `Activity.moveTaskToBack`); `Err` when it could not even be asked.
pub type MoveToBack = Rc<dyn Fn() -> Result<(), String>>;

/// Asks `checkSelfPermission` for the photo permissions (Android: JNI on the UI thread); `Err`
/// when the call failed, for the log.
pub type CheckPhotos = Box<dyn Fn() -> Result<Grants, String>>;
/// Posts `requestPermissions` for these permissions and returns at once (Android: JNI on the Java
/// main thread, never awaited).
pub type AskPhotos = Box<dyn Fn(&'static [&'static str])>;

/// What a photo command run on a worker says when access is not settled yet: the request has
/// been made.
pub const ASKING_MESSAGE: &str = "LightCraft is asking for access to your photos: allow it, then choose Import from Photo Library again";

/// Where photo access stands, for code off the UI thread: [`PhotoAccess`] writes it after every
/// change, and a worker asks for a request through it (made on the UI thread's next frame).
#[derive(Debug)]
pub struct PhotoGate {
    access: Mutex<Access>,
    ask: AtomicBool,
}

impl Default for PhotoGate {
    fn default() -> Self {
        PhotoGate { access: Mutex::new(Access::NotAsked), ask: AtomicBool::new(false) }
    }
}

impl PhotoGate {
    /// Where access stood after the UI thread's last look.
    pub fn access(&self) -> Access {
        *self.access.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, access: Access) {
        *self.access.lock().unwrap_or_else(PoisonError::into_inner) = access;
    }

    /// Ask for access on the UI thread's next frame.
    pub fn want_request(&self) {
        self.ask.store(true, Ordering::Relaxed);
    }

    /// A request was asked for since the last call.
    pub fn take_request(&self) -> bool {
        self.ask.swap(false, Ordering::Relaxed)
    }
}

/// The MediaStore query itself (Android: `media_store::query_buckets`, JNI on the calling worker).
pub type MediaQuery = Arc<dyn Fn() -> Result<Vec<MediaBucket>, String> + Send + Sync>;

/// File ▸ Import from Photo Library's query, behind the photo permission: readable (all photos,
/// or the ones the user picked) → `query`; denied → [`DENIED_MESSAGE`](lightcraft_android_sys::permissions::DENIED_MESSAGE);
/// not asked or pending → a request on the UI thread and [`ASKING_MESSAGE`].
pub fn import_media(gate: Arc<PhotoGate>, query: MediaQuery) -> ImportMedia {
    Arc::new(move || {
        let access = gate.access();
        if access.readable() {
            return query();
        }
        if let Some(denied) = access.error() {
            return Err(denied.to_owned());
        }
        gate.want_request();
        Err(ASKING_MESSAGE.to_owned())
    })
}

/// The photo library's folders from `media_store` (paths as text, the import's form).
pub fn buckets_for_ui(buckets: Vec<lightcraft_android_sys::media_store::Bucket>) -> Vec<MediaBucket> {
    buckets.into_iter().map(|b| MediaBucket { name: b.name, path: b.dir.to_string_lossy().into_owned(), count: b.count, bytes: b.bytes }).collect()
}

/// Access to the photo library: the [`PhotoPermission`] state machine with the device's check and
/// request calls. Clones share it; it lives on the UI thread. A state that is in use (a re-entrant
/// call) is logged and the call skipped, never a panic.
#[derive(Clone)]
pub struct PhotoAccess(Rc<PhotoAccessInner>);

struct PhotoAccessInner {
    state: RefCell<PhotoPermission>,
    check: CheckPhotos,
    ask: AskPhotos,
    /// The last answer that came back, used when a check fails (so a failed call never reads as
    /// "denied").
    last: Cell<Grants>,
    /// The state as workers see it.
    gate: Arc<PhotoGate>,
}

impl PhotoAccess {
    /// Access on API level `sdk`, checked once now: the user may have allowed it in an earlier run.
    pub fn new(sdk: i32, check: CheckPhotos, ask: AskPhotos, now: Instant) -> PhotoAccess {
        let inner = PhotoAccessInner {
            state: RefCell::new(PhotoPermission::new(sdk)),
            check,
            ask,
            last: Cell::new(Grants::default()),
            gate: Arc::new(PhotoGate::default()),
        };
        let access = PhotoAccess(Rc::new(inner));
        let grants = access.grants();
        if let Ok(mut state) = access.0.state.try_borrow_mut() {
            state.refresh(now, grants);
            access.0.gate.set(state.access());
        }
        access
    }

    /// Where workers read the access, and ask for a request.
    pub fn gate(&self) -> Arc<PhotoGate> {
        self.0.gate.clone()
    }

    /// Where access stands now.
    pub fn access(&self) -> Access {
        self.0.state.try_borrow().map_or(Access::Pending, |state| state.access())
    }

    /// Asks for access unless every photo is readable already or a request is pending; answers
    /// where access stands then (`Pending` once asked). The answer arrives through
    /// [`Self::on_event`].
    pub fn request(&self, now: Instant) -> Access {
        let Ok(mut state) = self.0.state.try_borrow_mut() else {
            log::warn!("photo permission request skipped: the state is in use");
            return Access::Pending;
        };
        if let Some(permissions) = state.request(now) {
            log::info!("asking for {permissions:?}");
            (self.0.ask)(permissions);
        }
        self.0.gate.set(state.access());
        state.access()
    }

    /// One event at `now` (a focus change, or the frame's tick); answers when to tick again while
    /// a request is pending.
    pub fn on_event(&self, event: Event, now: Instant) -> Option<Duration> {
        if self.0.gate.take_request() {
            self.request(now);
        }
        let Ok(mut state) = self.0.state.try_borrow_mut() else {
            log::warn!("photo permission event skipped: the state is in use");
            return None;
        };
        let before = state.access();
        let after = state.on_event(event, now, || self.grants());
        if state.access() != before {
            log::info!("photo access: {:?} → {:?}", before, state.access());
        }
        self.0.gate.set(state.access());
        after
    }

    /// What the device answers now, or the last answer when asking failed.
    fn grants(&self) -> Grants {
        match (self.0.check)() {
            Ok(grants) => {
                self.0.last.set(grants);
                grants
            }
            Err(e) => {
                log::warn!("photo permission check failed: {e}");
                self.0.last.get()
            }
        }
    }
}

/// The services every Android start has; `log_file` is the log being written, if any, `reopen`
/// where Try Again in the library problem window asks the host to open the library again, and
/// `move_to_back` what Back does when nothing in the app is left to close.
pub fn base_services(log_file: Option<&Path>, reopen: Option<&Reopen>, move_to_back: Option<&MoveToBack>, storage: &StorageDirs) -> Services {
    Services {
        write: Some(Box::new(lightcraft_engine::export::write_file)),
        // with it, exports started from the UI run in the background
        write_shared: Some(Arc::new(lightcraft_engine::export::write_file)),
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        log_file: log_file.map(|p| p.display().to_string()),
        library_notice: Some(LIBRARY_NOTICE.to_owned()),
        min_ram_for_sam: Some(crate::memory::SAM_MIN_RAM),
        reopen_library: reopen.cloned().map(|reopen| Box::new(move |path: &str| reopen.request(path)) as lightcraft_ui_egui::ReopenLibrary),
        move_to_back: move_to_back
            .cloned()
            .map(|f| Box::new(move |_: &mut lightcraft_engine::Session| f().map(|()| Default::default())) as lightcraft_ui_egui::HostAction),
        storage_roots: storage.roots.clone(),
        storage_inbox: storage.inbox.clone(),
        // under 1 GB free, the caches are trimmed to half their limits
        free_space: Some(Box::new(lightcraft_engine::storage_report::free_bytes)),
        quit_disabled: true,
        touch: true,
        // photos added in place from shared storage are other apps' files: read-only to LightCraft
        prefer_copy_import: true,
        ..Services::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn services_have_writer_and_png() {
        let mut s =
            base_services(Some(Path::new("/data/user/0/ai.storyteller.lightcraft/files/logs/lightcraft.log")), None, None, &StorageDirs::default());
        assert_eq!(s.log_file.as_deref(), Some("/data/user/0/ai.storyteller.lightcraft/files/logs/lightcraft.log"));
        assert!(base_services(None, None, None, &StorageDirs::default()).log_file.is_none());

        let dir = std::env::temp_dir().join(format!("lc-android-services-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("export.jpg");
        let target = target.to_string_lossy().to_string();
        (s.write.as_mut().unwrap())(&target, b"one").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"one");
        (s.write_shared.as_ref().unwrap())(&target, b"two").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");

        let img = lightcraft_raster::Rgba8::from_bytes(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let png = (s.png.as_ref().unwrap())(&img);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "a PNG file");
        // the phone's limits are in place; without a host to reopen for, Try Again is the command
        assert_eq!(s.library_notice.as_deref(), Some(LIBRARY_NOTICE));
        assert_eq!(s.min_ram_for_sam, Some(crate::memory::SAM_MIN_RAM));
        assert!(s.reopen_library.is_none());
        // nothing a desktop-only picker would need
        assert!(s.picker.is_none() && s.pick_files.is_none() && s.reveal.is_none());
        // Quit is never offered; without a way to the background, Back at the root does nothing
        assert!(s.quit_disabled && s.move_to_back.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Back at the root reaches the host's call, and its refusal comes back as the command's error.
    #[test]
    fn back_at_the_root_calls_the_host() {
        let calls = Rc::new(std::cell::Cell::new(0));
        let seen = calls.clone();
        let move_to_back: MoveToBack = Rc::new(move || {
            seen.set(seen.get() + 1);
            if seen.get() == 1 { Ok(()) } else { Err("no Activity".into()) }
        });
        let mut s = base_services(None, None, Some(&move_to_back), &StorageDirs::default());
        let mut session = lightcraft_engine::Session::new();
        let f = s.move_to_back.as_mut().unwrap();
        assert_eq!(f(&mut session), Ok(serde_json::Value::Null));
        assert_eq!(f(&mut session), Err("no Activity".to_owned()));
        assert_eq!(calls.get(), 2);
        assert!(s.quit_disabled);
    }

    /// The host's photo access: checked at the start, asked once per request, read back from
    /// the focus and the ticks while pending; a failed check keeps the last answer.
    #[test]
    fn photo_access_reads_the_answer_back() {
        use lightcraft_android_sys::permissions::{DENIED_MESSAGE, READ_MEDIA_IMAGES};
        let t0 = Instant::now();
        let device = Rc::new(RefCell::new(Ok::<Grants, String>(Grants::default())));
        let asked = Rc::new(RefCell::new(Vec::<Vec<&str>>::new()));
        let (answer, log) = (device.clone(), asked.clone());
        let photos =
            PhotoAccess::new(34, Box::new(move || answer.borrow().clone()), Box::new(move |perms| log.borrow_mut().push(perms.to_vec())), t0);
        assert_eq!(photos.access(), Access::NotAsked);
        assert_eq!(photos.request(t0), Access::Pending);
        assert_eq!(photos.request(t0), Access::Pending, "a pending request is not asked twice");
        assert_eq!(asked.borrow().len(), 1);
        assert!(asked.borrow()[0].contains(&READ_MEDIA_IMAGES));
        // the frame ticks while pending, then the dialog takes the focus
        assert!(photos.on_event(Event::Tick, t0).is_some());
        photos.on_event(Event::Focused(false), t0 + Duration::from_millis(100));
        // a check that fails is no answer: still pending, never denied by it
        *device.borrow_mut() = Err("no JVM".into());
        assert!(photos.on_event(Event::Tick, t0 + Duration::from_millis(600)).is_some());
        assert_eq!(photos.access(), Access::Pending);
        // the user allows every photo; the focus comes back
        *device.borrow_mut() = Ok(Grants { all: true, selected: false, location: true });
        assert!(photos.on_event(Event::Focused(true), t0 + Duration::from_millis(900)).is_none());
        assert_eq!(photos.access(), Access::Granted);
        assert_eq!(photos.request(t0 + Duration::from_secs(2)), Access::Granted);
        assert_eq!(asked.borrow().len(), 1, "granted: nothing more is asked");

        // allowed in an earlier run: granted from the start; denied: the command's error says where
        let granted = PhotoAccess::new(33, Box::new(|| Ok(Grants { all: true, ..Grants::default() })), Box::new(|_| {}), t0);
        assert_eq!(granted.access(), Access::Granted);
        let denied = PhotoAccess::new(33, Box::new(|| Ok(Grants::default())), Box::new(|_| {}), t0);
        denied.request(t0);
        denied.on_event(Event::Tick, t0 + Duration::from_secs(1));
        assert_eq!(denied.access().error(), Some(DENIED_MESSAGE));
    }

    /// File ▸ Import from Photo Library asks for access first (on the UI thread, from the worker's
    /// call), queries only once the photos are readable, and says where to allow it when denied;
    /// every import defaults to Copy.
    #[test]
    fn import_media_asks_for_access_first() {
        use lightcraft_android_sys::permissions::DENIED_MESSAGE;
        let t0 = Instant::now();
        let device = Rc::new(Cell::new(Grants::default()));
        let asked = Rc::new(Cell::new(0));
        let (answer, count) = (device.clone(), asked.clone());
        let photos = PhotoAccess::new(34, Box::new(move || Ok(answer.get())), Box::new(move |_| count.set(count.get() + 1)), t0);
        let queried = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = queried.clone();
        let query: MediaQuery = Arc::new(move || {
            seen.fetch_add(1, Ordering::Relaxed);
            Ok(vec![MediaBucket { name: "Camera".into(), path: "/storage/emulated/0/DCIM/Camera".into(), count: 3, bytes: 9 }])
        });
        let import = import_media(photos.gate(), query);
        // not asked yet: no query, a request on the next frame
        let worker = import.clone();
        let first = std::thread::spawn(move || worker()).join().unwrap();
        assert_eq!(first, Err(ASKING_MESSAGE.to_owned()));
        assert_eq!(queried.load(Ordering::Relaxed), 0);
        assert_eq!(asked.get(), 0, "the worker never asks the device itself");
        photos.on_event(Event::Tick, t0);
        assert_eq!(asked.get(), 1, "asked on the UI thread");
        assert_eq!(photos.gate().access(), Access::Pending);
        // the user allows the picked photos: the query runs
        photos.on_event(Event::Focused(false), t0 + Duration::from_millis(100));
        device.set(Grants { all: false, selected: true, location: true });
        photos.on_event(Event::Focused(true), t0 + Duration::from_millis(900));
        assert_eq!(photos.gate().access(), Access::Partial);
        let buckets = import().unwrap();
        assert_eq!(buckets[0].name, "Camera");
        assert_eq!(queried.load(Ordering::Relaxed), 1);
        photos.on_event(Event::Tick, t0 + Duration::from_secs(2));
        assert_eq!(asked.get(), 1, "readable: nothing more is asked");

        // denied: where to allow it, and still no query
        let denied = PhotoAccess::new(33, Box::new(|| Ok(Grants::default())), Box::new(|_| {}), t0);
        denied.request(t0);
        denied.on_event(Event::Tick, t0 + Duration::from_secs(1));
        let none: MediaQuery = Arc::new(|| panic!("no query without access"));
        assert_eq!(import_media(denied.gate(), none)(), Err(DENIED_MESSAGE.to_owned()));
        // the folders as the import takes them; the review starts on Copy
        let ui = buckets_for_ui(vec![lightcraft_android_sys::media_store::Bucket {
            name: "Camera".into(),
            dir: "/storage/emulated/0/DCIM/Camera".into(),
            count: 2,
            bytes: 5,
        }]);
        assert_eq!(ui, [MediaBucket { name: "Camera".into(), path: "/storage/emulated/0/DCIM/Camera".into(), count: 2, bytes: 5 }]);
        let s = base_services(None, None, None, &StorageDirs::default());
        assert!(s.prefer_copy_import);
        assert!(s.import_media.is_none(), "offered only with a device to query");
    }

    /// The in-app file chooser browses the Inbox (created if missing) and the library, never
    /// Download or Documents.
    #[test]
    fn pick_roots_are_inbox_and_library() {
        let dir = std::env::temp_dir().join(format!("lc-android-pick-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (inbox, library) = (dir.join("files/inbox"), dir.join("files/library"));
        let roots = pick_roots(Some(&inbox), &library);
        assert_eq!(roots, [("Inbox".to_owned(), inbox.clone()), ("Library".to_owned(), library.clone())]);
        assert!(inbox.is_dir(), "the Inbox can be listed before anything was shared");
        assert_eq!(pick_roots(None, &library), [("Library".to_owned(), library)]);
        assert!(roots.iter().all(|(_, p)| !p.ends_with("Download") && !p.ends_with("Documents")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Settings ▸ Storage counts `files/` and the sibling `cache/`, so its total is App info's.
    #[test]
    fn storage_counts_files_and_cache() {
        let dirs = StorageDirs::from_internal(Path::new("/data/user/0/x/files"), Path::new("/data/user/0/x/files/inbox"));
        assert_eq!(dirs.roots, [Path::new("/data/user/0/x/files"), Path::new("/data/user/0/x/cache")]);
        let s = base_services(None, None, None, &dirs);
        assert_eq!(s.storage_roots, dirs.roots);
        assert_eq!(s.storage_inbox.as_deref(), Some(Path::new("/data/user/0/x/files/inbox")));
        assert_eq!(PHONE_CACHE_BYTES, 512 << 20);
        // the low-space rule has a probe, and it answers for a real folder
        let free = s.free_space.as_ref().unwrap();
        assert!(free(&std::env::temp_dir()).is_some());
    }
}
