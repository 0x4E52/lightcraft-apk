//! The raw types a phone hides from apps (APK-52, `Services::hidden_types`): the import review and
//! the Library sidebar's Local section name them and say how to bring them in anyway; a host that
//! hides nothing, and the desktop, show no note.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

/// A folder with one PNG, to import.
fn folder(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lc-hidden-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = lightcraft_raster::Rgba8 { width: 8, height: 8, data: vec![[90, 30, 9, 255]; 64] };
    let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(dir.join("IMG_1.png"), png).unwrap();
    dir
}

/// The app over the demo library with Local listing `dir`, the sidebar open.
fn app(services: Services, dir: &std::path::Path) -> Headless {
    let services = Services { png: None, local_roots: vec![("Camera".into(), dir.to_path_buf())], ..services };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo().with_fs(), services);
    app.ui.settings.gpu = false;
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    let mut h = Headless::new(app, [1300.0, 1000.0], 1.0);
    h.request("ui.set", json!({"leftPanel": true}), T);
    let row = format!("source:local:{}", dir.to_string_lossy());
    assert!(h.step_until(T, |h| h.app.widgets.iter().any(|(w, _)| *w == row)), "Local lists the folder");
    h
}

fn shown(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

/// Open the import review on `dir`.
fn review(h: &mut Headless, dir: &std::path::Path) {
    let r = h.request("engine.execute", json!({"command": "file.addFolder", "params": {"path": dir.to_string_lossy()}}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.step_until(T, |h| matches!(h.app.ui.dialog, Some(Dialog::Import { .. }))), "the import review opens");
    h.settle(Duration::from_secs(5));
}

/// A phone that hides CR3 and 3FR: Local and the import review both say so, naming the types and
/// the way in (a share, "Open with").
#[test]
fn import_review_names_hidden_types() {
    let dir = folder("named");
    let mut h = app(Services { hidden_types: vec!["CR3".into(), "3FR".into()], prefer_copy_import: true, ..Default::default() }, &dir);
    h.settle(Duration::from_secs(5));
    assert!(shown(&h, "label:localHiddenTypes"), "Local names the hidden types");
    review(&mut h, &dir);
    assert!(shown(&h, "label:importHiddenTypes"), "the import review names them");
    let note = crate::import::hidden_types_note(&h.app.services.hidden_types).unwrap();
    assert!(note.contains("CR3, 3FR"), "{note}");
    assert!(note.contains("Share them to LightCraft"), "{note}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A phone whose media database knows every raw type: no note anywhere.
#[test]
fn no_note_when_nothing_is_hidden() {
    let dir = folder("none");
    let mut h = app(Services { hidden_types: Vec::new(), prefer_copy_import: true, ..Default::default() }, &dir);
    h.settle(Duration::from_secs(5));
    assert!(!shown(&h, "label:localHiddenTypes"));
    review(&mut h, &dir);
    assert!(!shown(&h, "label:importHiddenTypes"));
    assert_eq!(crate::import::hidden_types_note(&[]), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The desktop's services hide nothing: the note never appears there.
#[test]
fn desktop_never_shows_hidden_note() {
    assert!(Services::default().hidden_types.is_empty());
    let dir = folder("desktop");
    let mut h = app(Services::default(), &dir);
    h.settle(Duration::from_secs(5));
    assert!(!shown(&h, "label:localHiddenTypes"));
    review(&mut h, &dir);
    assert!(!shown(&h, "label:importHiddenTypes"));
    let _ = std::fs::remove_dir_all(&dir);
}
