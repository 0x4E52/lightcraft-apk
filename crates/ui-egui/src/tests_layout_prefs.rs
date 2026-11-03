//! Left-handed and remembered layouts (APK-63): the phone toolbar's order and the tablet panel
//! side mirror, the choices are kept per breakpoint, and a saved choice from an older layout
//! version is migrated rather than dropped.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::{LAYOUT_PREFS_VERSION, LayoutPrefs, UiState};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn at(size: [f32; 2], scale: f32) -> Headless {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, size, scale);
    h.app.ui.touch_mode = true;
    let r = h.request("ui.set", json!({"view": "detail", "leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    h
}

fn frames(h: &mut Headless) {
    h.app.ui.touch_mode = true;
    h.settle(SETTLE);
    h.step();
    h.step();
}

fn rect(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

#[test]
fn left_handed_mirrors_toolbar_and_panel() {
    // phone: the first tool is at the left edge, then at the right edge when mirrored
    let mut h = at([412.0, 915.0], 2.625);
    let first = rect(&h, "toolbar:library").expect("library tool");
    let last = rect(&h, "toolbar:edit").expect("edit tool");
    assert!(first.center().x < last.center().x, "tools run left to right by default");
    h.app.ui.layout.phone.left_handed = true;
    frames(&mut h);
    let first = rect(&h, "toolbar:library").expect("library tool");
    let last = rect(&h, "toolbar:edit").expect("edit tool");
    assert!(first.center().x > last.center().x, "mirrored: the Library tool is right of Edit");
    // compact: icon-only buttons are narrower
    let wide = first.width();
    h.app.ui.layout.phone.compact_toolbar = true;
    frames(&mut h);
    let narrow = rect(&h, "toolbar:library").expect("library tool").width();
    assert!(narrow < wide && narrow >= 48.0, "{narrow} vs {wide}");

    // tablet: library left / tools right by default, swapped when left-handed
    let mut h = at([1280.0, 800.0], 1.0);
    let lib = rect(&h, "panel:left_panel").expect("library panel");
    let tools = rect(&h, "panel:right_panel").expect("tool panel");
    assert!(lib.center().x < tools.center().x);
    h.app.ui.layout.tablet.left_handed = true;
    frames(&mut h);
    let lib = rect(&h, "panel:left_panel").expect("library panel");
    let tools = rect(&h, "panel:right_panel").expect("tool panel");
    assert!(lib.center().x > tools.center().x, "mirrored: {lib:?} {tools:?}");
    let canvas = h.app.canvas_rect.expect("canvas");
    assert!(canvas.width() >= crate::state::MIN_PHOTO_WIDTH);
    // back again restores the original sides
    h.app.ui.layout.tablet.left_handed = false;
    frames(&mut h);
    assert!(rect(&h, "panel:left_panel").expect("lib").center().x < rect(&h, "panel:right_panel").expect("tools").center().x);
}

#[test]
fn layout_choice_survives_restart() {
    let mut ui = UiState::default();
    ui.layout.phone.left_handed = true;
    ui.layout.phone.compact_toolbar = true;
    ui.layout.tablet.left_handed = false;
    let saved = serde_json::to_value(&ui).unwrap();
    let back: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(back.layout, ui.layout, "per-breakpoint choices come back");
    assert!(back.layout.phone.left_handed && !back.layout.tablet.left_handed);

    // a mirrored dock survives too, and mirroring twice is the identity
    let mut h = at([1280.0, 800.0], 1.0);
    h.app.ui.layout.tablet.left_handed = true;
    frames(&mut h);
    let saved = serde_json::to_value(&h.app.ui).unwrap();
    let back: UiState = serde_json::from_value(saved).unwrap();
    assert!(back.docking_mirrored && back.layout.tablet.left_handed);
    let mut w = back.docking.clone().expect("workspace");
    let once = w.clone();
    w.mirror();
    w.mirror();
    assert_eq!(serde_json::to_value(&w).unwrap(), serde_json::to_value(&once).unwrap());

    // settings without any layout choice read as the defaults
    let old: UiState = serde_json::from_value(json!({})).unwrap();
    assert_eq!(old.layout, LayoutPrefs::default());
}

#[test]
fn layout_pref_migrates_on_version_bump() {
    // version 0 kept one flat choice; it carries over to the breakpoints instead of being dropped
    let old: UiState = serde_json::from_value(json!({"layout": {"leftHanded": true, "compactToolbar": true}})).unwrap();
    assert_eq!(old.layout.version, LAYOUT_PREFS_VERSION);
    assert!(old.layout.phone.left_handed && old.layout.tablet.left_handed && old.layout.phone.compact_toolbar);
    // a current save is read as it is, not migrated over
    let cur: UiState = serde_json::from_value(json!({"layout": {"version": 1, "phone": {"leftHanded": true}, "leftHanded": false}})).unwrap();
    assert!(cur.layout.phone.left_handed && !cur.layout.tablet.left_handed);
    // a save from a newer build keeps the fields this one knows
    let newer: UiState = serde_json::from_value(json!({"layout": {"version": 9, "tablet": {"leftHanded": true}, "extra": 1}})).unwrap();
    assert!(newer.layout.tablet.left_handed);
    assert_eq!(newer.layout.version, 9, "a newer save keeps its version");
}

#[test]
fn reset_and_toolbar_stay_mirrored() {
    // tablet: Reset Panel Layout while Left-handed keeps the mirror
    let mut h = at([1280.0, 800.0], 1.0);
    h.app.ui.layout.tablet.left_handed = true;
    frames(&mut h);
    h.app.run("ui.dock.reset", json!({})).unwrap();
    frames(&mut h);
    let tools = rect(&h, "panel:right_panel").expect("tool panel");
    let canvas = h.app.canvas_rect.expect("canvas");
    assert!(tools.center().x < canvas.center().x, "reset keeps the tool panel on the left: {tools:?} {canvas:?}");

    // phone: mirrored, the Library tool is visible at the right edge
    let mut h = at([412.0, 915.0], 2.625);
    h.app.ui.layout.phone.left_handed = true;
    frames(&mut h);
    let lib = rect(&h, "toolbar:library").expect("library tool");
    assert!(lib.right() > 412.0 - 24.0 && lib.right() <= 412.0, "Library at the right edge: {lib:?}");
}

#[test]
fn invalid_docking_restores_mirrored() {
    // the restore fallback rebuilds the layout; with Left-handed on it must still come out mirrored
    let mut h = at([1280.0, 800.0], 1.0);
    h.app.ui.layout.tablet.left_handed = true;
    frames(&mut h);
    let mut broken = h.app.ui.docking.clone().expect("workspace");
    broken.layout.root = None; // no document view: fails validation
    h.app.ui.docking = Some(broken);
    frames(&mut h);
    assert!(h.app.ui.docking_mirrored);
    let tools = rect(&h, "panel:right_panel").expect("tool panel");
    let canvas = h.app.canvas_rect.expect("canvas");
    assert!(tools.center().x < canvas.center().x, "mirrored after the fallback: {tools:?} {canvas:?}");
}
