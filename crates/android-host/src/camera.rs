//! Cameras connected by USB (APK-59): `lightcraft_ui_egui::camera_import::CameraService` over
//! `android-sys`'s `usb` (the device list, the permission) and `mtp` (the files).
//!
//! [`UsbAccess`] keeps one [`UsbPermission`] per camera on the UI thread: an import asks for it
//! ([`UsbAccess::request`]: `hasPermission` first, then `requestPermission`), the eframe app's
//! `logic` hands it the window's focus changes and a tick ([`UsbAccess::on_event`]), so the answer
//! is read back on `Focused(true)` and every 500 ms while it is pending, and the import polls
//! [`UsbAccess::status`]. [`MtpReader`] is the `CameraReader` over an open MTP session
//! ([`MtpObjects`]: on Android `android_sys::mtp::Mtp`, in tests a fake): the object rows become the
//! review's items (`mtp::review_items`, the importer's extensions), those the library has are
//! marked (`mtp::skip_known`), and a copy is `importFile` into the staging folder.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use lightcraft_android_sys::mtp::{KnownFile as MtpKnown, ObjectRow, review_items, skip_known};
use lightcraft_android_sys::permissions::Event;
use lightcraft_android_sys::usb::{DENIED_MESSAGE, UsbAccess as State, UsbPermission};
use lightcraft_engine::devices::{CameraFile, CameraInfo, CameraReader, KnownFile};
use lightcraft_ui_egui::camera_import::{CameraAccess, CameraService};

/// The staging folder of camera copies, under `files/incoming` (outside the library).
pub const STAGING_DIR: &str = "camera";

/// The connected cameras (Android: `usb::query_cameras`, JNI on the device watch's worker). `Err`
/// is the message, for the log.
pub type CameraQuery = Arc<dyn Fn() -> Result<Vec<lightcraft_android_sys::usb::Camera>, String> + Send + Sync>;
/// `hasPermission` for a camera (Android: JNI on the UI thread).
pub type HasPermission = Box<dyn Fn(&str) -> Result<bool, String>>;
/// `requestPermission` for a camera; returns at once (Android: JNI on the UI thread).
pub type AskPermission = Box<dyn Fn(&str) -> Result<(), String>>;

/// An open camera's objects (Android: `android_sys::mtp::Mtp`).
pub trait MtpObjects {
    /// Every object's info, stopping early once `cancel` is set.
    fn objects(&mut self, cancel: &AtomicBool) -> Result<Vec<ObjectRow>, String>;
    /// Copies object `handle` to `to`.
    fn import(&mut self, handle: i32, to: &Path) -> Result<(), String>;
}

#[cfg(target_os = "android")]
impl MtpObjects for lightcraft_android_sys::mtp::Mtp<'_, '_> {
    fn objects(&mut self, cancel: &AtomicBool) -> Result<Vec<ObjectRow>, String> {
        lightcraft_android_sys::mtp::Mtp::objects(self, cancel).map_err(|e| e.user_message("read the camera"))
    }
    fn import(&mut self, handle: i32, to: &Path) -> Result<(), String> {
        lightcraft_android_sys::mtp::Mtp::import(self, handle, to).map_err(|e| e.user_message("copy from the camera"))
    }
}

/// Runs a job with the open camera named by its id, on the calling worker (Android:
/// `mtp::with_mtp`); `Err` when it can't be opened, as a message the user can act on.
pub type OpenMtp = Arc<dyn Fn(&str, &mut dyn FnMut(&mut dyn MtpObjects)) -> Result<(), String> + Send + Sync>;

/// The camera's photos as the import review lists them, from an open MTP session.
pub struct MtpReader<'a> {
    pub mtp: &'a mut dyn MtpObjects,
}

impl CameraReader for MtpReader<'_> {
    fn files(&mut self, known: &[KnownFile], cancel: &AtomicBool) -> Result<Vec<CameraFile>, String> {
        let rows = self.mtp.objects(cancel)?;
        let items = review_items(rows, |name| lightcraft_engine::import::is_supported(Path::new(name)));
        let known: Vec<MtpKnown> = known.iter().map(|k| MtpKnown { name: k.name.clone(), size: k.size, day: k.day }).collect();
        let (_, have) = skip_known(items.clone(), &known);
        let have: std::collections::HashSet<i32> = have.iter().map(|h| h.handle).collect();
        Ok(items
            .into_iter()
            .map(|o| CameraFile { id: i64::from(o.handle), in_library: have.contains(&o.handle), name: o.name, size: o.size, date: o.date })
            .collect())
    }

    fn copy(&mut self, file: &CameraFile, to: &Path) -> Result<(), String> {
        let handle = i32::try_from(file.id).map_err(|_| format!("{} has no MTP handle", file.name))?;
        self.mtp.import(handle, to)
    }
}

/// The permission per camera, on the UI thread. Clones share it; a state in use (a re-entrant
/// call) is logged and answered as pending, never a panic.
#[derive(Clone)]
pub struct UsbAccess(Rc<Inner>);

struct Inner {
    states: RefCell<HashMap<String, UsbPermission>>,
    has: HasPermission,
    ask: AskPermission,
}

fn to_ui(state: State) -> CameraAccess {
    match state {
        State::Granted => CameraAccess::Granted,
        State::Pending => CameraAccess::Pending,
        State::NotAsked | State::Denied => CameraAccess::Denied(DENIED_MESSAGE.to_owned()),
    }
}

impl UsbAccess {
    pub fn new(has: HasPermission, ask: AskPermission) -> UsbAccess {
        UsbAccess(Rc::new(Inner { states: RefCell::default(), has, ask }))
    }

    fn has(&self, id: &str) -> bool {
        (self.0.has)(id).unwrap_or_else(|e| {
            log::warn!("USB permission check for {id}: {e}");
            false
        })
    }

    /// An import asked for camera `id` at `now`: granted when `hasPermission` says so, else the
    /// system dialog is asked for (again after a refusal).
    pub fn request(&self, id: &str, now: Instant) -> CameraAccess {
        let Ok(mut states) = self.0.states.try_borrow_mut() else {
            log::warn!("USB permission request skipped: the state is in use");
            return CameraAccess::Pending;
        };
        let state = states.entry(id.to_owned()).or_default();
        // unplugged since: Android forgets a permission that wasn't "always"
        if state.access() == State::Granted && !self.has(id) {
            state.forget();
        }
        if state.request(now, || self.has(id))
            && let Err(e) = (self.0.ask)(id)
        {
            log::warn!("USB permission request for {id}: {e}");
            state.forget();
            return CameraAccess::Denied(format!("Could not ask for access to the camera: {e}"));
        }
        to_ui(state.access())
    }

    /// Where the permission for camera `id` stands (never asked: denied).
    pub fn status(&self, id: &str) -> CameraAccess {
        match self.0.states.try_borrow() {
            Ok(states) => to_ui(states.get(id).map_or(State::NotAsked, UsbPermission::access)),
            Err(_) => CameraAccess::Pending,
        }
    }

    /// One event at `now` for every pending request; answers when to tick again.
    pub fn on_event(&self, event: Event, now: Instant) -> Option<Duration> {
        let Ok(mut states) = self.0.states.try_borrow_mut() else { return Some(Duration::from_millis(500)) };
        let mut next: Option<Duration> = None;
        for (id, state) in states.iter_mut() {
            let before = state.access();
            let after = state.on_event(event, now, || self.has(id));
            if state.access() != before {
                log::info!("USB permission for {id}: {before:?} → {:?}", state.access());
            }
            next = match (next, after) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        next
    }
}

/// What the host has for cameras: the device list, the permission and the MTP job.
#[derive(Clone)]
pub struct CameraHost {
    pub query: CameraQuery,
    pub access: UsbAccess,
    pub open: OpenMtp,
}

/// `Services::cameras` over the host's calls; copies wait in `staging`.
pub fn camera_service(host: &CameraHost, staging: PathBuf) -> CameraService {
    let query = host.query.clone();
    let (request, status) = (host.access.clone(), host.access.clone());
    let open = host.open.clone();
    CameraService {
        list: Arc::new(move || match query() {
            Ok(found) => found.into_iter().map(|c| CameraInfo { id: c.device, name: c.label }).collect(),
            Err(e) => {
                log::warn!("can't list the USB cameras: {e}");
                Vec::new()
            }
        }),
        request: Box::new(move |id: &str| request.request(id, Instant::now())),
        status: Box::new(move |id: &str| status.status(id)),
        with_camera: Arc::new(move |id: &str, f: &mut dyn FnMut(&mut dyn CameraReader)| {
            open(id, &mut |mtp: &mut dyn MtpObjects| {
                let mut reader = MtpReader { mtp };
                f(&mut reader);
            })
        }),
        staging,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::Mutex;

    struct Fake {
        rows: Vec<ObjectRow>,
        imported: Arc<Mutex<Vec<(i32, PathBuf)>>>,
    }

    impl MtpObjects for Fake {
        fn objects(&mut self, _cancel: &AtomicBool) -> Result<Vec<ObjectRow>, String> {
            Ok(self.rows.clone())
        }
        fn import(&mut self, handle: i32, to: &Path) -> Result<(), String> {
            self.imported.lock().unwrap().push((handle, to.to_path_buf()));
            Ok(())
        }
    }

    fn row(handle: i32, name: &str, size: i64) -> ObjectRow {
        ObjectRow { handle, name: Some(name.into()), format: 0x3000, size, created_ms: 1_700_000_000_000, modified_ms: 0 }
    }

    /// The MTP rows reach the review as the importer's files, those the library has marked; a copy
    /// is `importFile` of the object's handle.
    #[test]
    fn mtp_rows_reach_the_review() {
        let imported = Arc::new(Mutex::new(Vec::new()));
        let mut fake = Fake {
            rows: vec![row(1, "IMG_0001.CR3", 30), row(2, "IMG_0002.CR3", 31), row(3, "CANONMSC.CTG", 5), row(4, "IMG_0004.JPG", 7)],
            imported: imported.clone(),
        };
        let mut reader = MtpReader { mtp: &mut fake };
        let known = [KnownFile { name: "img_0002.cr3".into(), size: 31, day: lightcraft_engine::devices::day_of_date("2023-11-14") }];
        let files = reader.files(&known, &AtomicBool::new(false)).unwrap();
        let got: Vec<(i64, &str, bool)> = files.iter().map(|f| (f.id, f.name.as_str(), f.in_library)).collect();
        assert_eq!(got, [(1, "IMG_0001.CR3", false), (2, "IMG_0002.CR3", true), (4, "IMG_0004.JPG", false)]);
        reader.copy(&files[0], Path::new("/tmp/x/IMG_0001.CR3")).unwrap();
        assert_eq!(*imported.lock().unwrap(), [(1, PathBuf::from("/tmp/x/IMG_0001.CR3"))]);
        let bad = CameraFile { id: i64::MAX, ..files[0].clone() };
        assert!(reader.copy(&bad, Path::new("/tmp/x/y")).is_err());
    }

    /// The service over the host's calls: cameras listed (a failed query lists none), the
    /// permission asked once and read back on the focus, and the job opened by id.
    #[test]
    fn camera_service_over_host_calls() {
        let granted = Rc::new(Cell::new(false));
        let asked = Rc::new(Cell::new(0));
        let (g, a) = (granted.clone(), asked.clone());
        let access = UsbAccess::new(
            Box::new(move |_| Ok(g.get())),
            Box::new(move |_| {
                a.set(a.get() + 1);
                Ok(())
            }),
        );
        let answer: Arc<Mutex<Result<Vec<lightcraft_android_sys::usb::Camera>, String>>> =
            Arc::new(Mutex::new(Ok(vec![lightcraft_android_sys::usb::Camera {
                device: "/dev/bus/usb/001/002".into(),
                label: "Canon EOS R5".into(),
            }])));
        let q = answer.clone();
        let opened = Arc::new(Mutex::new(Vec::new()));
        let o = opened.clone();
        let host = CameraHost {
            query: Arc::new(move || q.lock().unwrap().clone()),
            access: access.clone(),
            open: Arc::new(move |id: &str, f: &mut dyn FnMut(&mut dyn MtpObjects)| {
                o.lock().unwrap().push(id.to_owned());
                let mut fake = Fake { rows: vec![row(1, "A.JPG", 3)], imported: Arc::default() };
                f(&mut fake);
                Ok(())
            }),
        };
        let service = camera_service(&host, PathBuf::from("/data/files/incoming/camera"));
        assert_eq!((service.list)(), [CameraInfo { id: "/dev/bus/usb/001/002".into(), name: "Canon EOS R5".into() }]);
        *answer.lock().unwrap() = Err("no USB service".into());
        assert!((service.list)().is_empty());

        let id = "/dev/bus/usb/001/002";
        assert_eq!((service.status)(id), CameraAccess::Denied(DENIED_MESSAGE.into()), "never asked");
        assert_eq!((service.request)(id), CameraAccess::Pending);
        assert_eq!((service.request)(id), CameraAccess::Pending);
        assert_eq!(asked.get(), 1, "a pending request isn't made twice");
        let t = Instant::now();
        assert!(access.on_event(Event::Focused(false), t).is_some());
        granted.set(true);
        assert_eq!(access.on_event(Event::Focused(true), t + Duration::from_millis(300)), None);
        assert_eq!((service.status)(id), CameraAccess::Granted);
        // unplugged and plugged in again: asked anew
        granted.set(false);
        assert_eq!((service.request)(id), CameraAccess::Pending);
        assert_eq!(asked.get(), 2);

        let mut names = Vec::new();
        (service.with_camera)(id, &mut |reader: &mut dyn CameraReader| {
            names = reader.files(&[], &AtomicBool::new(false)).unwrap().into_iter().map(|f| f.name).collect();
        })
        .unwrap();
        assert_eq!(names, ["A.JPG"]);
        assert_eq!(*opened.lock().unwrap(), [id]);
    }
}
