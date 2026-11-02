//! Import from a camera connected by USB (APK-59; Android: MTP).
//!
//! A camera is a device of [`DeviceKind::Camera`](lightcraft_engine::devices::DeviceKind) in the
//! session's device watch, so File ▸ Import from Device and an album's Import from Device list it
//! next to the cards, and `file.addFromDevice {path: "usb:…"}` comes here. The host gives
//! [`crate::Services::cameras`]: the cameras connected, the permission to open one, and a job
//! with its [`CameraReader`](lightcraft_engine::devices::CameraReader) on a worker.
//!
//! The flow, each step on a worker that holds an activity row with ✕ (`Cancel::Flag`):
//! 1. The permission ([`CameraService::request`]): granted → on; asked → the import waits, polled
//!    every frame within 500 ms ([`poll`]), and goes on once allowed; refused → the host's message.
//! 2. "Reading the camera": the reader lists its photos, those the library has already (same name,
//!    size and day, [`lightcraft_engine::devices::known_files`]) marked, and the import review
//!    opens on them: those marked show "In library" and are not checked, nothing is copied yet.
//! 3. Import: "Copying from <camera>" copies the checked photos into the host's staging folder
//!    ([`CameraService::staging`], outside the library), checking ✕ between files and each copy's
//!    size; a file that fails (the camera unplugged mid-import, a full disk) is a per-file error in
//!    the notices and its partial copy is removed.
//! 4. The copies go to the importer's Move (verified by content hash, then removed from the staging
//!    folder) with the review's album, preset, keywords, destination and naming.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use lightcraft_engine::activity::{Cancel, TaskGuard, TaskHandle};
use lightcraft_engine::devices::{CameraFile, CameraReader, CamerasFn, WithCamera};
use lightcraft_engine::import::ImportCandidate;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::import::ImportDialog;

/// The activity row while the camera's photos are listed.
pub const LISTING: &str = "Reading the camera";

/// The activity row while photos are copied from `camera`.
pub fn copying_label(camera: &str) -> String {
    format!("Copying from {camera}")
}

/// How often a pending permission is looked at.
const PERMISSION_POLL: Duration = Duration::from_millis(500);

/// Where the permission to open a camera stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CameraAccess {
    Granted,
    /// Asked: the system dialog is up.
    Pending,
    /// Refused: what to do about it.
    Denied(String),
}

/// What the host gives for cameras connected by USB ([`crate::Services::cameras`]).
pub struct CameraService {
    /// The connected cameras (the device watch's thread).
    pub list: CamerasFn,
    /// The import was asked for (UI thread): where the permission stands, asking for it when it
    /// isn't granted (again after a refusal: the user chose the camera once more).
    pub request: Box<dyn Fn(&str) -> CameraAccess>,
    /// Where the asked permission stands now (UI thread, every frame while it is pending).
    pub status: Box<dyn Fn(&str) -> CameraAccess>,
    /// A job with the camera's reader, on the calling worker.
    pub with_camera: WithCamera,
    /// The staging folder copies wait in until the import moved them (outside the library).
    pub staging: PathBuf,
}

/// The camera an import review lists (the review's candidates are its files, in order).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraSource {
    /// The host's id for the camera.
    pub id: String,
    pub name: String,
    pub files: Vec<CameraFile>,
}

/// What a copy job did.
#[derive(Debug, Default)]
pub struct Copied {
    /// The copies in the staging folder.
    pub paths: Vec<String>,
    /// One message per file that was not copied.
    pub errors: Vec<String>,
}

/// A camera import in flight ([`LightcraftApp::camera`]).
pub enum CameraTask {
    /// Waiting for the permission.
    Waiting { id: String, name: String, album: Option<u64> },
    /// Listing its photos.
    Listing { id: String, name: String, album: Option<u64>, rx: Receiver<Result<Vec<CameraFile>, String>>, guard: TaskGuard },
    /// Copying the chosen photos.
    Copying { rx: Receiver<Copied>, guard: TaskGuard, params: Value },
    /// Copied, waiting for the importer (another import was running).
    Staged { paths: Vec<String>, params: Value },
}

impl CameraTask {
    /// `{state}` for `ui.inspect`.
    pub fn status(&self) -> Value {
        match self {
            CameraTask::Waiting { .. } => json!({"state": "permission"}),
            CameraTask::Listing { .. } => json!({"state": "listing"}),
            CameraTask::Copying { .. } => json!({"state": "copying"}),
            CameraTask::Staged { paths, .. } => json!({"state": "staged", "files": paths.len()}),
        }
    }
}

/// `file.addFromDevice` on a camera (`path` is `usb:<id>`): asks for the permission, then lists
/// the camera into the import review. `album` is the album the review starts on.
pub(crate) fn open(app: &mut LightcraftApp, id: &str, album: Option<u64>) -> Result<Value, String> {
    let Some(service) = &app.services.cameras else { return Err("Cameras connected by USB can't be read here".into()) };
    if app.camera.is_some() {
        return Err("A camera import is running".into());
    }
    // the label from the watch's last scan; a camera it doesn't list yet is asked for directly
    let name = app.session.devices.get().into_iter().find(|d| lightcraft_engine::devices::camera_id(&d.path) == Some(id)).map(|d| d.name);
    let name = match name {
        Some(name) => name,
        None => (service.list)().into_iter().find(|c| c.id == id).map(|c| c.name).ok_or("The camera is no longer connected. Plug it in again.")?,
    };
    match (service.request)(id) {
        CameraAccess::Granted => start_listing(app, id.to_owned(), name, album),
        CameraAccess::Pending => {
            app.camera = Some(CameraTask::Waiting { id: id.to_owned(), name, album });
            Ok(json!({"waiting": true}))
        }
        CameraAccess::Denied(message) => Err(message),
    }
}

fn start_listing(app: &mut LightcraftApp, id: String, name: String, album: Option<u64>) -> Result<Value, String> {
    let Some(service) = &app.services.cameras else { return Err("Cameras connected by USB can't be read here".into()) };
    let with_camera = service.with_camera.clone();
    let known = lightcraft_engine::devices::known_files(&app.session.catalog);
    let cancel = Arc::new(AtomicBool::new(false));
    let guard = app.session.activity.start("scan", LISTING, Cancel::Flag(cancel.clone()));
    guard.detail(&name);
    let (tx, rx) = std::sync::mpsc::channel();
    let camera = id.clone();
    let spawned = std::thread::Builder::new().name("camera-list".into()).spawn(move || {
        let mut listed: Option<Result<Vec<CameraFile>, String>> = None;
        let ran = with_camera(&camera, &mut |reader: &mut dyn CameraReader| listed = Some(reader.files(&known, &cancel)));
        let answer = match (ran, listed) {
            (Err(e), _) => Err(e),
            (Ok(()), Some(listed)) => listed,
            (Ok(()), None) => Err("The camera could not be read.".to_owned()),
        };
        let _ = tx.send(answer);
    });
    if let Err(e) = spawned {
        return Err(format!("Could not read the camera: {e}"));
    }
    app.camera = Some(CameraTask::Listing { id, name, album, rx, guard });
    Ok(json!({"reading": true}))
}

/// The review's items for a camera's files: the library's ones marked "In library".
pub fn candidates(device_path: &str, files: &[CameraFile]) -> Vec<ImportCandidate> {
    files
        .iter()
        .map(|f| {
            let ext = Path::new(&f.name).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
            ImportCandidate {
                path: format!("{device_path}/{}", f.name),
                name: f.name.clone(),
                format: ext,
                file_size: f.size,
                captured: f.date.and_then(lightcraft_engine::devices::date_text),
                duplicate: f.in_library.then(|| "path".to_owned()),
                ..Default::default()
            }
        })
        .collect()
}

/// The copies' names in `dir`: `name`, or `stem-2.ext`, `stem-3.ext`… when it is taken there or
/// earlier in the same job.
fn free_name(dir: &Path, name: &str, given: &[String]) -> String {
    let taken = |n: &str| dir.join(n).exists() || given.iter().any(|g| g.eq_ignore_ascii_case(n));
    if !taken(name) {
        return name.to_owned();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, Some(e)),
        _ => (name, None),
    };
    (2..10_000)
        .map(|i| match ext {
            Some(e) => format!("{stem}-{i}.{e}"),
            None => format!("{stem}-{i}"),
        })
        .find(|n| !taken(n))
        .unwrap_or_else(|| format!("{stem}-{}", std::process::id()))
}

/// Copies `files` with `reader` into `dir` (the job itself, on the calling thread), checking
/// `cancel` between files; a copy whose size is not the camera's is an error and removed.
pub fn copy_files(reader: &mut dyn CameraReader, files: &[CameraFile], dir: &Path, cancel: &AtomicBool, row: &TaskHandle) -> Copied {
    let mut out = Copied::default();
    let mut given: Vec<String> = Vec::new();
    let total = u64::try_from(files.len()).unwrap_or(u64::MAX);
    if let Err(e) = std::fs::create_dir_all(dir) {
        out.errors.push(format!("Could not copy from the camera: the folder {} can't be made ({e}).", dir.display()));
        return out;
    }
    for (done, f) in files.iter().enumerate() {
        row.progress(u64::try_from(done).unwrap_or(u64::MAX), total);
        if cancel.load(Ordering::Relaxed) {
            let left = files.len().saturating_sub(done);
            out.errors.push(format!("Copying from the camera was cancelled: {left} of {} photos were not copied.", files.len()));
            break;
        }
        row.detail(&f.name);
        let name = free_name(dir, &f.name, &given);
        let target = dir.join(&name);
        let copied = reader.copy(f, &target).and_then(|()| match std::fs::metadata(&target) {
            Ok(m) if m.len() == f.size => Ok(()),
            Ok(m) => Err(format!("{} of {} bytes arrived", m.len(), f.size)),
            Err(e) => Err(e.to_string()),
        });
        match copied {
            Ok(()) => {
                given.push(name);
                out.paths.push(target.to_string_lossy().into_owned());
            }
            Err(e) => {
                log::warn!("camera copy {}: {e}", f.name);
                let _ = std::fs::remove_file(&target);
                out.errors.push(format!("Could not copy {} from the camera: {e}", f.name));
            }
        }
    }
    row.progress(total, total);
    out
}

/// The review's Import on a camera: copies the checked photos on a worker, then imports them
/// (Move from the staging folder) with the review's other choices.
pub(crate) fn start_copy(app: &mut LightcraftApp, d: &ImportDialog) -> Result<Value, String> {
    let Some(source) = &d.camera else { return Err("not a camera".into()) };
    let Some(service) = &app.services.cameras else { return Err("Cameras connected by USB can't be read here".into()) };
    if app.camera.is_some() {
        return Err("A camera import is running".into());
    }
    let files: Vec<CameraFile> = d.selected().into_iter().filter_map(|i| source.files.get(i)).filter(|f| !f.in_library).cloned().collect();
    if files.is_empty() {
        return Err("no photos selected".into());
    }
    let (with_camera, dir, id) = (service.with_camera.clone(), service.staging.clone(), source.id.clone());
    // the copies are the importer's to move: the review's other choices stay
    let mut copy = d.clone();
    copy.copy = true;
    copy.move_files = true;
    let params = crate::import::review_params(app, &copy)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let guard = app.session.activity.start("import", &copying_label(&source.name), Cancel::Flag(cancel.clone()));
    let row = guard.handle();
    let (tx, rx) = std::sync::mpsc::channel();
    let count = files.len();
    let spawned = std::thread::Builder::new().name("camera-copy".into()).spawn(move || {
        let mut copied = None;
        let ran = with_camera(&id, &mut |reader: &mut dyn CameraReader| copied = Some(copy_files(reader, &files, &dir, &cancel, &row)));
        let copied = match (ran, copied) {
            (_, Some(copied)) => copied,
            (Err(e), None) => Copied { errors: vec![e], ..Copied::default() },
            (Ok(()), None) => Copied::default(),
        };
        let _ = tx.send(copied);
    });
    if let Err(e) = spawned {
        return Err(format!("Could not copy from the camera: {e}"));
    }
    app.camera = Some(CameraTask::Copying { rx, guard, params });
    Ok(json!({"copying": count}))
}

/// Advance the camera import (called every frame).
pub fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(task) = app.camera.take() else { return };
    ctx.request_repaint_after(PERMISSION_POLL.min(Duration::from_millis(100)));
    match task {
        CameraTask::Waiting { id, name, album } => {
            let status = app.services.cameras.as_ref().map(|s| (s.status)(&id));
            match status {
                Some(CameraAccess::Pending) => {
                    app.camera = Some(CameraTask::Waiting { id, name, album });
                    ctx.request_repaint_after(PERMISSION_POLL);
                }
                Some(CameraAccess::Granted) => {
                    if let Err(e) = start_listing(app, id, name, album) {
                        app.notices.push(e);
                    }
                }
                Some(CameraAccess::Denied(message)) => app.notices.push(message),
                None => {}
            }
        }
        CameraTask::Listing { id, name, album, rx, guard } => match rx.try_recv() {
            Err(TryRecvError::Empty) => {
                if !guard.is_cancelled() {
                    app.camera = Some(CameraTask::Listing { id, name, album, rx, guard });
                }
            }
            Err(TryRecvError::Disconnected) => app.notices.push("Reading the camera stopped unexpectedly.".to_owned()),
            Ok(_) if guard.is_cancelled() => {}
            Ok(Err(e)) => app.notices.push(e),
            Ok(Ok(files)) => {
                drop(guard);
                open_review(app, id, name, album, files);
            }
        },
        CameraTask::Copying { rx, guard, params } => match rx.try_recv() {
            Err(TryRecvError::Empty) => app.camera = Some(CameraTask::Copying { rx, guard, params }),
            Err(TryRecvError::Disconnected) => app.notices.push("Copying from the camera stopped unexpectedly.".to_owned()),
            Ok(copied) => {
                drop(guard);
                app.notices.extend(copied.errors);
                if !copied.paths.is_empty() {
                    app.camera = Some(CameraTask::Staged { paths: copied.paths, params });
                    import_staged(app);
                }
            }
        },
        staged @ CameraTask::Staged { .. } => {
            app.camera = Some(staged);
            import_staged(app);
        }
    }
}

/// The copies to the importer, once it is free.
fn import_staged(app: &mut LightcraftApp) {
    let Some(CameraTask::Staged { paths, params }) = app.camera.take() else { return };
    match crate::import::start_paths_with(app, paths.clone(), params.clone()) {
        Ok(_) => {}
        // (an import, a folder scan or a synchronize running: tried again next frame)
        Err(_) if app.import.is_some() || app.scan.as_ref().is_some_and(|t| !t.browse()) || app.sync_run.is_some() => {
            app.camera = Some(CameraTask::Staged { paths, params });
        }
        Err(e) => app.notices.push(format!("Could not import the photos copied from the camera: {e}")),
    }
}

fn open_review(app: &mut LightcraftApp, id: String, name: String, album: Option<u64>, files: Vec<CameraFile>) {
    if files.is_empty() {
        app.notices.push(format!("No photos found on {name}."));
        return;
    }
    let path = lightcraft_engine::devices::Device::camera(&name, &id).path;
    app.renderer.forget_imports();
    let mut d = ImportDialog::new(candidates(&path, &files));
    d.copy = true;
    d.album = album.filter(|a| app.session.catalog.album(lightcraft_catalog::AlbumId(*a)).is_some_and(|a| !a.folder && !a.is_smart()));
    d.sources = vec![name.clone()];
    d.camera = Some(CameraSource { id, name, files });
    app.ui.dialog = Some(crate::state::Dialog::Import { opts: Box::new(d) });
}
