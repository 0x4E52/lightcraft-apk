//! Import from a camera connected by USB (APK-59) with a fake host: the camera is a device, the
//! permission is waited for, the camera is read and copied on workers under activity rows, and a
//! camera unplugged mid-import gives one error per photo it didn't send.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lightcraft_engine::devices::{CameraFile, CameraInfo, CameraReader, DeviceKind, KnownFile};
use serde_json::json;

use crate::camera_import::{CameraAccess, CameraService, LISTING, copying_label};
use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(60);
const ID: &str = "/dev/bus/usb/001/002";
const NAME: &str = "Canon EOS R5";

/// A fake camera: its photos (PNG bytes), a gate the listing and each copy wait on, and how many
/// copies it sends before it is "unplugged".
#[derive(Clone)]
struct FakeCamera {
    photos: Vec<(CameraFile, Vec<u8>)>,
    gate: Arc<AtomicBool>,
    unplug_after: Option<usize>,
    copies: Arc<AtomicUsize>,
}

impl CameraReader for FakeCamera {
    fn files(&mut self, known: &[KnownFile], _cancel: &AtomicBool) -> Result<Vec<CameraFile>, String> {
        wait(&self.gate);
        Ok(self
            .photos
            .iter()
            .map(|(f, _)| CameraFile { in_library: known.iter().any(|k| k.name == f.name.to_lowercase() && k.size == f.size), ..f.clone() })
            .collect())
    }

    fn copy(&mut self, file: &CameraFile, to: &Path) -> Result<(), String> {
        wait(&self.gate);
        let n = self.copies.fetch_add(1, Ordering::SeqCst);
        if self.unplug_after.is_some_and(|max| n >= max) {
            return Err("the camera did not send the file (unplugged, or asleep?)".into());
        }
        let bytes = self.photos.iter().find(|(f, _)| f.id == file.id).map(|(_, b)| b.clone()).ok_or("no such object")?;
        std::fs::write(to, bytes).map_err(|e| e.to_string())
    }
}

fn wait(gate: &AtomicBool) {
    let t = std::time::Instant::now();
    while !gate.load(Ordering::SeqCst) && t.elapsed() < T {
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `n` distinct small PNGs as the camera's photos.
fn photos(n: usize) -> Vec<(CameraFile, Vec<u8>)> {
    (0..n)
        .map(|i| {
            let img = lightcraft_raster::Rgba8::from_fn(16, 12, |x, y| [(x * 9 + i * 40) as u8, (y * 12) as u8, 80, 255]);
            let o = lightcraft_engine::export::ExportOptions { format: lightcraft_engine::export::ExportFormat::Png, ..Default::default() };
            let png = lightcraft_engine::export::encode_image(&img, &o).unwrap();
            let f = CameraFile {
                id: i as i64 + 1,
                name: format!("IMG_{:04}.png", i + 1),
                size: png.len() as u64,
                date: Some(1_700_000_000),
                in_library: false,
            };
            (f, png)
        })
        .collect()
}

struct Fixture {
    h: Headless,
    dir: PathBuf,
    staging: PathBuf,
    granted: Arc<AtomicBool>,
    asked: Arc<Mutex<Vec<String>>>,
}

/// An app on a library on disk with a camera service over `camera`; the permission is pending until
/// `granted` is set.
fn fixture(tag: &str, camera: FakeCamera) -> Fixture {
    let dir = std::env::temp_dir().join(format!("lc-camera-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let staging = dir.join("incoming").join("camera");
    let (session, _problem) = crate::host::session_open::open_session(false, Some(dir.join("library")), false);
    let granted = Arc::new(AtomicBool::new(false));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (g1, g2, a) = (granted.clone(), granted.clone(), asked.clone());
    let access = move |g: &AtomicBool| if g.load(Ordering::SeqCst) { CameraAccess::Granted } else { CameraAccess::Pending };
    let service = CameraService {
        list: Arc::new(|| vec![CameraInfo { id: ID.into(), name: NAME.into() }]),
        request: Box::new(move |id: &str| {
            a.lock().unwrap().push(id.to_owned());
            access(&g1)
        }),
        status: Box::new(move |_: &str| access(&g2)),
        with_camera: Arc::new(move |id: &str, f: &mut dyn FnMut(&mut dyn CameraReader)| {
            if id != ID {
                return Err("The camera is no longer connected.".into());
            }
            let mut reader = camera.clone();
            f(&mut reader);
            Ok(())
        }),
        staging: staging.clone(),
    };
    let mut app = LightcraftApp::new(session, Services { png: None, cameras: Some(service), ..Default::default() });
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.settle(Duration::from_secs(30));
    Fixture { h, dir, staging, granted, asked }
}

fn row(h: &Headless, label: &str) -> bool {
    h.app.session.activity.list().iter().any(|t| t.label == label)
}

fn notices(h: &Headless) -> String {
    h.app.notices.join("\n")
}

/// The camera is listed with the cards; choosing it waits for the permission, reads the camera
/// under "Reading the camera" into the review (a photo the library has is marked and unchecked),
/// and Import copies the chosen photos under "Copying from …", then imports them (Move: the staging
/// folder is empty again).
#[test]
fn camera_device_import_shows_activity_row() {
    let gate = Arc::new(AtomicBool::new(false));
    let camera = FakeCamera { photos: photos(3), gate: gate.clone(), unplug_after: None, copies: Arc::default() };
    let Fixture { mut h, dir, staging, granted, asked } = fixture("import", camera);
    // a device of its own kind, next to the cards; not a place to browse
    let devices = h.app.session.devices.scan_now();
    let cam = devices.iter().find(|d| d.kind == DeviceKind::Camera).expect("the camera is a device");
    assert_eq!((cam.name.as_str(), cam.path.as_str()), (NAME, "usb:/dev/bus/usb/001/002"));
    // the permission is asked for, and the import waits for it
    let r = h.request("engine.execute", json!({"command": "file.addFromDevice", "params": {"path": cam.path}}), T);
    assert_eq!(r["result"]["waiting"], true, "{r}");
    assert_eq!(*asked.lock().unwrap(), [ID]);
    for _ in 0..3 {
        h.step();
    }
    assert!(!row(&h, LISTING), "nothing is read before the permission");
    let inspect = h.request("ui.inspect", json!({}), T);
    assert_eq!(inspect["result"]["camera"]["state"], "permission", "the wait shows over the control port: {inspect}");
    assert!(h.app.run("file.addFromDevice", json!({"path": cam.path})).is_err(), "one camera import at a time");
    granted.store(true, Ordering::SeqCst);
    assert!(h.step_until(T, |h| row(h, LISTING)), "read under its row once allowed");
    assert_eq!(h.request("ui.inspect", json!({}), T)["result"]["camera"]["state"], "listing");
    for _ in 0..3 {
        h.step();
        assert!(row(&h, LISTING), "the row stays while the camera answers");
    }
    gate.store(true, Ordering::SeqCst);
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    assert!(!row(&h, LISTING), "the row is gone with the answer");
    let Some(Dialog::Import { opts }) = &mut h.app.ui.dialog else { panic!("no import review") };
    assert_eq!(opts.candidates.len(), 3);
    assert_eq!(opts.sources, [NAME]);
    assert!(opts.copy, "copied off the camera");
    assert_eq!(opts.candidates[0].captured.as_deref(), Some("2023-11-14T22:13:20"));
    assert!(std::fs::read_dir(&staging).map_or(true, |mut d| d.next().is_none()), "nothing copied before Import");
    // two of three photos
    opts.checked[1] = false;
    // the copies wait while the camera answers: "Copying from …" shows
    gate.store(false, Ordering::SeqCst);
    let before = h.app.session.catalog.len();
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(T, |h| row(h, &copying_label(NAME))));
    for _ in 0..3 {
        h.step();
        assert!(row(&h, &copying_label(NAME)));
    }
    gate.store(true, Ordering::SeqCst);
    assert!(h.step_until(T, |h| h.app.camera.is_none() && h.app.import.is_none() && h.app.session.catalog.len() == before + 2), "{}", notices(&h));
    assert!(!row(&h, &copying_label(NAME)));
    assert!(h.request("ui.inspect", json!({}), T)["result"]["camera"].is_null(), "no camera import left");
    let names: Vec<String> = h.app.session.catalog.photos().map(|p| p.file_name.clone()).collect();
    assert!(names.contains(&"IMG_0001.png".to_owned()) && names.contains(&"IMG_0003.png".to_owned()), "{names:?}");
    assert!(!names.contains(&"IMG_0002.png".to_owned()));
    // moved into the library: the staging folder keeps nothing
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0, "the copies were moved into the library");
    assert!(h.app.notices.is_empty(), "{}", notices(&h));
    // read again: the library's photos are marked and not checked
    h.app.ui.dialog = None;
    let r = h.request("engine.execute", json!({"command": "file.addFromDevice", "params": {"path": "usb:/dev/bus/usb/001/002"}}), T);
    assert_eq!(r["result"]["reading"], true, "allowed already: read at once ({r})");
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    let marked: Vec<bool> = opts.candidates.iter().map(|c| c.duplicate.as_deref() == Some("path")).collect();
    assert_eq!(marked, [true, false, true]);
    assert_eq!(opts.selected(), [1], "only the new photo is checked");
    // a camera that is gone
    assert!(h.app.run("file.addFromDevice", json!({"path": "usb:/dev/bus/usb/009/009"})).unwrap_err().contains("no longer connected"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Unplugged after the first photo: that one is imported, each other one is an error in the
/// notices, and no partial copy stays behind.
#[test]
fn camera_unplugged_mid_import_gives_errors() {
    let gate = Arc::new(AtomicBool::new(true));
    let camera = FakeCamera { photos: photos(3), gate, unplug_after: Some(1), copies: Arc::default() };
    let Fixture { mut h, dir, staging, granted, .. } = fixture("unplug", camera);
    granted.store(true, Ordering::SeqCst);
    let r = h.request("engine.execute", json!({"command": "file.addFromDevice", "params": {"path": "usb:/dev/bus/usb/001/002"}}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    let before = h.app.session.catalog.len();
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(T, |h| h.app.camera.is_none() && h.app.import.is_none() && h.app.session.catalog.len() == before + 1), "{}", notices(&h));
    let errors: Vec<&String> = h.app.notices.iter().filter(|n| n.starts_with("Could not copy")).collect();
    assert_eq!(errors.len(), 2, "{}", notices(&h));
    assert!(errors[0].contains("IMG_0002.png") && errors[1].contains("IMG_0003.png"), "{errors:?}");
    assert!(errors[0].contains("unplugged"), "{errors:?}");
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0, "no partial copies stay");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The copy job alone: a name taken in the staging folder gets a suffix, a short copy is an error
/// and removed, and ✕ stops before the next file with how many were left.
#[test]
fn camera_copy_names_sizes_and_cancel() {
    let dir = std::env::temp_dir().join(format!("lc-camera-copy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("IMG_0001.png"), b"left from before").unwrap();
    let mut files = photos(3);
    // the camera says one byte more than it sends
    files[2].0.size += 1;
    let mut camera = FakeCamera { photos: files.clone(), gate: Arc::new(AtomicBool::new(true)), unplug_after: None, copies: Arc::default() };
    let activity = lightcraft_engine::activity::Activity::default();
    let guard = activity.start("import", "test", lightcraft_engine::activity::Cancel::No);
    let list: Vec<CameraFile> = files.iter().map(|(f, _)| f.clone()).collect();
    let out = crate::camera_import::copy_files(&mut camera, &list, &dir, &AtomicBool::new(false), &guard.handle());
    let names: Vec<String> = out.paths.iter().map(|p| Path::new(p).file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["IMG_0001-2.png", "IMG_0002.png"]);
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
    assert!(out.errors[0].contains("IMG_0003.png") && out.errors[0].contains("bytes arrived"), "{:?}", out.errors);
    assert!(!dir.join("IMG_0003.png").exists(), "the short copy is removed");
    // cancelled: nothing more is copied
    let out = crate::camera_import::copy_files(&mut camera, &list, &dir.join("again"), &AtomicBool::new(true), &guard.handle());
    assert!(out.paths.is_empty());
    assert_eq!(out.errors, ["Copying from the camera was cancelled: 3 of 3 photos were not copied."]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A reader's file name is joined onto the staging folder only when it is a plain name: one with
/// a folder or `..` is an error and nothing is written outside the folder.
#[test]
fn camera_copy_refuses_names_that_leave_staging() {
    let dir = std::env::temp_dir().join(format!("lc-camera-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let staging = dir.join("staging");
    let mut files = photos(4);
    files[0].0.name = "../escaped.png".into();
    files[1].0.name = "..".into();
    files[2].0.name = "sub/inner.png".into();
    let mut camera = FakeCamera { photos: files.clone(), gate: Arc::new(AtomicBool::new(true)), unplug_after: None, copies: Arc::default() };
    let activity = lightcraft_engine::activity::Activity::default();
    let guard = activity.start("import", "test", lightcraft_engine::activity::Cancel::No);
    let list: Vec<CameraFile> = files.iter().map(|(f, _)| f.clone()).collect();
    let out = crate::camera_import::copy_files(&mut camera, &list, &staging, &AtomicBool::new(false), &guard.handle());
    assert_eq!(out.paths.len(), 1, "{:?}", out.errors);
    assert!(out.paths[0].ends_with("IMG_0004.png"), "{:?}", out.paths);
    assert_eq!(out.errors.len(), 3, "{:?}", out.errors);
    assert!(out.errors.iter().all(|e| e.contains("not a plain file name")), "{:?}", out.errors);
    assert!(!dir.join("escaped.png").exists(), "nothing written outside the staging folder");
    let _ = std::fs::remove_dir_all(&dir);
}
