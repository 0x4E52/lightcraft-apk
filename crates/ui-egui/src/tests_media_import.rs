//! File ▸ Import from Photo Library… (APK-23) with a fake host query: the command exists only with
//! the service, reads the library on a worker under an activity row, and feeds the chosen folder
//! to the import review (its own photos, not its subfolders'); a host whose originals are read-only starts the
//! review on Copy.

use std::sync::{Arc, Barrier};
use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::media_import::{ImportMedia, LABEL, MediaBucket};
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn demo(services: Services) -> Headless {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    let r = h.request("ui.set", json!({"view": "photoGrid"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(120));
    h
}

/// A folder with two PNGs, as a photo library folder.
fn folder(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lc-media-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = lightcraft_raster::Rgba8::from_fn(24, 16, |x, y| [(x * 9) as u8, (y * 12) as u8, 80, 255]);
    let png = lightcraft_engine::export::encode_image(
        &img,
        &lightcraft_engine::export::ExportOptions { format: lightcraft_engine::export::ExportFormat::Png, ..Default::default() },
    )
    .unwrap();
    std::fs::write(dir.join("IMG_1.png"), &png).unwrap();
    std::fs::write(dir.join("IMG_2.png"), &png).unwrap();
    dir
}

fn bucket(name: &str, dir: &std::path::Path) -> MediaBucket {
    MediaBucket { name: name.into(), path: dir.to_string_lossy().to_string(), count: 2, bytes: 100 }
}

fn listed(h: &Headless) -> bool {
    let bar = crate::menubar::menu_bar(&h.app);
    let file = &bar.iter().find(|(t, _)| t == "File").unwrap().1;
    file.iter().any(|n| matches!(n, crate::menubar::MenuNode::Item { id, .. } if id == "file.importMediaLibrary"))
}

/// Without the host's query (the desktop, the web) the command is disabled, refused and not in
/// the File menu.
#[test]
fn import_media_disabled_without_service() {
    let mut h = demo(Services { png: None, ..Default::default() });
    assert!(!crate::menus::ui_enabled(&h.app, "file.importMediaLibrary"));
    assert!(h.app.run("file.importMediaLibrary", json!({})).is_err());
    assert!(h.app.media_query.is_none() && h.app.ui.dialog.is_none());
    assert!(!listed(&h));
    let commands = h.request("engine.commands", json!({}), T);
    let c = commands["result"].as_array().and_then(|v| v.iter().find(|c| c["id"] == "file.importMediaLibrary")).cloned().unwrap();
    assert_eq!(c["enabled"], false, "{c}");
}

/// The library's folders open the import review as its source; choosing one scans it as a
/// picked folder (`file.addFolder`), and the list stays to choose another.
#[test]
fn import_media_feeds_import_dialog() {
    let (a, b) = (folder("a"), folder("b"));
    let buckets = vec![bucket("Camera", &a), bucket("Screenshots", &b)];
    let answer = buckets.clone();
    let query: ImportMedia = Arc::new(move || Ok(answer.clone()));
    let mut h = demo(Services { png: None, import_media: Some(query), ..Default::default() });
    assert!(crate::menus::ui_enabled(&h.app, "file.importMediaLibrary"));
    assert!(listed(&h), "in the File menu");
    let r = h.request("engine.execute", json!({"command": "file.importMediaLibrary"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    assert_eq!(opts.buckets, buckets);
    assert!(opts.candidates.is_empty() && opts.sources.is_empty(), "nothing scanned before a folder is chosen");
    h.settle(Duration::from_secs(5));
    assert!(h.app.widgets.iter().any(|(w, _)| w == "button:importBucket:1"), "a button per folder");
    // choosing a folder (its button) scans it into the review
    let r = h.request("ui.clickWidget", json!({"id": "button:importBucket:1"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let b_path = b.to_string_lossy().to_string();
    assert!(h.step_until(T, |h| matches!(&h.app.ui.dialog, Some(Dialog::Import { opts }) if opts.sources == [b_path.clone()])));
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    assert_eq!(opts.candidates.len(), 2, "the folder's photos");
    assert_eq!(opts.buckets, buckets, "the list stays");
    assert!(!opts.copy, "without the host's preference, a folder is added in place as from Import from Folder");
    // agents name the folder
    let a_path = a.to_string_lossy().to_string();
    let r = h.request("engine.execute", json!({"command": "file.importMediaLibrary", "params": {"bucket": a_path}}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(T, |h| matches!(&h.app.ui.dialog, Some(Dialog::Import { opts }) if opts.sources == [a_path.clone()])));
    assert!(h.app.run("file.importMediaLibrary", json!({"bucket": 3})).is_err());
    // importing the shown folder adds its photos
    let before = h.app.session.catalog.len();
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(Duration::from_secs(60), |h| h.app.import.is_none() && h.app.session.catalog.len() == before + 2));
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// The query runs off the UI thread under its activity row: the frames go on while the host
/// waits, the row is there all along and gone with the answer; a failure shows as the error.
#[test]
fn media_query_shows_activity_row() {
    let barrier = Arc::new(Barrier::new(2));
    let wait = barrier.clone();
    let query: ImportMedia = Arc::new(move || {
        wait.wait();
        Err("LightCraft needs access to your photos".to_owned())
    });
    let mut h = demo(Services { png: None, import_media: Some(query), ..Default::default() });
    assert_eq!(h.app.run("file.importMediaLibrary", json!({})).unwrap()["reading"], true);
    let row = |h: &Headless| h.app.session.activity.list().iter().any(|t| t.label == LABEL && t.kind == "import");
    for _ in 0..3 {
        h.step();
        assert!(row(&h), "the row shows while the host is still answering");
    }
    assert!(h.app.media_query.is_some());
    assert!(!crate::menus::ui_enabled(&h.app, "file.importMediaLibrary"), "one query at a time");
    assert!(h.app.run("file.importMediaLibrary", json!({})).is_err());
    barrier.wait();
    assert!(h.step_until(T, |h| h.app.media_query.is_none()));
    assert!(!row(&h), "the row is gone with the answer");
    assert!(h.app.ui.status.contains("needs access to your photos"), "{}", h.app.ui.status);
    assert!(h.app.ui.dialog.is_none());
}

/// A host whose originals are read-only (Android) starts every import review on Copy, and Add in
/// place says why it is not the default.
#[test]
fn prefer_copy_import_defaults_to_copy() {
    let dir = folder("copy");
    let buckets = vec![bucket("Camera", &dir)];
    let answer = buckets.clone();
    let query: ImportMedia = Arc::new(move || Ok(answer.clone()));
    let mut h = demo(Services { png: None, import_media: Some(query), prefer_copy_import: true, ..Default::default() });
    h.app.run("file.importMediaLibrary", json!({})).unwrap();
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    assert!(opts.copy && !opts.move_files, "the folder list starts on Copy");
    let path = dir.to_string_lossy().to_string();
    h.app.run("file.importMediaLibrary", json!({"bucket": path})).unwrap();
    assert!(h.step_until(T, |h| matches!(&h.app.ui.dialog, Some(Dialog::Import { opts }) if !opts.candidates.is_empty())));
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    assert!(opts.copy, "the scanned folder too");
    // a folder imported the usual way defaults to Copy as well
    h.app.ui.dialog = None;
    h.app.run("file.addFolder", json!({"path": dir.to_string_lossy()})).unwrap();
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    assert!(opts.copy);
    // Add stays available, with the note
    h.settle(Duration::from_secs(5));
    assert!(!h.app.widgets.iter().any(|(w, _)| w == "label:importAddNote"));
    let r = h.request("ui.clickWidget", json!({"id": "button:importAdd"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(5));
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    assert!(!opts.copy, "Add in place can still be chosen");
    assert!(h.app.widgets.iter().any(|(w, _)| w == "label:importAddNote"), "with the note");
    // and a failed write next to such an original says to import with Copy
    let e = crate::media_import::read_only_original_error("/x/IMG_1.xmp: Permission denied (os error 13)").unwrap();
    assert!(e.contains("Copy"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A photo library folder is one directory: choosing it scans only its own photos, not those of a
/// subfolder, which is a folder of its own in the list (Pictures and Pictures/Screenshots).
#[test]
fn bucket_scans_only_its_own_photos() {
    let a = folder("nested");
    let b = a.join("Screenshots");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::copy(a.join("IMG_1.png"), b.join("SHOT_1.png")).unwrap();
    std::fs::copy(a.join("IMG_1.png"), b.join("SHOT_2.png")).unwrap();
    std::fs::copy(a.join("IMG_1.png"), b.join("SHOT_3.png")).unwrap();
    let buckets = vec![bucket("Pictures", &a), MediaBucket { count: 3, ..bucket("Screenshots", &b) }];
    let answer = buckets.clone();
    let query: ImportMedia = Arc::new(move || Ok(answer.clone()));
    let mut h = demo(Services { png: None, import_media: Some(query), ..Default::default() });
    h.app.run("file.importMediaLibrary", json!({})).unwrap();
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))));
    let a_path = a.to_string_lossy().to_string();
    h.app.run("file.importMediaLibrary", json!({"bucket": a_path})).unwrap();
    assert!(
        h.step_until(T, |h| h.app.scan.is_none() && matches!(&h.app.ui.dialog, Some(Dialog::Import { opts }) if opts.sources == [a_path.clone()]))
    );
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    let mut names: Vec<&str> = opts.candidates.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["IMG_1.png", "IMG_2.png"], "the folder's own photos, as its count says");
    assert_eq!(opts.candidates.len(), buckets[0].count);
    // the subfolder, chosen, lists its own
    let b_path = b.to_string_lossy().to_string();
    h.app.run("file.importMediaLibrary", json!({"bucket": b_path})).unwrap();
    assert!(
        h.step_until(T, |h| h.app.scan.is_none() && matches!(&h.app.ui.dialog, Some(Dialog::Import { opts }) if opts.sources == [b_path.clone()]))
    );
    let Some(Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("no import review") };
    assert_eq!(opts.candidates.len(), 3);
    assert!(opts.candidates.iter().all(|c| c.name.starts_with("SHOT_")));
    let _ = std::fs::remove_dir_all(&a);
}

/// Where originals are read-only (`prefer_copy_import`), a command on an original that the system
/// refuses says to import with Copy, through the app's command path. The refusal is real: sysfs
/// takes no new files, even from root (which ignores read-only modes elsewhere).
#[test]
fn refused_write_on_original_says_import_with_copy() {
    use lightcraft_catalog::{Op, Photo, Source};
    let dir = std::path::Path::new("/sys");
    let refused =
        dir.is_dir() && std::fs::write(dir.join("lc-media-probe.xmp"), b"x").is_err_and(|e| e.raw_os_error().is_some_and(|c| c == 13 || c == 30));
    if !refused {
        lightcraft_testkit::skip!(System, "no folder here refuses new files to this user (/sys on Linux)");
        return;
    }
    let save = |prefer_copy_import: bool| {
        let mut session = lightcraft_engine::Session::new().with_fs();
        let id = session.catalog.alloc_photo_id();
        let original = dir.join("IMG_1.png").to_string_lossy().to_string();
        let p = Photo::new(id, Source::File { path: original }, "IMG_1.png", "PNG", 8, 8, "2026-01-01T10:00:00");
        session.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
        let mut app = LightcraftApp::new(session, Services { png: None, prefer_copy_import, ..Default::default() });
        app.run("library.select", json!({"ids": [id.0]})).unwrap();
        app.run("photo.saveMetadataToFile", json!({})).unwrap()
    };
    let told = save(true);
    let error = told["failed"][0]["error"].as_str().unwrap_or_default().to_owned();
    assert!(error.contains("IMG_1.xmp"), "the file stays named: {told}");
    assert!(error.contains("Import the photo with Copy"), "{told}");
    // a host whose originals are writable keeps the system's text
    let plain = save(false);
    assert!(!plain["failed"][0]["error"].as_str().unwrap_or_default().contains("Copy"), "{plain}");
}
