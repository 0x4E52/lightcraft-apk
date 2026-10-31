//! Headless test of the filmstrip's file-name labels (#266): a name with multi-byte (CJK)
//! characters is drawn without panicking.

use std::time::Duration;

use lightcraft_catalog::Op;
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

#[test]
fn filmstrip_shows_a_cjk_file_name() {
    let mut session = lightcraft_engine::Session::with_demo();
    let first = session.visible_cloned()[0];
    let source = session.catalog.photo(first).expect("demo photo").source.clone();
    // the name from #266: cutting it at byte 13 lands inside '限'
    session.catalog.apply(Op::SetFile { id: first, file_name: "202407層三限定訂閱圖(4).jpg".into(), source }).unwrap();
    let app = LightcraftApp::new(session, Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(h.app.ui.settings.film_names);
    let film = format!("film:{}", first.0);
    assert!(h.app.widgets.iter().any(|(w, _)| *w == film), "{film} drawn in the filmstrip");
}

fn detail_at(size: [f32; 2], touch: bool) -> Headless {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, size, 1.0);
    h.app.ui.touch_mode = touch;
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h.step();
    h
}

fn has_filmstrip(h: &Headless) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == "filmstrip")
}

/// On a phone held sideways (915x412) the filmstrip starts folded, the bottom bar's button opens
/// it, and the choice is kept per window class.
#[test]
fn filmstrip_folds_on_short_windows() {
    let mut h = detail_at([915.0, 412.0], true);
    assert!(!has_filmstrip(&h), "folded by default on a short window");
    let r = h.request("ui.clickWidget", json!({"id": "icon:filmstrip"}), T);
    assert_eq!(r["ok"], true, "the bottom bar handle: {r}");
    h.settle(SETTLE);
    h.step();
    assert!(has_filmstrip(&h), "the handle opens it");
    // growing the window restores the tall choice (shown) through the frame wiring in `update`
    h.size = egui::vec2(1600.0, 1000.0);
    h.settle(SETTLE);
    h.step();
    assert!(has_filmstrip(&h), "tall window shows its own (default) choice");
    assert!(h.app.ui.filmstrip && !h.app.ui.filmstrip_short && h.app.ui.filmstrip_other);
    // shrinking again restores the short choice (open)
    h.size = egui::vec2(915.0, 412.0);
    h.settle(SETTLE);
    h.step();
    assert!(h.app.ui.filmstrip_short && has_filmstrip(&h), "the short choice came back");
    // fold it on the short window: the tall choice is untouched, and both survive the settings file
    let r = h.request("ui.clickWidget", json!({"id": "icon:filmstrip"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h.step();
    assert!(!has_filmstrip(&h));
    let saved = serde_json::to_string(&h.app.ui).unwrap();
    assert!(saved.contains("\"filmstripShort\":true") && saved.contains("\"filmstripOther\":true"), "{saved}");
    let mut loaded: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert!(loaded.filmstrip_short && !loaded.filmstrip && loaded.filmstrip_other);
    loaded.set_short_window(false);
    assert!(loaded.filmstrip, "the desktop choice is untouched");
    // a fresh short session starts folded
    let mut ui = crate::UiState::default();
    ui.set_short_window(true);
    assert!(!ui.filmstrip);
}

/// The soft keyboard shrinks the content area but is not a window-class change: a tablet with the
/// keyboard up keeps its filmstrip.
#[test]
fn keyboard_inset_does_not_fold_the_filmstrip() {
    let mut h = detail_at([1280.0, 800.0], true);
    assert!(has_filmstrip(&h));
    h.safe_area = Some(egui::epaint::MarginF32 { left: 0.0, right: 0.0, top: 0.0, bottom: 350.0 });
    h.settle(SETTLE);
    h.step();
    assert!(!h.app.ui.filmstrip_short, "not a short window");
    assert!(has_filmstrip(&h), "the filmstrip stays with the keyboard up");
}

/// At 1280x800 with touch the panels fit: the left, centre and right areas don't overlap and stay
/// inside the window, and the resize grab area is finger-sized.
#[test]
fn tablet_breakpoint_layout() {
    let mut h = detail_at([1280.0, 800.0], true);
    let r = h.request("ui.set", json!({"leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    // the request's own pointer activity clears touch mode; set it again before the layout settles
    h.app.ui.touch_mode = true;
    h.settle(SETTLE);
    h.step();
    let window = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0));
    let panels: Vec<(String, egui::Rect)> = h.app.widgets.iter().filter(|(w, _)| w.starts_with("panel:")).map(|(w, r)| (w.to_string(), *r)).collect();
    assert!(panels.len() >= 2, "side panels are laid out: {panels:?}");
    for (id, r) in &panels {
        assert!(window.expand(0.5).contains_rect(*r), "{id} {r:?} inside the window");
    }
    for (i, (a, ra)) in panels.iter().enumerate() {
        for (b, rb) in &panels[i + 1..] {
            let overlap = ra.intersect(*rb);
            assert!(overlap.width() <= 0.5 || overlap.height() <= 0.5, "{a} {ra:?} overlaps {b} {rb:?}");
        }
    }
    let canvas = h.app.canvas_rect.expect("canvas");
    assert!(canvas.width() >= crate::state::MIN_PHOTO_WIDTH, "the photo keeps {} pt", canvas.width());
    for (id, r) in &panels {
        assert!(!r.intersects(canvas.shrink(1.0)), "{id} overlaps the photo");
    }
    assert!(
        h.view.ctx.global_style().interaction.resize_grab_radius_side >= 20.0,
        "touch-sized resize handle: {} (density {})",
        h.view.ctx.global_style().interaction.resize_grab_radius_side,
        h.app.ui.touch_density
    );
}
