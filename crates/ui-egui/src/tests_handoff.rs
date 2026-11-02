//! File ▸ Hand Off Photos… and File ▸ Receive Handoff… (APK-62; the format and the rules are
//! `lightcraft_engine::handoff`'s).
//!
//! * On a phone (the host has a folder for originals and a share sheet) Hand Off writes a new
//!   handoff folder in `Documents/LightCraft` under an activity row, then offers exactly its files
//!   to share.
//! * Receive Handoff reads the folder in the background and lists what it holds; newer settings
//!   are read only for the photos ticked, as one undo step.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{ExportKind, LightcraftApp, Services};

const T: Duration = Duration::from_secs(60);

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("lc-ui-handoff-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_png(path: &str, seed: u8) {
    let (w, h) = (24usize, 16usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 9) as u8, (i / w * 13) as u8, seed, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// An app on a library holding `folder`'s photos, its clock at `now`.
fn app(folder: &str, now: &'static str, services: Services) -> Headless {
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.clock = Box::new(move || now.to_string());
    s.execute("library.import", &json!({"paths": [folder]})).unwrap();
    let mut app = LightcraftApp::new(s, services);
    app.ui.settings.gpu = false;
    Headless::new(app, [1400.0, 900.0], 1.0)
}

fn wait_tasks(h: &mut Headless) {
    assert!(h.step_until(T, |h| h.app.tasks.is_empty()), "the background work finishes");
}

#[test]
fn handoff_writes_to_documents_and_offers_to_share() {
    let dir = Scratch::new("send");
    write_png(&dir.path("phone/a.png"), 1);
    let docs = dir.path("Documents/LightCraft");
    let shared = Arc::new(Mutex::new(Vec::<String>::new()));
    let got = shared.clone();
    let d = docs.clone();
    let services = Services {
        export_dir_for: Some(Box::new(move |_: ExportKind| d.clone())),
        share: Some(Arc::new(move |paths: &[String], _: &std::sync::atomic::AtomicBool| {
            got.lock().unwrap().extend(paths.iter().cloned());
            Ok(())
        })),
        ..Default::default()
    };
    let mut h = app(&dir.path("phone"), "2026-10-01T10:00:00", services);
    // nothing selected: nothing to hand off
    h.app.session.selection = Default::default();
    assert!(!crate::menus::ui_enabled(&h.app, "file.handoff"));
    let id = h.app.session.catalog.photos().next().unwrap().id;
    h.app.session.selection.ids = vec![id];
    assert!(crate::menus::ui_enabled(&h.app, "file.handoff"));
    let r = h.app.run("file.handoff", json!({})).unwrap();
    let out = r["dir"].as_str().unwrap().to_string();
    assert!(out.starts_with(&docs) && out.contains("LightCraft Handoff 2026-10-01 10-00-00"), "{r}");
    assert!(
        h.app.session.activity.list().iter().any(|t| t.label == crate::handoff::SEND_LABEL && t.kind == "export"),
        "the hand-off holds an activity row"
    );
    wait_tasks(&mut h);
    let manifest = std::path::Path::new(&out).join(lightcraft_engine::handoff::MANIFEST);
    assert!(manifest.is_file(), "{:?}", h.app.ui.toast);
    assert!(h.step_until(T, |h| h.app.share.is_none() && !shared.lock().unwrap().is_empty()), "the files are offered to share");
    let mut files = shared.lock().unwrap().clone();
    files.sort();
    let mut want = [manifest.to_string_lossy().into_owned(), format!("{out}/a.png"), format!("{out}/a.xmp")];
    want.sort();
    let files: Vec<String> = files.iter().map(|f| f.replace('\\', "/")).collect();
    let want: Vec<String> = want.iter().map(|f| f.replace('\\', "/")).collect();
    assert_eq!(files, want);
    let last = h.request("ui.inspect", json!({}), T);
    assert_eq!(last["result"]["handoff"]["last"]["photos"], 1, "{}", last["result"]["handoff"]);
}

#[test]
fn receive_lists_and_reads_only_ticked_photos() {
    let dir = Scratch::new("receive");
    write_png(&dir.path("phone/a.png"), 1);
    write_png(&dir.path("phone/b.png"), 2);
    // the computer had both since 09:00; the phone edited both at 10:00 and handed them off
    std::fs::create_dir_all(dir.path("desk")).unwrap();
    for n in ["a.png", "b.png"] {
        std::fs::copy(dir.path(&format!("phone/{n}")), dir.path(&format!("desk/{n}"))).unwrap();
    }
    let mut phone = lightcraft_engine::Session::new().with_fs();
    phone.clock = Box::new(|| "2026-10-01T10:00:00".to_string());
    phone.execute("library.import", &json!({"paths": [dir.path("phone")]})).unwrap();
    let ids: Vec<u64> = phone.catalog.photos().map(|p| p.id.0).collect();
    phone.execute("develop.set", &json!({"ids": ids, "control": "light.exposure", "value": 1.0})).unwrap();
    phone.execute("export.handoff", &json!({"dir": dir.path("h"), "ids": ids})).unwrap();

    let mut h = app(&dir.path("desk"), "2026-10-01T09:00:00", Services::default());
    let r = h.app.run("file.receiveHandoff", json!({"dir": dir.path("h")})).unwrap();
    assert!(r.get("reading").is_some(), "{r}");
    wait_tasks(&mut h);
    let Some(Dialog::ReceiveHandoff { newer, read, new, .. }) = h.app.ui.dialog.clone() else { panic!("no dialog: {:?}", h.app.ui.dialog) };
    assert_eq!((new, newer.len()), (0, 2), "{newer:?}");
    assert!(read.is_empty(), "nothing is read by default");
    // tick one
    let a = newer.iter().find(|r| r.name == "a.png").unwrap().id;
    let mut dlg = h.app.ui.dialog.take().unwrap();
    if let Dialog::ReceiveHandoff { read, .. } = &mut dlg {
        read.push(a);
    }
    let undo = h.app.session.undo.len();
    let r: Value = crate::panels::dialogs::confirm_dialog(&mut h.app, &dlg).unwrap();
    assert_eq!(r["read"], 1, "{r}");
    let exp = |h: &Headless, name: &str| h.app.session.catalog.photos().find(|p| p.file_name == name).unwrap().develop.light.exposure;
    assert_eq!(exp(&h, "a.png"), 1.0);
    assert_eq!(exp(&h, "b.png"), 0.0, "not ticked: not read");
    assert_eq!(h.app.session.undo.len(), undo + 1);
    // the plan went with the dialog: confirming again is refused, never a second read
    assert!(crate::panels::dialogs::confirm_dialog(&mut h.app, &dlg).is_err());
    // a folder that isn't a handoff: said, no dialog
    h.app.run("file.receiveHandoff", json!({"dir": dir.path("desk")})).unwrap();
    wait_tasks(&mut h);
    assert!(h.app.ui.dialog.is_none());
    assert!(h.app.ui.status.contains("not a handoff folder"), "{}", h.app.ui.status);
}
