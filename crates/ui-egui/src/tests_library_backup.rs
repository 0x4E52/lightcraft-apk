//! File ▸ Back Up Library… and Restore Library from Backup… on a device (APK-50), with fake host
//! services: the backup runs under its activity row and its zip goes to the share sheet; a
//! backup that can't fit (the space, or the zip's 4 GB) is refused before anything is written;
//! an action that panics stays installed; the first-start notice offers
//! Back Up Now; a restore replaces the library only once it checked out, keeps the previous one
//! until the restored one has opened, and puts it back when the restored one doesn't open.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::library_backup::{device_backup, device_restore, marker_path, previous_dir, staging_dir};
use crate::share::ShareFn;
use crate::{ExportKind, LightcraftApp, Services};
use lightcraft_engine::backup_dir::{BACKUP_LABEL, RESTORE_LABEL};

const T: Duration = Duration::from_secs(60);

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lc-ui-backup-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_png(path: &Path, seed: u8) {
    let (w, h) = (40usize, 30usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 5) as u8, (i / w * 7) as u8, seed, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A session over `<root>/library` with one Copy-imported photo rated 4; its id.
fn session(root: &Path) -> (lightcraft_engine::Session, lightcraft_engine::catalog::PhotoId) {
    let src = root.join("camera").join("a.png");
    write_png(&src, 3);
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(root.join("library"), false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [src.to_string_lossy()], "mode": "copy"})).unwrap();
    let id = lightcraft_engine::catalog::PhotoId(r["imported"][0].as_u64().unwrap());
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    s.execute("photo.rate", &json!({"rating": 4})).unwrap();
    (s, id)
}

/// The device's services: Documents for the zip, a share sheet that records what it was given.
fn device(root: &Path, services: Services) -> (Services, Arc<Mutex<Vec<Vec<String>>>>) {
    let shared = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let calls = shared.clone();
    let share: ShareFn = Arc::new(move |paths: &[String], _| {
        calls.lock().unwrap().push(paths.to_vec());
        Ok(())
    });
    let docs = root.join("Documents").join("LightCraft").to_string_lossy().into_owned();
    let services = Services {
        export_dir_for: Some(Box::new(move |_: ExportKind| docs.clone())),
        share: Some(share),
        backup_library: Some(device_backup()),
        restore_library: Some(device_restore()),
        png: None,
        ..services
    };
    (services, shared)
}

fn headless(app: LightcraftApp) -> Headless {
    let mut app = app;
    app.ui.settings.gpu = false;
    Headless::new(app, [1200.0, 800.0], 1.0)
}

fn row(h: &Headless, label: &str) -> Option<lightcraft_engine::activity::TaskInfo> {
    h.app.session.activity.list().into_iter().find(|t| t.label == label)
}

#[test]
fn backup_shows_activity_row_and_offers_share() {
    let root = temp("row");
    let (session, _) = session(&root);
    let (services, shared) = device(&root, Services::default());
    let mut h = headless(LightcraftApp::new(session, services));
    assert!(crate::menus::ui_enabled(&h.app, "file.backupLibrary"));
    let r = h.app.run("file.backupLibrary", json!({})).unwrap();
    assert_eq!(r["originals"], 1, "{r}");
    let path = r["path"].as_str().unwrap().to_string();
    assert!(path.starts_with(&root.join("Documents").join("LightCraft").to_string_lossy().into_owned()) && path.ends_with(".zip"), "{path}");
    let task = row(&h, BACKUP_LABEL).expect("the backup's activity row");
    assert!(task.cancellable && task.kind == "export");
    assert!(h.app.run("file.backupLibrary", json!({})).is_err(), "one at a time");
    assert!(h.step_until(T, |_| !shared.lock().unwrap().is_empty()), "the share sheet was offered");
    assert!(h.step_until(T, |h| h.app.share.is_none() && h.app.library_backup.is_none()));
    assert_eq!(shared.lock().unwrap()[0], vec![path.clone()], "exactly the backup file");
    assert!(Path::new(&path).is_file());
    assert!(row(&h, BACKUP_LABEL).is_none(), "the row is gone with the backup");
    assert!(h.app.ui.status.contains("Library backed up") && h.app.ui.status.contains("1 originals"), "{}", h.app.ui.status);
    // the zip is a backup a restore accepts
    let v = lightcraft_engine::backup_dir::verify_file(Path::new(&path), &std::sync::atomic::AtomicBool::new(false)).unwrap();
    assert_eq!(v.originals, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn backup_is_refused_before_writing_when_space_is_short() {
    let root = temp("space");
    let (session, _) = session(&root);
    let (services, shared) = device(&root, Services { free_space: Some(Box::new(|_: &Path| Some(1_000))), ..Default::default() });
    let mut h = headless(LightcraftApp::new(session, services));
    let e = h.app.run("file.backupLibrary", json!({})).unwrap_err();
    assert!(e.contains("Not enough space") && e.contains("1 KB free"), "{e}");
    assert!(h.app.library_backup.is_none() && row(&h, BACKUP_LABEL).is_none());
    let docs = root.join("Documents").join("LightCraft");
    assert_eq!(std::fs::read_dir(&docs).map(|d| d.count()).unwrap_or(0), 0, "nothing written");
    assert!(shared.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// A library over the zip's 4 GB (no zip64) is refused before the space check and before the
/// worker starts: no row, no file, and its originals aren't read.
#[test]
fn backup_over_4_gb_is_refused_before_it_starts() {
    let root = temp("limits");
    let (session, id) = session(&root);
    let lightcraft_engine::catalog::Source::File { path } = &session.catalog.photo(id).unwrap().source else { panic!() };
    // (sparse: 5 GB long, no disk used)
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_len(5_000_000_000).unwrap();
    // a space check would ask for more than 4 GB free first; this one says there is plenty
    let (services, shared) = device(&root, Services { free_space: Some(Box::new(|_: &Path| Some(u64::MAX))), ..Default::default() });
    let mut h = headless(LightcraftApp::new(session, services));
    let e = h.app.run("file.backupLibrary", json!({})).unwrap_err();
    assert!(e.contains("too large for one backup file") && e.contains("zip64"), "{e}");
    assert!(h.app.library_backup.is_none() && row(&h, BACKUP_LABEL).is_none());
    let docs = root.join("Documents").join("LightCraft");
    assert_eq!(std::fs::read_dir(&docs).map(|d| d.count()).unwrap_or(0), 0, "nothing written");
    assert!(shared.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// An action that panics is still there for the next call (it is out of `Services` while it runs).
#[test]
fn panicking_backup_action_is_put_back() {
    let root = temp("panic");
    let (session, _) = session(&root);
    let calls = Arc::new(Mutex::new(0usize));
    let counted = calls.clone();
    let action: crate::LibraryAction = Box::new(move |_: &mut LightcraftApp, _: &serde_json::Value| {
        let mut n = counted.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *n += 1;
        if *n == 1 {
            drop(n);
            panic!("boom");
        }
        Ok(json!({"started": true}))
    });
    let services = Services { backup_library: Some(action), ..Default::default() };
    let mut h = headless(LightcraftApp::new(session, services));
    let e = h.app.run("file.backupLibrary", json!({})).unwrap_err();
    assert!(e.contains("failed unexpectedly") && e.contains("boom"), "{e}");
    assert!(h.app.services.backup_library.is_some(), "put back after the panic");
    assert_eq!(h.app.run("file.backupLibrary", json!({})).unwrap()["started"], true);
    assert_eq!(*calls.lock().unwrap(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn first_start_notice_offers_backup() {
    let root = temp("notice");
    let (session, _) = session(&root);
    let calls = Arc::new(Mutex::new(0usize));
    let c = calls.clone();
    let services = Services {
        library_notice: Some("Your library is stored inside LightCraft.".into()),
        backup_library: Some(Box::new(move |_: &mut LightcraftApp, _: &serde_json::Value| {
            *c.lock().unwrap() += 1;
            Ok(json!({"started": true}))
        })),
        png: None,
        ..Default::default()
    };
    let mut h = headless(LightcraftApp::new(session, services));
    h.settle(Duration::from_secs(10));
    assert!(h.app.library_notice.is_some(), "told at the first start");
    let r = h.request("ui.clickWidget", json!({"id": "button:libraryNoticeBackup"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(5));
    assert_eq!(*calls.lock().unwrap(), 1, "Back Up Now runs the backup");
    assert!(h.app.library_notice.is_none() && h.app.ui.library_notice_shown, "and closes the notice as read");

    // a host without backups: only OK
    let other = temp("notice-desktop");
    let (desktop, _) = self::session(&other);
    let mut h = headless(LightcraftApp::new(desktop, Services { library_notice: Some("Told.".into()), png: None, ..Default::default() }));
    h.settle(Duration::from_secs(10));
    assert!(h.app.library_notice.is_some());
    assert!(h.app.widgets.iter().any(|(w, _)| w == "button:libraryNoticeOk"));
    assert!(!h.app.widgets.iter().any(|(w, _)| w == "button:libraryNoticeBackup"));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&other);
}

/// Back up, change the library, restore: the backup's catalog and original are back, the
/// previous library is gone once the restored one opened, and nothing is left beside it.
#[test]
fn restore_replaces_library_after_it_checked_out() {
    let root = temp("restore");
    let (session, id) = session(&root);
    let lib = root.join("library");
    let (services, _) = device(&root, Services::default());
    let mut h = headless(LightcraftApp::new(session, services));
    let path = h.app.run("file.backupLibrary", json!({})).unwrap()["path"].as_str().unwrap().to_string();
    assert!(h.step_until(T, |h| h.app.library_backup.is_none() && h.app.share.is_none()));
    h.app.session.execute("photo.rate", &json!({"rating": 1})).unwrap();

    // a damaged backup changes nothing
    let bad = root.join("bad.zip");
    std::fs::write(&bad, b"not a backup").unwrap();
    h.app.run("file.restoreLibrary", json!({"path": bad.to_string_lossy()})).unwrap();
    assert!(row(&h, RESTORE_LABEL).is_some_and(|t| t.kind == "import" && t.cancellable), "the restore's activity row");
    assert!(h.step_until(T, |h| h.app.library_backup.is_none()));
    assert!(h.app.ui.status.contains("Restore failed") && h.app.ui.status.contains("Nothing was changed"), "{}", h.app.ui.status);
    assert_eq!(h.app.session.catalog.photo(id).unwrap().rating, 1);
    assert!(!staging_dir(&lib).exists());

    h.app.run("file.restoreLibrary", json!({"path": path})).unwrap();
    assert!(h.step_until(T, |h| h.app.library_backup.is_none()));
    assert!(h.app.ui.status.contains("Library restored"), "{}", h.app.ui.status);
    assert_eq!(h.app.session.library.as_ref().map(|l| l.dir.clone()), Some(lib.clone()));
    assert_eq!(h.app.session.catalog.photo(id).unwrap().rating, 4, "the backup's edits");
    let lightcraft_engine::catalog::Source::File { path: original } = &h.app.session.catalog.photo(id).unwrap().source else { panic!() };
    assert!(Path::new(original).starts_with(lib.join("Originals")) && Path::new(original).is_file(), "{original}");
    for gone in [staging_dir(&lib), previous_dir(&lib), marker_path(&lib)] {
        assert!(!gone.exists(), "{} left behind", gone.display());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Android: the host reopens the library behind its loading frame, so the new app finishes the
/// restore; a restored library that doesn't open gives the previous one back.
#[test]
fn restore_through_host_reopen_and_rollback() {
    let root = temp("reopen");
    let (session, id) = session(&root);
    let lib = root.join("library");
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let a = asked.clone();
    let reopen = move |path: &str| -> Result<(), String> {
        a.lock().unwrap().push(path.to_string());
        Ok(())
    };
    let (services, _) = device(&root, Services { reopen_library: Some(Box::new(reopen)), ..Default::default() });
    let mut h = headless(LightcraftApp::new(session, services));
    let path = h.app.run("file.backupLibrary", json!({})).unwrap()["path"].as_str().unwrap().to_string();
    assert!(h.step_until(T, |h| h.app.library_backup.is_none() && h.app.share.is_none()));
    h.app.session.execute("photo.rate", &json!({"rating": 2})).unwrap();
    h.app.run("file.restoreLibrary", json!({"path": path})).unwrap();
    assert!(h.step_until(T, |h| h.app.library_backup.is_none()));
    assert_eq!(*asked.lock().unwrap(), [lib.to_string_lossy().into_owned()], "the host opens it");
    assert!(h.app.session.library.is_none(), "nothing writes into the moved folder");
    assert!(previous_dir(&lib).is_dir() && marker_path(&lib).is_file(), "the previous library waits until the restored one opened");
    drop(h);

    // the host's new app over the restored library finishes it
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let mut app = LightcraftApp::new(s, Services { png: None, ..Default::default() });
    let text = crate::library_backup::finish_pending(&mut app).unwrap();
    assert!(text.contains("Library restored"), "{text}");
    assert_eq!(app.session.catalog.photo(id).unwrap().rating, 4);
    assert!(!previous_dir(&lib).exists() && !marker_path(&lib).exists());
    assert!(crate::library_backup::finish_pending(&mut app).is_none(), "once");
    drop(app);

    // a restored library that fails to open: the previous one comes back
    std::fs::rename(&lib, previous_dir(&lib)).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("catalog.snap"), b"damaged").unwrap();
    let marker = crate::library_backup::Marker {
        library: lib.to_string_lossy().into_owned(),
        previous: previous_dir(&lib).to_string_lossy().into_owned(),
        from_dir: lib.to_string_lossy().into_owned(),
        ..Default::default()
    };
    std::fs::write(marker_path(&lib), serde_json::to_vec(&marker).unwrap()).unwrap();
    let mut app = LightcraftApp::new(lightcraft_engine::Session::new().with_fs(), Services { png: None, ..Default::default() });
    app.library_problem = Some(crate::panels::library_problem::LibraryProblem::new(lib.to_string_lossy(), "damaged catalog"));
    let text = crate::library_backup::finish_pending(&mut app).unwrap();
    assert!(text.contains("previous library is back"), "{text}");
    assert!(app.library_problem.is_none(), "the previous library opened");
    assert_eq!(app.session.catalog.photo(id).unwrap().rating, 4);
    assert!(!marker_path(&lib).exists());
    assert!(!crate::library_backup::failed_dir(&lib).exists(), "the failed copy is removed once the previous library is back");
    let _ = std::fs::remove_dir_all(&root);
}

/// A backup of `s`'s library (rated 4) written and extracted into `staging_dir`, as a restore's
/// worker leaves it; returns the marker a swap would write (phase `Moving`).
fn staged(s: &mut lightcraft_engine::Session, root: &Path, lib: &Path) -> crate::library_backup::Marker {
    use lightcraft_engine::backup_dir;
    let no = std::sync::atomic::AtomicBool::new(false);
    let plan = backup_dir::Plan::of(s).unwrap();
    let zip = root.join("b.zip");
    backup_dir::write(&plan, &zip, &no).unwrap();
    let v = backup_dir::verify_file(&zip, &no).unwrap();
    backup_dir::extract(&zip, &v, &staging_dir(lib), &no).unwrap();
    crate::library_backup::Marker {
        library: lib.to_string_lossy().into_owned(),
        previous: previous_dir(lib).to_string_lossy().into_owned(),
        staging: staging_dir(lib).to_string_lossy().into_owned(),
        from_dir: lib.to_string_lossy().into_owned(),
        phase: crate::library_backup::Phase::Moving,
    }
}

fn put_marker(lib: &Path, m: &crate::library_backup::Marker) {
    std::fs::write(marker_path(lib), serde_json::to_vec(m).unwrap()).unwrap();
}

/// Open the library as a host does at its start (settling an unfinished restore first) and
/// finish what is pending.
fn start(lib: &Path) -> (LightcraftApp, Option<String>) {
    let (s, problem) = crate::host::session_open::open_library_session(false, Some(lib.to_path_buf()), false);
    let mut app = LightcraftApp::new(s, Services { png: None, ..Default::default() });
    app.library_problem = problem;
    let text = crate::library_backup::finish_pending(&mut app);
    (app, text)
}

/// The process died after the library moved aside and before the restored one took its place:
/// the next start puts the library back instead of creating an empty one there.
#[test]
fn crash_between_the_moves_puts_the_library_back() {
    let root = temp("crash-mid");
    let lib = root.join("library");
    let (mut s, id) = session(&root);
    let marker = staged(&mut s, &root, &lib);
    s.execute("photo.rate", &json!({"rating": 2})).unwrap();
    s.close_library().unwrap();
    drop(s);
    put_marker(&lib, &marker);
    std::fs::rename(&lib, previous_dir(&lib)).unwrap();

    let (app, text) = start(&lib);
    let text = text.unwrap();
    assert!(text.contains("previous library is in use"), "{text}");
    assert_eq!(app.session.catalog.photo(id).unwrap().rating, 2, "the library from before the restore");
    for gone in [previous_dir(&lib), staging_dir(&lib), marker_path(&lib)] {
        assert!(!gone.exists(), "{} left behind", gone.display());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Died before anything moved, or after both moves but before the marker said so.
#[test]
fn crash_before_or_after_the_moves() {
    let root = temp("crash-ends");
    let lib = root.join("library");
    let (mut s, id) = session(&root);
    let marker = staged(&mut s, &root, &lib);
    s.execute("photo.rate", &json!({"rating": 2})).unwrap();
    s.close_library().unwrap();
    drop(s);

    // before the first move: nothing changed, the written copy goes
    put_marker(&lib, &marker);
    let (app, text) = start(&lib);
    assert!(text.unwrap().contains("previous library is in use"));
    assert_eq!(app.session.catalog.photo(id).unwrap().rating, 2);
    assert!(!staging_dir(&lib).exists() && !marker_path(&lib).exists());
    drop(app);

    // both moves done, the marker still says moving: the restore completes
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let marker = staged(&mut s, &root, &lib);
    s.close_library().unwrap();
    drop(s);
    put_marker(&lib, &marker);
    std::fs::rename(&lib, previous_dir(&lib)).unwrap();
    std::fs::rename(staging_dir(&lib), &lib).unwrap();
    let (app, text) = start(&lib);
    assert!(text.unwrap().contains("Library restored"));
    assert_eq!(app.session.catalog.photo(id).unwrap().rating, 2, "the backup's catalog");
    assert!(!previous_dir(&lib).exists() && !marker_path(&lib).exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// A rollback whose second move failed leaves the library missing and the previous one beside
/// it, with its marker: the next start moves it back. And a `.before-restore` that no marker
/// vouches for survives the next restore under another name.
#[test]
fn failed_rollback_then_second_restore_keeps_the_previous_library() {
    let root = temp("rollback-fail");
    let lib = root.join("library");
    let (mut s, id) = session(&root);
    let mut marker = staged(&mut s, &root, &lib);
    s.execute("photo.rate", &json!({"rating": 2})).unwrap();
    s.close_library().unwrap();
    drop(s);
    std::fs::remove_dir_all(staging_dir(&lib)).unwrap();
    marker.phase = crate::library_backup::Phase::Swapped;
    put_marker(&lib, &marker);
    std::fs::rename(&lib, previous_dir(&lib)).unwrap();
    std::fs::create_dir_all(crate::library_backup::failed_dir(&lib)).unwrap();
    let (app, text) = start(&lib);
    assert!(text.unwrap().contains("previous library is in use"));
    assert_eq!(app.session.catalog.photo(id).unwrap().rating, 2);
    assert!(!marker_path(&lib).exists() && !crate::library_backup::failed_dir(&lib).exists());
    drop(app);

    // a stray .before-restore (no marker), then a restore
    let stray = previous_dir(&lib);
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("catalog.snap"), b"the only copy").unwrap();
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let (services, _) = device(&root, Services::default());
    let mut h = headless(LightcraftApp::new(s, services));
    let path = h.app.run("file.backupLibrary", json!({})).unwrap()["path"].as_str().unwrap().to_string();
    assert!(h.step_until(T, |h| h.app.library_backup.is_none() && h.app.share.is_none()));
    h.app.run("file.restoreLibrary", json!({"path": path})).unwrap();
    assert!(h.step_until(T, |h| h.app.library_backup.is_none()));
    assert!(h.app.ui.status.contains("Library restored") && h.app.ui.status.contains("was kept in"), "{}", h.app.ui.status);
    let kept: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("library.before-restore-"))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(std::fs::read(kept[0].join("catalog.snap")).unwrap(), b"the only copy");
    let _ = std::fs::remove_dir_all(&root);
}
