//! File ▸ Share… (APK-27) with a fake host share sheet: the command exists only with the service,
//! exports into the host's folder for images and hands exactly the written files to the service
//! on a worker under an activity row; original files with their sidecars are refused in a folder
//! for images alone, with why.

use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::share::{LABEL, ShareFn};
use crate::{ExportKind, LightcraftApp, Services};

const T: Duration = Duration::from_secs(60);
const PICTURES: &str = "/lc-share/Pictures/LightCraft";
const DOCUMENTS: &str = "/lc-share/Documents/LightCraft";

fn demo(services: Services) -> Headless {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    let r = h.request("ui.set", json!({"view": "photoGrid"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(120));
    h
}

/// Android's folders: images to Pictures/LightCraft, the rest to Documents/LightCraft, and
/// Pictures takes images alone.
fn android_folders(services: Services) -> Services {
    Services {
        export_dir_for: Some(Box::new(|kind| match kind {
            ExportKind::Image => PICTURES.into(),
            ExportKind::Document => DOCUMENTS.into(),
        })),
        media_only_dirs: vec!["/lc-share/Pictures".into(), "/lc-share/DCIM".into()],
        ..services
    }
}

/// A writer that keeps the paths it was asked to write (nothing touches the disk).
fn recording_writer(services: Services) -> (Services, Arc<Mutex<Vec<String>>>) {
    let written = Arc::new(Mutex::new(Vec::<String>::new()));
    let (a, b) = (written.clone(), written.clone());
    let services = Services {
        write: Some(Box::new(move |p: &str, _: &[u8]| {
            a.lock().unwrap().push(p.to_string());
            Ok(())
        })),
        write_shared: Some(Arc::new(move |p: &str, _: &[u8]| {
            b.lock().unwrap().push(p.to_string());
            Ok(())
        })),
        ..services
    };
    (services, written)
}

fn listed(h: &Headless) -> bool {
    let bar = crate::menubar::menu_bar(&h.app);
    let file = &bar.iter().find(|(t, _)| t == "File").unwrap().1;
    file.iter().any(|n| matches!(n, crate::menubar::MenuNode::Item { id, .. } if id == "file.share"))
}

fn row(h: &Headless) -> bool {
    h.app.session.activity.list().iter().any(|t| t.label == LABEL && t.kind == "export")
}

/// Without the host's share sheet (the desktop, the web) the command is disabled, refused and not
/// in the File menu.
#[test]
fn share_disabled_without_service() {
    let mut h = demo(Services { png: None, ..Default::default() });
    assert!(!crate::menus::ui_enabled(&h.app, "file.share"));
    assert_eq!(h.app.run("file.share", json!({})).unwrap_err(), crate::share::UNAVAILABLE);
    assert!(h.app.share.is_none() && h.app.export.is_none() && !h.app.share_after_export);
    assert!(!listed(&h));
    let commands = h.request("engine.commands", json!({}), T);
    let c = commands["result"].as_array().and_then(|v| v.iter().find(|c| c["id"] == "file.share")).cloned().unwrap();
    assert_eq!(c["enabled"], false, "{c}");
}

/// Share exports the selection (with the previous export's settings) into the host's folder for
/// images in the background, then hands the service exactly the files written; Export with
/// Previous keeps the export before. `{paths}` shares those files as they are.
#[test]
fn share_calls_service_with_exported_paths() {
    let shared = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let calls = shared.clone();
    let share: ShareFn = Arc::new(move |paths: &[String], _| {
        calls.lock().unwrap().push(paths.to_vec());
        Ok(())
    });
    let (services, written) = recording_writer(android_folders(Services { png: None, share: Some(share), ..Default::default() }));
    let mut h = demo(services);
    assert!(crate::menus::ui_enabled(&h.app, "file.share"));
    assert!(listed(&h), "in the File menu");
    let previous = json!({"format": "png", "width": 40, "dir": "/lc-elsewhere"});
    h.app.session.last_export = Some(previous.clone());
    let ids: Vec<Value> = h.app.session.visible().iter().take(2).map(|id| json!(id)).collect();
    let r = h.request("engine.execute", json!({"command": "file.share", "params": {"ids": ids}}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(T, |_| !shared.lock().unwrap().is_empty()), "the share sheet was asked");
    assert!(h.step_until(T, |h| h.app.share.is_none()));
    let calls = shared.lock().unwrap().clone();
    let written = written.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], written, "exactly the exported files");
    assert_eq!(written.len(), 2, "{written:?}");
    assert!(written.iter().all(|p| p.starts_with(&format!("{PICTURES}/")) && p.ends_with(".png")), "{written:?}");
    assert_eq!(h.app.session.last_export, Some(previous), "a share is not the previous export");
    assert!(!row(&h), "the row is gone with the share");
    // files named: shared as they are
    let r = h.app.run("file.share", json!({"paths": [format!("{DOCUMENTS}/Contact Sheet.pdf")]})).unwrap();
    assert_eq!(r["sharing"], 1);
    assert!(h.step_until(T, |h| h.app.share.is_none()));
    assert_eq!(shared.lock().unwrap()[1], [format!("{DOCUMENTS}/Contact Sheet.pdf")]);
    assert!(h.app.run("file.share", json!({"paths": "a.jpg"})).is_err());
    assert!(h.app.run("file.share", json!({"paths": []})).is_err(), "nothing to share");
}

/// The lookup runs off the UI thread under its cancellable activity row: the frames go on while
/// the host works, one share at a time, and a failure shows as the error.
#[test]
fn share_lookup_shows_activity_row() {
    let barrier = Arc::new(Barrier::new(2));
    let wait = barrier.clone();
    let share: ShareFn = Arc::new(move |_: &[String], _| {
        wait.wait();
        Err("Could not share: no app can receive these files".to_owned())
    });
    let mut h = demo(android_folders(Services { png: None, share: Some(share), ..Default::default() }));
    assert_eq!(h.app.run("file.share", json!({"paths": [format!("{PICTURES}/a.jpg")]})).unwrap()["sharing"], 1);
    for _ in 0..3 {
        h.step();
        assert!(row(&h), "the row shows while the host is still answering");
    }
    let task = h.app.session.activity.list().into_iter().find(|t| t.label == LABEL).unwrap();
    assert!(task.cancellable, "the lookup can be stopped");
    assert!(!crate::menus::ui_enabled(&h.app, "file.share"), "one share at a time");
    assert!(h.app.run("file.share", json!({"paths": ["/x.jpg"]})).is_err());
    barrier.wait();
    assert!(h.step_until(T, |h| h.app.share.is_none()));
    assert!(!row(&h), "the row is gone with the answer");
    assert!(h.app.ui.status.contains("no app can receive"), "{}", h.app.ui.status);
}

/// Original files bring XMP sidecars, which a folder for images alone (Android's Pictures) can't
/// take: the export is refused before anything is written, saying where to put them instead, and
/// the Export dialog doesn't offer them there.
#[test]
fn sidecar_into_pictures_is_refused_with_reason() {
    let (services, written) = recording_writer(android_folders(Services { png: None, ..Default::default() }));
    let mut h = demo(services);
    let e = h.app.run("app.export", json!({"format": "original", "dir": PICTURES})).unwrap_err();
    assert!(e.contains("can't be exported into Pictures") && e.contains("XMP sidecars"), "{e}");
    assert!(e.contains(DOCUMENTS), "says where they can go: {e}");
    let e = h.app.run("app.export", json!({"format": "original", "dir": "/lc-share/DCIM/Camera"})).unwrap_err();
    assert!(e.contains("into DCIM"), "{e}");
    assert!(written.lock().unwrap().is_empty(), "nothing written");
    assert!(crate::control::sidecar_refusal(&h.app.services, DOCUMENTS).is_none(), "Documents takes them");
    // the dialog in Pictures: no Original files choice, and why
    h.app.session.last_export = Some(json!({"format": "jpeg", "dir": PICTURES}));
    assert!(h.app.run("dialog.export", json!({})).is_ok());
    h.settle(Duration::from_secs(5));
    let has = |h: &Headless, id: &str| h.app.widgets.iter().any(|(w, _)| w == id);
    assert!(has(&h, "button:exportFormat-0") && has(&h, "label:exportNoSidecars"), "the note shows");
    assert!(!has(&h, "button:exportFormat-8"), "no Original files in Pictures");
    // the desktop: no such folders, nothing refused
    assert!(crate::control::sidecar_refusal(&Services::default(), PICTURES).is_none());
}

/// "Same folder as original" with Original files writes the sidecar next to each original: a
/// photo added in place from DCIM is refused with why, before anything is written.
#[test]
fn sidecar_next_to_original_in_dcim_is_refused() {
    let (services, written) = recording_writer(android_folders(Services { png: None, ..Default::default() }));
    let mut h = demo(services);
    let id = h.app.session.visible()[0];
    let name = h.app.session.catalog.photo(id).unwrap().file_name.clone();
    let relink = lightcraft_engine::catalog::Op::Relink {
        id,
        file_name: name,
        source: lightcraft_engine::catalog::Source::File { path: "/lc-share/DCIM/Camera/IMG_0001.jpg".into() },
        format: None,
    };
    h.app.session.catalog.apply(relink).unwrap();
    let e = h.app.run("app.export", json!({"format": "original", "destinationMode": "sameAsOriginal", "ids": [id]})).unwrap_err();
    assert!(e.contains("into DCIM") && e.contains(DOCUMENTS), "{e}");
    assert!(written.lock().unwrap().is_empty(), "nothing written");
}
