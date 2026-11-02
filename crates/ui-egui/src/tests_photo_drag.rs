//! APK-61: slider gestures on the photo (a vertical drag chooses one of the open panel's sliders, a horizontal
//! drag changes it in one undo step) and the touch slider (a tap beside the thumb nudges, a double tap on the
//! thumb resets), all through `ui.touch` as a phone sends them.

use std::time::Duration;

use egui::{Pos2, pos2, vec2};
use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(120);

/// The demo library in Detail with the Edit panel's Light section open, a finger as the pointer.
fn edit_panel() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.settle(T);
    let r = h.request("ui.set", json!({"view": "detail", "right": "edit", "left": "none"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.app.ui.open_sections = vec!["light".to_string()];
    h.app.ui.touch_mode = true;
    assert!(h.settle(T));
    h
}

fn run(h: &mut Headless, cmd: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": cmd, "params": params}), T);
    assert_eq!(r["ok"], true, "{cmd}: {r}");
    h.settle(T);
    r["result"].clone()
}

/// A screen point as `ui.touch`'s fractions of the image rect (points off the photo give fractions outside 0..1).
fn frac(h: &Headless, p: Pos2) -> (f64, f64) {
    let img = h.app.image_rect.expect("a photo is shown");
    (f64::from((p.x - img.left()) / img.width()), f64::from((p.y - img.top()) / img.height()))
}

fn touch(id: u64, phase: &str, (x, y): (f64, f64), t: f64) -> Value {
    json!({"id": id, "phase": phase, "x": x, "y": y, "t": t})
}

fn send(h: &mut Headless, events: Vec<Value>) {
    let r = h.request("ui.touch", json!({"events": events}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
}

/// One finger from `from`: first 20 points along the drag (it locks to that axis), then the rest of (`dx`, `dy`)
/// in four moves, in 0.25 s (a flick, which would rate, flag or change photo with no panel open).
fn drag(h: &mut Headless, from: Pos2, dx: f32, dy: f32) {
    let len = vec2(dx, dy).length().max(1.0);
    let lock = from + vec2(dx, dy) * (20.0 / len);
    let mut ev = vec![touch(1, "start", frac(h, from), 0.0), touch(1, "move", frac(h, lock), 0.04)];
    for i in 1..=4u8 {
        let k = f32::from(i) / 4.0;
        ev.push(touch(1, "move", frac(h, lock + (from + vec2(dx, dy) - lock) * k), 0.04 + 0.05 * f64::from(i)));
    }
    ev.push(touch(1, "end", frac(h, from + vec2(dx, dy)), 0.25));
    send(h, ev);
}

fn canvas(h: &Headless) -> egui::Rect {
    h.app.canvas_rect.expect("a canvas")
}

fn left_half(h: &Headless) -> Pos2 {
    let c = canvas(h);
    pos2(c.left() + c.width() * 0.25, c.center().y - 150.0)
}

fn active(h: &Headless) -> PhotoId {
    h.app.session.active().expect("an active photo")
}

fn value(h: &Headless, control: &str) -> f64 {
    lightcraft_develop::controls::get(&h.app.session.develop_of(active(h)).unwrap_or_default(), control).expect("a control")
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

/// Where the thumb of `control`'s slider is now, plus `dx` points.
fn thumb(h: &Headless, control: &str, dx: f32) -> Pos2 {
    let spec = lightcraft_develop::controls::find(control).expect("a control");
    let track = widget(h, &format!("slider:{control}"));
    pos2(track.left() + spec.position(value(h, control)) as f32 * track.width() + dx, track.center().y)
}

fn tap(h: &mut Headless, at: Pos2, t0: f64) {
    let p = frac(h, at);
    send(h, vec![touch(1, "start", p, t0), touch(1, "end", p, t0 + 0.05)]);
}

#[test]
fn vertical_drag_selects_control() {
    let mut h = edit_panel();
    let offered: Vec<String> = h.app.photo_drag.offered_ids().into_iter().map(str::to_string).collect();
    assert!(offered.len() >= 5 && offered.iter().any(|c| c == "light.exposure"), "the Light section's sliders: {offered:?}");
    assert!(offered.iter().all(|c| lightcraft_develop::controls::find(c).is_some()), "every one a develop control");
    let id = active(&h);
    run(&mut h, "photo.rate", json!({"rating": 2, "ids": [id.0]}));
    let first = offered.iter().position(|c| c == "light.exposure").expect("exposure");
    h.app.ui.photo_drag_control = Some("light.exposure".into());
    let undo = h.app.session.undo.len();
    // down three rows (20 points to lock, then 3 × 36 and a little)
    let from = left_half(&h);
    drag(&mut h, from, 0.0, 20.0 + 3.0 * 36.0 + 10.0);
    assert_eq!(h.app.ui.photo_drag_control.as_deref(), offered.get(first + 3).map(String::as_str), "three sliders down");
    assert!(h.app.widgets.iter().any(|(w, _)| w == "loupe:photoDragHud"), "with a readout over the photo");
    // and up one
    let from = left_half(&h);
    drag(&mut h, from, 0.0, -(20.0 + 36.0 + 10.0));
    assert_eq!(h.app.ui.photo_drag_control.as_deref(), offered.get(first + 2).map(String::as_str), "one back up");
    let rating = h.app.session.catalog.photo(id).map(|p| p.rating);
    assert_eq!(rating, Some(2), "with a panel open a vertical flick is not a rating swipe");
    assert_eq!(active(&h), id);
    assert_eq!(h.app.session.undo.len(), undo, "choosing edits nothing");
}

#[test]
fn horizontal_drag_changes_value_one_undo() {
    let mut h = edit_panel();
    let id = active(&h);
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0.0}));
    h.app.ui.photo_drag_control = Some("light.exposure".into());
    let undo = h.app.session.undo.len();
    let c = canvas(&h);
    drag(&mut h, c.center(), 200.0, 0.0);
    let v = value(&h, "light.exposure");
    assert!(v > 0.0, "a drag to the right raises Exposure: {v}");
    let spec = lightcraft_develop::controls::find("light.exposure").expect("exposure");
    let expect = spec.value_at(spec.position(0.0) + f64::from(180.0 / c.width()) * crate::panels::photo_drag::SPAN);
    // (the photo's area is the canvas less its margins, so a little narrower)
    assert!((v - expect).abs() <= 0.1 * expect, "as far as dragging the thumb would: {v} vs {expect}");
    assert_eq!(active(&h), id, "not a swipe to the next photo");
    assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step for the gesture");
    assert!(h.app.ui.dragging_control.is_none(), "the interaction ended");
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(value(&h, "light.exposure"), 0.0, "one undo takes it all back");
    let c = canvas(&h);
    drag(&mut h, c.center(), -200.0, 0.0);
    assert!(value(&h, "light.exposure") < 0.0, "to the left lowers it");
    // a mouse drag on the photo does nothing of the kind
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0.0}));
    let r = h.request(
        "ui.pointer",
        json!({"events": [
            {"kind": "move", "x": 0.4, "y": 0.5}, {"kind": "down", "x": 0.4, "y": 0.5},
            {"kind": "move", "x": 0.5, "y": 0.5}, {"kind": "move", "x": 0.6, "y": 0.5}, {"kind": "up", "x": 0.6, "y": 0.5}
        ]}),
        T,
    );
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert_eq!(value(&h, "light.exposure"), 0.0, "a mouse drag doesn't edit");
}

#[test]
fn tool_owns_canvas_no_drag_edit() {
    let mut h = edit_panel();
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0.0}));
    h.app.ui.photo_drag_control = Some("light.exposure".into());
    // a picker owns the photo (the Edit panel stays open)
    h.app.ui.tool = "pointColor".into();
    h.settle(T);
    let c = canvas(&h);
    drag(&mut h, c.center(), 200.0, 0.0);
    assert_eq!(value(&h, "light.exposure"), 0.0, "a picker's drag edits nothing");
    assert!(!h.app.photo_drag.offered_ids().is_empty(), "the Edit panel still offers its sliders");
    assert!(!h.app.photo_drag.armed, "but the gesture is off while the picker owns the photo");
    // the White Balance picker, started from the Edit panel whose sliders stay drawn
    h.app.ui.tool = "wbPicker".into();
    h.settle(T);
    assert!(!h.app.photo_drag.offered_ids().is_empty() && !h.app.photo_drag.armed, "the WB picker owns the photo");
    let c = canvas(&h);
    drag(&mut h, c.center(), 200.0, 0.0);
    assert_eq!(value(&h, "light.exposure"), 0.0, "the WB picker's drag edits nothing");
    assert!(h.app.ui.dragging_control.is_none());
    h.app.ui.tool.clear();
    h.settle(T);
    assert!(h.app.photo_drag.armed, "armed again once the tool is put away");
    // Crop, Masking, Remove and Red Eye own it, and their panels offer no develop sliders
    for panel in ["crop", "masking", "remove", "redEye"] {
        let r = h.request("ui.set", json!({"right": panel}), T);
        assert_eq!(r["ok"], true, "{panel}: {r}");
        h.settle(T);
        let c = canvas(&h);
        drag(&mut h, c.center() + vec2(-100.0, 0.0), 200.0, 0.0);
        assert_eq!(value(&h, "light.exposure"), 0.0, "{panel}: a drag is the tool's");
        assert!(!h.app.photo_drag.armed, "{panel}: off while the tool owns the photo");
        assert!(h.app.ui.dragging_control.is_none(), "{panel}");
    }
    // with the panel closed there is nothing to choose from: a flick is a swipe again
    let r = h.request("ui.set", json!({"right": "none"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    let vis = h.app.session.visible_cloned();
    let at = vis.iter().position(|p| *p == active(&h)).expect("in view");
    let c = canvas(&h);
    drag(&mut h, c.center(), -250.0, 0.0);
    assert_eq!(Some(active(&h)), vis.get(at + 1).copied(), "the swipe shows the next photo");
    assert_eq!(value(&h, "light.exposure"), 0.0);
}

#[test]
fn tap_beside_thumb_nudges() {
    let mut h = edit_panel();
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0.0}));
    let spec = lightcraft_develop::controls::find("light.exposure").expect("exposure");
    let step = crate::widgets::nudged(spec, 0.0, 1.0);
    assert!(step > 0.0 && step < 0.2, "a small step: {step}");
    let undo = h.app.session.undo.len();
    let at = thumb(&h, "light.exposure", 90.0);
    tap(&mut h, at, 0.0);
    assert!((value(&h, "light.exposure") - step).abs() < 1e-9, "a tap right of the thumb nudges up, not jumps: {}", value(&h, "light.exposure"));
    assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
    let at = thumb(&h, "light.exposure", -90.0);
    tap(&mut h, at, 1.0);
    assert!(value(&h, "light.exposure").abs() < 1e-9, "left of it nudges down");
    // two quick taps beside the thumb are two nudges, not a reset
    let at = thumb(&h, "light.exposure", 90.0);
    let p = frac(&h, at);
    send(&mut h, vec![touch(1, "start", p, 2.0), touch(1, "end", p, 2.05), touch(1, "start", p, 2.15), touch(1, "end", p, 2.2)]);
    assert!((value(&h, "light.exposure") - 2.0 * step).abs() < 1e-9, "two nudges: {}", value(&h, "light.exposure"));
}

#[test]
fn double_tap_thumb_resets() {
    let mut h = edit_panel();
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 1.0}));
    let undo = h.app.session.undo.len();
    // a fingertip lands off the thumb's centre: past the mouse's reach (THUMB_RING + 1 = 8 pt),
    // inside the finger-sized thumb (TOUCH_HIT / 2 = 24 pt)
    let at = thumb(&h, "light.exposure", 16.0);
    let p = frac(&h, at);
    send(&mut h, vec![touch(1, "start", p, 0.0), touch(1, "end", p, 0.05), touch(1, "start", p, 0.15), touch(1, "end", p, 0.2)]);
    assert_eq!(value(&h, "light.exposure"), 0.0, "a double tap on the thumb resets Exposure");
    assert_eq!(h.app.session.undo.len(), undo + 1, "in one undo step (the first tap on the thumb moved nothing)");
    // beside the finger-sized thumb (40 pt off), a double tap is two nudges, not a reset
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 1.0}));
    let at = thumb(&h, "light.exposure", 40.0);
    let p = frac(&h, at);
    send(&mut h, vec![touch(1, "start", p, 1.0), touch(1, "end", p, 1.05), touch(1, "start", p, 1.15), touch(1, "end", p, 1.2)]);
    assert!(value(&h, "light.exposure") > 1.0, "40 pt off the thumb nudges up: {}", value(&h, "light.exposure"));
}

#[test]
fn settings_switch_turns_it_off() {
    let mut h = edit_panel();
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0.0}));
    assert!(h.app.ui.photo_drag, "on by default");
    // Settings ▸ Interface has the switch
    h.app.ui.dialog = Some(crate::state::Dialog::Settings { tab: "interface".into() });
    h.settle(T);
    // (below the Grid and Keywords settings: scroll the tab until it is in the window)
    for _ in 0..20 {
        let w = widget(&h, "check:settings.photoDrag");
        if w.bottom() < 880.0 && w.top() > 0.0 {
            break;
        }
        let r = h.request("ui.hoverWidget", json!({"id": "check:settings.filmNames"}), T);
        assert_eq!(r["ok"], true, "{r}");
        let r = h.request("ui.scroll", json!({"dy": -300.0}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.settle(T);
    }
    let r = h.request("ui.clickWidget", json!({"id": "check:settings.photoDrag"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert!(!h.app.ui.photo_drag, "the checkbox turns it off");
    h.app.ui.dialog = None;
    h.settle(T);
    assert!(!h.app.photo_drag.armed, "off: not armed with the Edit panel open");
    // with the Edit panel open, a sideways flick is the swipe to the next photo again and edits nothing
    let vis = h.app.session.visible_cloned();
    let at = vis.iter().position(|p| *p == active(&h)).expect("in view");
    let undo = h.app.session.undo.len();
    let c = canvas(&h);
    drag(&mut h, c.center(), -250.0, 0.0);
    assert_eq!(Some(active(&h)), vis.get(at + 1).copied(), "the swipe shows the next photo");
    assert_eq!(h.app.session.undo.len(), undo, "no slider edit");
    // and `ui.set` turns it back on
    let r = h.request("ui.set", json!({"photoDrag": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert!(h.app.ui.photo_drag && h.app.photo_drag.armed);
}

#[test]
fn view_change_mid_drag_ends_interaction() {
    let mut h = edit_panel();
    run(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0.0}));
    h.app.ui.photo_drag_control = Some("light.exposure".into());
    let undo = h.app.session.undo.len();
    let c = canvas(&h);
    let from = c.center();
    // a finger locks sideways and moves, then stays down
    let ev = vec![
        touch(1, "start", frac(&h, from), 0.0),
        touch(1, "move", frac(&h, from + vec2(20.0, 0.0)), 0.04),
        touch(1, "move", frac(&h, from + vec2(120.0, 0.0)), 0.08),
    ];
    send(&mut h, ev);
    assert_eq!(h.app.ui.dragging_control.as_deref(), Some("light.exposure"), "the drag is under way");
    let moved = value(&h, "light.exposure");
    assert!(moved > 0.0, "{moved}");
    let lift = frac(&h, from + vec2(120.0, 0.0));
    // the view changes under the finger (a key, Back): the loupe is gone
    let r = h.request("ui.set", json!({"view": "photoGrid"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert!(h.app.ui.dragging_control.is_none(), "the interaction ended with the loupe");
    assert_eq!(h.app.session.undo.len(), undo + 1, "what was done is one undo step");
    assert_eq!(value(&h, "light.exposure"), moved, "and stays");
    // the finger lifts in the grid; back in Detail a new one-finger drag works (not taken for a second finger)
    send(&mut h, vec![touch(1, "end", lift, 0.5)]);
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    let c = canvas(&h);
    drag(&mut h, c.center(), 200.0, 0.0);
    assert!(value(&h, "light.exposure") > moved, "a new drag changes it again");
    assert_eq!(h.app.session.undo.len(), undo + 2);
    assert!(h.app.ui.dragging_control.is_none());
}
