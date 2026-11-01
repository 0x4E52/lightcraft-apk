//! Files other apps sent LightCraft ("Open with", share one, share several; APK-25).
//!
//! `run` reads `getIntent()` once per start on android_main (NativeActivity has no
//! `onNewIntent`, so a share while LightCraft runs only brings it to the front) and hands the
//! URIs to [`Shares`], which the eframe app ticks every frame. Once the library is open,
//! [`Shares::tick`] starts [`start`]: a worker that holds an activity row ("Receiving N files",
//! cancellable) for the whole job and copies each URI at once, before the Activity and its read
//! grant end, through the [`ContentReader`] it is given (on Android `content::with_content`,
//! which attaches the worker to the JVM; in tests a fake). Each file is named by
//! `sanitize_display_name` and routed by `route_received`: photos and raws are copied into the
//! staging folder `files/incoming/` (outside the library, so the importer really moves them) and
//! then imported with the importer's Move (into the library's `Originals/`, the staged copy
//! removed once verified), everything else (presets, track logs, keyword lists, models) into the
//! Inbox, where the in-app file chooser finds it (APK-24). A file that can't be received
//! (revoked, unreadable, half read, over 2 GB, cancelled, or no folder for it) is a per-file
//! error in the notices, and its partial copy is removed. The staging folder is emptied when a
//! job starts and at a start without a share, so what an earlier start left (a duplicate the
//! import skipped, a killed copy) goes; a file a catalog record points at is never removed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, TryRecvError};

use lightcraft_android_sys::content::{ContentReader, CopyError, MAX_BYTES, Route, route_received, sanitize_display_name};
use lightcraft_engine::activity::{Activity, Cancel, TaskGuard};
use lightcraft_ui_egui::LightcraftApp;

/// The staging folder under the app's internal storage (`files/incoming`, next to the library,
/// never inside it) that received photos wait in until the import moved them into `Originals/`.
pub const INCOMING_DIR: &str = "incoming";

/// Runs the job with the device's content reader (Android: `content::with_content`, attached
/// to the JVM on the calling worker); `Err` when there is none, as text the user can act on.
pub type WithReader = Box<dyn FnOnce(&mut dyn FnMut(&mut dyn ContentReader)) -> Result<(), String> + Send>;

/// Where received files go (`None`: the host has no such folder, and a file for it is an error).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dirs {
    /// `files/incoming`: photos, until the import moved them (outside the library).
    pub incoming: Option<PathBuf>,
    /// `files/inbox`: everything else.
    pub inbox: Option<PathBuf>,
}

impl Dirs {
    pub fn new(incoming: &Path, inbox: &Path) -> Dirs {
        Dirs { incoming: Some(incoming.to_path_buf()), inbox: Some(inbox.to_path_buf()) }
    }
}

/// What a job received.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Received {
    /// Photos in the staging folder, for the import.
    pub photos: Vec<String>,
    /// Files saved to the Inbox.
    pub inbox: Vec<PathBuf>,
    /// One message per file that could not be received.
    pub errors: Vec<String>,
}

/// The activity row's label.
pub fn label(count: usize) -> String {
    if count == 1 { "Receiving 1 file".to_owned() } else { format!("Receiving {count} files") }
}

/// The files in `dir` that catalog records point at (the import never leaves one there, as the
/// folder is outside the library; this is the guard that a record never loses its original).
pub fn in_use(catalog: &lightcraft_engine::catalog::Catalog, dir: &Path) -> HashSet<PathBuf> {
    catalog
        .photos()
        .filter_map(|p| match &p.source {
            lightcraft_engine::catalog::Source::File { path } => Some(PathBuf::from(path)),
            lightcraft_engine::catalog::Source::Demo { .. } => None,
        })
        .filter(|p| p.starts_with(dir))
        .collect()
}

/// Removes what an earlier start left in the staging folder, except the files in `keep` (those a
/// catalog record points at, [`in_use`]).
pub fn clear_incoming(dir: &Path, keep: &HashSet<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            log::warn!("can't empty {}: {e}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if keep.contains(&path) {
            log::warn!("{} stays: the catalog uses it", path.display());
            continue;
        }
        let removed = if entry.file_type().is_ok_and(|t| t.is_dir()) { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        if let Err(e) = removed {
            log::warn!("can't remove {}: {e}", path.display());
        }
    }
}

/// Copies every URI with `reader` (the job itself, on the calling thread), checking `cancel`
/// between files and between chunks; `photos` are the importer's extensions.
pub fn receive(reader: &mut dyn ContentReader, uris: &[String], dirs: &Dirs, photos: &[&str], cancel: &AtomicBool, row: &TaskGuard) -> Received {
    let mut out = Received::default();
    let mut given: Vec<(Route, String)> = Vec::new();
    let total = u64::try_from(uris.len()).unwrap_or(u64::MAX);
    for (done, uri) in uris.iter().enumerate() {
        row.progress(u64::try_from(done).unwrap_or(u64::MAX), total);
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let left = uris.len().saturating_sub(done);
            out.errors.push(format!("Receiving was cancelled: {left} of {} files were not received.", uris.len()));
            break;
        }
        let meta = match reader.meta(uri) {
            Ok(meta) => meta,
            Err(e) => {
                log::warn!("receive {uri}: {e}");
                out.errors.push(e.user_message("receive a shared file"));
                continue;
            }
        };
        let mime = meta.mime.as_deref();
        // the route from the sender's name and type; then a name not taken in that folder
        let probe = sanitize_display_name(meta.name.as_deref(), mime, |_| false);
        let route = route_received(&probe, mime, photos);
        let Some(dir) = (match route {
            Route::Import => dirs.incoming.as_ref(),
            Route::Inbox => dirs.inbox.as_ref(),
        }) else {
            log::warn!("receive {uri}: no folder for {route:?}");
            out.errors.push(format!("LightCraft has no folder to keep {probe} in, so it was not received."));
            continue;
        };
        let name = sanitize_display_name(meta.name.as_deref(), mime, |n| {
            dir.join(n).exists() || given.iter().any(|(r, g)| *r == route && g.eq_ignore_ascii_case(n))
        });
        row.detail(&name);
        if meta.size.is_some_and(|s| u64::try_from(s).is_ok_and(|s| s > MAX_BYTES)) {
            out.errors.push(CopyError::TooBig.user_message(&name));
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            out.errors.push(CopyError::Write(e.to_string()).user_message(&name));
            continue;
        }
        let target = dir.join(&name);
        match reader.copy_to(uri, &target, MAX_BYTES, cancel) {
            Ok(bytes) => {
                log::info!("received {name} ({bytes} bytes) → {}", target.display());
                given.push((route, name));
                match route {
                    Route::Import => out.photos.push(target.to_string_lossy().into_owned()),
                    Route::Inbox => out.inbox.push(target),
                }
            }
            Err(e) => {
                log::warn!("receive {uri}: {e:?}");
                out.errors.push(e.user_message(&name));
            }
        }
    }
    row.progress(total, total);
    out
}

/// A receiving job in flight: its answer arrives once, on [`Incoming::poll`].
pub struct Incoming {
    rx: Receiver<Received>,
}

impl Incoming {
    /// The job's answer, once it is done (`None` while it runs). A worker that died answers
    /// with an error, never a panic here.
    pub fn poll(&mut self) -> Option<Received> {
        match self.rx.try_recv() {
            Ok(received) => Some(received),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Received { errors: vec!["Receiving the shared files stopped unexpectedly.".to_owned()], ..Received::default() })
            }
        }
    }
}

/// Starts receiving `uris` on a worker that holds an activity row in `activity` (kind `import`,
/// cancellable) until the last file is copied; the staging folder is emptied first, except `keep`.
pub fn start(
    activity: &Activity,
    uris: Vec<String>,
    dirs: Dirs,
    keep: HashSet<PathBuf>,
    photos: &'static [&'static str],
    with_reader: WithReader,
) -> Incoming {
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let row = activity.start("import", &label(uris.len()), Cancel::Flag(cancel.clone()));
    let spawned = std::thread::Builder::new().name("receive".to_owned()).spawn(move || {
        let row = row;
        if let Some(incoming) = &dirs.incoming {
            clear_incoming(incoming, &keep);
        }
        let mut received = None;
        let ran = with_reader(&mut |reader| received = Some(receive(reader, &uris, &dirs, photos, &cancel, &row)));
        let received = match (ran, received) {
            (_, Some(received)) => received,
            (Err(e), None) => Received { errors: vec![e], ..Received::default() },
            (Ok(()), None) => Received::default(),
        };
        // (the row goes before the answer, so a frame that sees the answer sees no row)
        drop(row);
        let _ = tx.send(received);
    });
    if let Err(e) = spawned {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(Received { errors: vec![format!("Could not receive the shared files: {e}")], ..Received::default() });
        return Incoming { rx };
    }
    Incoming { rx }
}

/// What the eframe app holds for the files LightCraft was sent at this start.
pub struct Shares {
    /// The URIs, until the job starts.
    uris: Vec<String>,
    with_reader: Option<WithReader>,
    job: Option<Incoming>,
    /// Received photos waiting for the import (another import running, the library problem up).
    photos: Vec<String>,
    /// The staging folder was looked at this start.
    cleared: bool,
}

impl Shares {
    /// The URIs `getIntent()` named, and how to read them.
    pub fn new(uris: Vec<String>, with_reader: Option<WithReader>) -> Shares {
        Shares { uris, with_reader, job: None, photos: Vec::new(), cleared: false }
    }

    /// Nothing was sent.
    pub fn none() -> Shares {
        Shares::new(Vec::new(), None)
    }

    /// Whether something is still to do (the app asks for frames until it is done).
    pub fn busy(&self) -> bool {
        self.job.is_some() || !self.photos.is_empty() || (!self.uris.is_empty() && self.with_reader.is_some())
    }

    /// One frame with the running app: starts the job, reads its answer, hands the photos to the
    /// import once it can take them. A folder missing from `dirs` makes its files errors.
    pub fn tick(&mut self, app: &mut LightcraftApp, dirs: &Dirs) {
        let keep = || dirs.incoming.as_deref().map(|d| in_use(&app.session.catalog, d)).unwrap_or_default();
        if !self.uris.is_empty()
            && let Some(with_reader) = self.with_reader.take()
        {
            let uris = std::mem::take(&mut self.uris);
            log::info!("receiving {} shared files", uris.len());
            self.cleared = true;
            let keep = keep();
            self.job = Some(start(&app.session.activity, uris, dirs.clone(), keep, lightcraft_engine::import::EXTENSIONS, with_reader));
        } else if !self.cleared {
            // nothing sent: what an earlier start left goes now
            self.cleared = true;
            if let Some(incoming) = &dirs.incoming {
                clear_incoming(incoming, &keep());
            }
        }
        if let Some(received) = self.job.as_mut().and_then(Incoming::poll) {
            self.job = None;
            self.take(app, received);
        }
        self.import_photos(app);
    }

    /// A finished job's answer: errors and the Inbox files said in the notices, photos queued.
    fn take(&mut self, app: &mut LightcraftApp, received: Received) {
        app.notices.extend(received.errors);
        if !received.inbox.is_empty() {
            let names: Vec<String> = received.inbox.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
            app.notices.push(format!(
                "Saved to the Inbox: {}. Open them with the command that uses them (Import Presets…, Auto-Tag from Tracklog…), choosing Inbox.",
                names.join(", ")
            ));
        }
        self.photos.extend(received.photos);
    }

    /// The waiting photos to the import (Move: into `Originals/`), once no import runs and the
    /// library problem window (if any) was answered.
    fn import_photos(&mut self, app: &mut LightcraftApp) {
        if self.photos.is_empty() || app.import.is_some() || app.library_problem.as_ref().is_some_and(|p| !p.dismissed) {
            return;
        }
        let photos = std::mem::take(&mut self.photos);
        match lightcraft_ui_egui::import::start_paths_with(app, photos.clone(), serde_json::json!({"mode": "move"})) {
            Ok(_) => app.ui.view = lightcraft_ui_egui::state::ViewMode::PhotoGrid,
            // (a folder scan or a synchronize running: tried again next frame)
            Err(e) if app.import.is_none() && (app.scan.is_some() || app.sync_run.is_some()) => {
                log::info!("received photos wait: {e}");
                self.photos = photos;
            }
            Err(e) => app.notices.push(format!("Could not import the received photos: {e}")),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lightcraft_android_sys::content::{Meta, copy_to_file};
    use std::sync::Mutex;
    use std::sync::mpsc::{Sender, channel};
    use std::time::{Duration, Instant};

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-android-incoming-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A provider: per URI a name, type and content (`None`: the grant was revoked); the copy of
    /// a URI in `hold` waits until the test says go.
    struct Fake {
        files: Vec<(&'static str, Option<(&'static str, &'static str, Vec<u8>)>)>,
        hold: Option<(&'static str, Sender<()>, Arc<Mutex<Receiver<()>>>)>,
    }

    impl ContentReader for Fake {
        fn meta(&mut self, uri: &str) -> Result<Meta, lightcraft_android_sys::Error> {
            match self.files.iter().find(|(u, _)| *u == uri).and_then(|(_, f)| f.as_ref()) {
                Some((name, mime, data)) => {
                    Ok(Meta { name: Some((*name).to_owned()), size: Some(data.len() as i64), mime: Some((*mime).to_owned()) })
                }
                None => Err(lightcraft_android_sys::Error::Java { class: "java.lang.SecurityException".into(), message: "revoked".into() }),
            }
        }

        fn copy_to(&mut self, uri: &str, target: &Path, cap: u64, cancel: &AtomicBool) -> Result<u64, CopyError> {
            if let Some((held, started, go)) = &self.hold
                && *held == uri
            {
                started.send(()).unwrap();
                go.lock().unwrap().recv().unwrap();
            }
            let (_, _, data) = self.files.iter().find(|(u, _)| *u == uri).and_then(|(_, f)| f.as_ref()).unwrap();
            copy_to_file(&mut &data[..], target, cap, cancel)
        }
    }

    const PHOTOS: &[&str] = &["jpg", "dng"];

    /// A share being received shows its row in the activity stack while it copies, and the row is
    /// gone once it is done; the photo waits in the staging folder, the preset is in the Inbox and
    /// the revoked URI is a per-file error. What an earlier start left goes, except a file a
    /// catalog record points at.
    #[test]
    fn received_files_show_activity_row() {
        let dir = temp("row");
        let (incoming, inbox) = (dir.join(INCOMING_DIR), dir.join("inbox"));
        // left by an earlier start: emptied when the job starts
        std::fs::create_dir_all(incoming.join(".IMG_9.jpg.part")).unwrap();
        std::fs::write(incoming.join("old.jpg"), b"old").unwrap();
        std::fs::write(incoming.join("used.jpg"), b"used").unwrap();
        let keep: HashSet<PathBuf> = [incoming.join("used.jpg")].into_iter().collect();
        let session = lightcraft_engine::Session::new();
        let (started_tx, started_rx) = channel();
        let (go_tx, go_rx) = channel();
        let fake = Fake {
            files: vec![
                ("content://p/1", Some(("IMG_1.jpg", "image/jpeg", vec![1u8; 5000]))),
                ("content://p/2", Some(("Look.xmp", "application/rdf+xml", b"<x:xmpmeta/>".to_vec()))),
                ("content://p/3", None),
            ],
            hold: Some(("content://p/2", started_tx, Arc::new(Mutex::new(go_rx)))),
        };
        let with_reader: WithReader = Box::new(move |job| {
            let mut fake = fake;
            job(&mut fake);
            Ok(())
        });
        let uris = vec!["content://p/1".to_owned(), "content://p/2".to_owned(), "content://p/3".to_owned()];
        let mut job = start(&session.activity, uris, Dirs::new(&incoming, &inbox), keep, PHOTOS, with_reader);
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let rows = session.activity.list();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!((rows[0].kind, rows[0].label.as_str()), ("import", "Receiving 3 files"));
        assert!(rows[0].cancellable);
        assert_eq!(rows[0].detail, "Look.xmp");
        assert!(job.poll().is_none(), "still copying");
        go_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let received = loop {
            if let Some(r) = job.poll() {
                break r;
            }
            assert!(Instant::now() < deadline, "the job never answered");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(session.activity.list().is_empty(), "the row is gone once done");
        let photo = incoming.join("IMG_1.jpg");
        assert_eq!(received.photos, [photo.to_string_lossy().into_owned()]);
        assert_eq!(std::fs::read(&photo).unwrap().len(), 5000);
        assert!(!incoming.join("old.jpg").exists(), "an earlier start's leftovers went");
        assert!(!incoming.join(".IMG_9.jpg.part").exists(), "folders too");
        assert_eq!(std::fs::read(incoming.join("used.jpg")).unwrap(), b"used", "a catalogued file is never removed");
        assert_eq!(received.inbox, [inbox.join("Look.xmp")]);
        assert_eq!(std::fs::read(inbox.join("Look.xmp")).unwrap(), b"<x:xmpmeta/>");
        assert_eq!(received.errors.len(), 1, "{:?}", received.errors);
        assert!(received.errors[0].contains("Access was withdrawn"), "{:?}", received.errors);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A name already in the Inbox gets a number; the size from the provider over 2 GB is refused
    /// before anything is read; a cancel stops between files; no reader is an error; a host
    /// without the folder a file needs says so per file.
    #[test]
    fn received_files_names_caps_and_cancel() {
        let dir = temp("names");
        let (incoming, inbox) = (dir.join(INCOMING_DIR), dir.join("inbox"));
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("Look.xmp"), b"mine").unwrap();
        struct Big;
        impl ContentReader for Big {
            fn meta(&mut self, _: &str) -> Result<Meta, lightcraft_android_sys::Error> {
                Ok(Meta { name: Some("huge.dng".into()), size: Some(3 << 30), mime: None })
            }
            fn copy_to(&mut self, _: &str, _: &Path, _: u64, _: &AtomicBool) -> Result<u64, CopyError> {
                panic!("an oversized file is never read")
            }
        }
        let activity = Activity::default();
        let row = activity.start("import", "t", Cancel::No);
        let cancel = AtomicBool::new(false);
        let dirs = Dirs::new(&incoming, &inbox);
        let got = receive(&mut Big, &["content://b/1".to_owned()], &dirs, PHOTOS, &cancel, &row);
        assert!(got.photos.is_empty() && got.errors[0].contains("larger than 2 GB"), "{got:?}");

        let mut fake = Fake {
            files: vec![
                ("content://a/1", Some(("Look.xmp", "application/rdf+xml", b"one".to_vec()))),
                ("content://a/2", Some(("Look.xmp", "application/rdf+xml", b"two".to_vec()))),
            ],
            hold: None,
        };
        let uris = ["content://a/1".to_owned(), "content://a/2".to_owned()];
        let got = receive(&mut fake, &uris, &dirs, PHOTOS, &cancel, &row);
        assert_eq!(got.inbox, [inbox.join("Look-1.xmp"), inbox.join("Look-2.xmp")], "{got:?}");
        assert_eq!(std::fs::read(inbox.join("Look.xmp")).unwrap(), b"mine", "an Inbox file is never overwritten");

        let no_inbox = Dirs { incoming: Some(incoming.clone()), inbox: None };
        let got = receive(&mut fake, &uris[..1], &no_inbox, PHOTOS, &cancel, &row);
        assert!(got.inbox.is_empty() && got.errors.len() == 1, "{got:?}");
        assert!(got.errors[0].contains("no folder to keep Look.xmp in"), "{got:?}");

        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let got = receive(&mut fake, &uris, &dirs, PHOTOS, &cancel, &row);
        assert!(got.inbox.is_empty() && got.errors[0].contains("cancelled: 2 of 2"), "{got:?}");

        let session = lightcraft_engine::Session::new();
        let mut job = start(&session.activity, uris.to_vec(), dirs, HashSet::new(), PHOTOS, Box::new(|_| Err("no JVM".to_owned())));
        let deadline = Instant::now() + Duration::from_secs(10);
        let got = loop {
            if let Some(r) = job.poll() {
                break r;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(got.errors, ["no JVM"]);
        assert_eq!(label(1), "Receiving 1 file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A JPEG as a provider would send it.
    fn jpeg() -> Vec<u8> {
        let img = lightcraft_raster::Rgba8::from_fn(24, 16, |x, y| [(x * 9) as u8, (y * 12) as u8, 80, 255]);
        let opts = lightcraft_engine::export::ExportOptions { format: lightcraft_engine::export::ExportFormat::Jpeg, ..Default::default() };
        lightcraft_engine::export::encode_image(&img, &opts).unwrap()
    }

    /// The app's side, in a library on disk: once the job answers, its errors and Inbox files are
    /// in the notices; while the library problem window is up the photos wait; then they go to the
    /// import as a Move, which runs to the end: the photo is in the library's `Originals/`, the
    /// staged copy is gone and the catalog points into `Originals/`, not the staging folder (the
    /// staging folder used to be inside the library, where Move kept the file in place and the
    /// next start deleted it).
    #[test]
    fn received_photos_go_to_the_import() {
        let dir = temp("import");
        let (library, incoming, inbox) = (dir.join("library"), dir.join(INCOMING_DIR), dir.join("inbox"));
        let (session, problem) = lightcraft_ui_egui::host::session_open::open_session(false, Some(library.clone()), false);
        assert!(problem.is_none(), "{problem:?}");
        let mut state = crate::state::HostState::new(Ok((session, None)), crate::state::Start { library: library.clone(), ..Default::default() });
        let app = &mut state.app;
        app.library_problem = Some(lightcraft_ui_egui::panels::library_problem::LibraryProblem::new(library.to_string_lossy(), "test".to_owned()));
        let fake = Fake {
            files: vec![
                ("content://p/1", Some(("IMG_1.jpg", "image/jpeg", jpeg()))),
                ("content://p/2", Some(("walk.gpx", "application/gpx+xml", b"<gpx/>".to_vec()))),
            ],
            hold: None,
        };
        let mut shares = Shares::new(
            vec!["content://p/1".to_owned(), "content://p/2".to_owned()],
            Some(Box::new(move |job| {
                let mut fake = fake;
                job(&mut fake);
                Ok(())
            })),
        );
        let dirs = Dirs::new(&incoming, &inbox);
        assert!(shares.busy());
        let deadline = Instant::now() + Duration::from_secs(10);
        while shares.job.is_some() || shares.photos.is_empty() {
            shares.tick(app, &dirs);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.notices.iter().any(|n| n.contains("Saved to the Inbox: walk.gpx")), "{:?}", app.notices);
        let staged = incoming.join("IMG_1.jpg");
        assert!(staged.is_file());
        assert!(app.import.is_none(), "the library problem window is up: the photos wait");
        app.library_problem.as_mut().unwrap().dismissed = true;
        shares.tick(app, &dirs);
        let task = app.import.as_ref().expect("the received photo is being imported");
        assert_eq!(task.params()["mode"], "move", "{}", task.params());
        assert!(!shares.busy());
        assert!(!Shares::none().busy());

        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        while app.import.is_some() {
            lightcraft_ui_egui::import::tick(app, &ctx);
            assert!(Instant::now() < deadline, "the import never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
        let paths: Vec<PathBuf> = app
            .session
            .catalog
            .photos()
            .filter_map(|p| match &p.source {
                lightcraft_engine::catalog::Source::File { path } => Some(PathBuf::from(path)),
                lightcraft_engine::catalog::Source::Demo { .. } => None,
            })
            .collect();
        assert_eq!(paths.len(), 1, "{paths:?} {:?}", app.notices);
        let stored = &paths[0];
        assert!(stored.starts_with(library.join("Originals")), "{}", stored.display());
        assert!(stored.is_file(), "{}", stored.display());
        assert!(!stored.starts_with(&incoming));
        assert!(!staged.exists(), "the staged copy was moved, not kept");
        assert!(in_use(&app.session.catalog, &incoming).is_empty());

        // the next start empties the staging folder; the photo stays
        let mut next = Shares::none();
        std::fs::write(incoming.join("left.jpg"), b"left").unwrap();
        next.tick(app, &dirs);
        assert!(!incoming.join("left.jpg").exists());
        assert!(stored.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
