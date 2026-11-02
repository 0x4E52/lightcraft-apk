//! APK-56: fingers paint brush strokes at full pressure, a stylus's force scales them; the desktop keeps Pen
//! Pressure on.

use crate::headless::Headless;
use crate::panels::detail::{record_pressure, stroke_force};
use crate::{LightcraftApp, Services};

fn app_after_a_frame(touch: bool, events: Vec<egui::Event>) -> Headless {
    let services = Services { png: None, touch, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [800.0, 600.0], 1.0);
    h.events = events;
    h.step();
    h
}

fn touch_event(force: Option<f32>) -> egui::Event {
    egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::Move, pos: egui::pos2(10.0, 10.0), force }
}

fn input(force: Option<f32>) -> egui::InputState {
    let mut i = egui::InputState::default();
    i.events = vec![touch_event(force)];
    i
}

#[test]
fn finger_touch_paints_full_pressure() {
    let h = app_after_a_frame(true, vec![]);
    assert!(h.app.ui.touch_mode);
    assert!(!h.app.ui.brush_pressure, "a touch-first host starts with Pen Pressure off");
    // Android reports a contact pressure for a finger: it is not recorded
    let force = stroke_force(h.app.ui.brush_pressure, &input(Some(0.2)));
    let mut pressure = Vec::new();
    record_pressure(&mut pressure, 0, force);
    record_pressure(&mut pressure, 1, force);
    assert!(pressure.is_empty(), "no pressure list = full pressure");
}

#[test]
fn stylus_force_scales_dab() {
    let mut h = app_after_a_frame(true, vec![]);
    h.app.ui.brush_pressure = true; // the user switched it on in the brush sheet
    let mut pressure = Vec::new();
    for f in [0.3, 0.9] {
        let n = pressure.len();
        record_pressure(&mut pressure, n, stroke_force(h.app.ui.brush_pressure, &input(Some(f))));
    }
    assert_eq!(pressure, vec![0.3, 0.9]);
    h.step();
    assert!(h.app.ui.brush_pressure, "the choice is kept");
}

#[test]
fn desktop_pen_pressure_unchanged() {
    let h = app_after_a_frame(false, vec![touch_event(Some(0.5))]);
    assert!(h.app.ui.touch_mode, "a touch event turns touch mode on");
    assert!(h.app.ui.brush_pressure, "the desktop keeps Pen Pressure on");
    assert_eq!(stroke_force(h.app.ui.brush_pressure, &input(Some(0.5))), Some(0.5));
}

#[test]
fn touch_brush_sheet_explains_pen_pressure() {
    use serde_json::json;
    let services = Services { png: None, touch: true, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1200.0, 1600.0], 1.0);
    h.step();
    let t = std::time::Duration::from_secs(20);
    assert_eq!(h.request("ui.set", json!({"view": "detail"}), t)["ok"], true);
    for (command, params) in [("panel.masking", json!({})), ("tool.brush", json!({"new": true}))] {
        let r = h.request("engine.execute", json!({"command": command, "params": params}), t);
        assert_eq!(r["ok"], true, "{command}: {r}");
    }
    let r = h.request("ui.pointer", json!({"events": [{"kind": "down", "x": 0.5, "y": 0.5}, {"kind": "up", "x": 0.5, "y": 0.5}]}), t);
    assert_eq!(r["ok"], true, "{r}");
    for _ in 0..3 {
        h.step();
    }
    let text = h.painted_text();
    assert!(text.iter().any(|t| t.contains("For a stylus")), "{text:?}");
}
