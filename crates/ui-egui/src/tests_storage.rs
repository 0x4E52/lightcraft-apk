//! Settings ▸ Storage: the parts of what the app keeps, measured on a worker, with Clear on the caches.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn put(path: &std::path::Path, len: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![1u8; len]).unwrap();
}

#[test]
fn storage_section_lists_parts() {
    let files = std::env::temp_dir().join(format!("lc-ui-storage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&files);
    let lib = files.join("library");
    let mut session = lightcraft_engine::Session::new().with_fs();
    session.open_library(lib.clone(), true).unwrap();
    put(&lib.join("Originals/a.jpg"), 3000);
    put(&lib.join("Smart Previews/a.lcsp"), 2000);
    put(&files.join("inbox/shared.jpg"), 1000);
    let inbox_file = files.join("inbox/shared.jpg");
    let services = Services { storage_roots: vec![files.clone()], storage_inbox: Some(files.join("inbox")), ..Default::default() };
    let mut h = Headless::new(LightcraftApp::new(session, services), [1000.0, 900.0], 1.0);
    h.app.ui.dialog = Some(Dialog::Settings { tab: "storage".into() });
    let has = |h: &Headless, id: &str| h.app.widgets.iter().any(|(w, _)| w == id);
    // measuring holds a stoppable row of the activity stack for as long as it runs
    {
        let _gate = crate::storage::MEASURE_GATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // (the dialog opens over a frame or two)
        for _ in 0..5 {
            h.step();
        }
        let rows: Vec<_> = h.app.session.activity.list().into_iter().filter(|t| t.kind == "storage").collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].label == "Measuring storage" && rows[0].cancellable, "{rows:?}");
        // stopping it leaves the page without a (wrong) report; it measures again
        h.app.session.activity.cancel(rows[0].id).unwrap();
    }
    // the worker reports; the rows appear
    let mut seen = false;
    for _ in 0..400 {
        h.step();
        if has(&h, "button:storageClear-smartPreviews") {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen, "the rows list after measuring");
    for part in ["thumbnails", "smartPreviews", "denoiseCache", "models", "logs", "inbox"] {
        assert!(has(&h, &format!("button:storageClear-{part}")), "{part} can be cleared");
    }
    for part in ["library", "originals", "places", "temp"] {
        assert!(!has(&h, &format!("button:storageClear-{part}")), "{part} is never cleared from here");
    }
    // the command says the same as the page
    let report =
        h.app.run("library.storage", json!({"roots": [files.display().to_string()], "inbox": files.join("inbox").display().to_string()})).unwrap();
    let bytes = |id: &str| report["parts"].as_array().unwrap().iter().find(|p| p["part"] == id).unwrap()["bytes"].as_u64().unwrap();
    assert_eq!((bytes("originals"), bytes("smartPreviews"), bytes("inbox")), (3000, 2000, 1000));
    assert!(report["total"].as_u64().unwrap() >= 6000);
    // Clear on smart previews asks first: one tap deletes nothing
    // (the rows may still be moving while the first report settles: tap again until the question shows)
    for _ in 0..50 {
        assert_eq!(h.request("ui.clickWidget", json!({"id": "button:storageClear-smartPreviews"}), T)["ok"], true);
        for _ in 0..5 {
            h.step();
        }
        if has(&h, "button:storageConfirm-smartPreviews") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(lib.join("Smart Previews/a.lcsp").exists(), "one tap does not delete");
    assert!(has(&h, "button:storageConfirm-smartPreviews") && has(&h, "button:storageCancel"));
    // Cancel puts the Clear button back
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:storageCancel"}), T)["ok"], true);
    h.step();
    h.step();
    assert!(has(&h, "button:storageClear-smartPreviews") && lib.join("Smart Previews/a.lcsp").exists());
    // Confirming empties the part on a worker and leaves the originals
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:storageClear-smartPreviews"}), T)["ok"], true);
    h.step();
    h.step();
    let gate = crate::storage::CLEAR_GATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:storageConfirm-smartPreviews"}), T)["ok"], true);
    h.step();
    let rows: Vec<_> = h.app.session.activity.list().into_iter().filter(|t| t.kind == "storage").collect();
    assert!(rows.len() == 1 && rows[0].label == "Clearing storage" && !rows[0].cancellable, "{rows:?}");
    assert!(lib.join("Smart Previews/a.lcsp").exists(), "the worker has not run yet");
    drop(gate);
    let mut gone = false;
    for _ in 0..400 {
        h.step();
        if !lib.join("Smart Previews/a.lcsp").exists() {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(gone, "confirmed clear removes the files");
    assert!(lib.join("Originals/a.jpg").exists());
    assert!(inbox_file.exists(), "other parts are untouched");
    assert!(h.step_until(T, |h| h.app.session.activity.list().is_empty()), "the row goes with the job");
    let _ = std::fs::remove_dir_all(&files);
}
