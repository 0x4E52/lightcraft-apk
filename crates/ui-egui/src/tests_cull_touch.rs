//! APK-55: culling by finger in the loupe (swipes rate, flag and move on; a tap hides the controls; a two-finger
//! tap cycles the info overlay) and Cull Mode, all through `ui.touch` as a phone sends them.

use std::time::Duration;

use egui::{Pos2, pos2};
use lightcraft_catalog::{Flag, PhotoId};
use serde_json::{Value, json};

use crate::headless::Headless;
use crate::state::{InfoOverlay, LightsOut, ViewMode, Zoom};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(120);

fn loupe() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.settle(T);
    let r = h.request("ui.set", json!({"view": "detail", "right": "none", "left": "none"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.settle(T));
    h
}

fn run(h: &mut Headless, cmd: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": cmd, "params": params}), T);
    assert_eq!(r["ok"], true, "{cmd}: {r}");
    h.settle(T);
    r["result"].clone()
}

/// A screen point as `ui.touch`'s fractions of the image rect.
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

/// One finger from `from` by (`dx`, `dy`) points in 0.2 s.
fn swipe(h: &mut Headless, from: Pos2, dx: f32, dy: f32) {
    let mut ev = vec![touch(1, "start", frac(h, from), 0.0)];
    for i in 1..=4u8 {
        let k = f32::from(i) / 4.0;
        ev.push(touch(1, "move", frac(h, from + egui::vec2(dx * k, dy * k)), 0.05 * f64::from(i)));
    }
    ev.push(touch(1, "end", frac(h, from + egui::vec2(dx, dy)), 0.21));
    send(h, ev);
}

fn canvas(h: &Headless) -> egui::Rect {
    h.app.canvas_rect.expect("a canvas")
}

fn left_half(h: &Headless) -> Pos2 {
    let c = canvas(h);
    pos2(c.left() + c.width() * 0.25, c.center().y + 120.0)
}

fn right_half(h: &Headless) -> Pos2 {
    let c = canvas(h);
    pos2(c.left() + c.width() * 0.75, c.center().y + 120.0)
}

fn active(h: &Headless) -> PhotoId {
    h.app.session.active().expect("an active photo")
}

fn rating(h: &Headless) -> u8 {
    h.app.session.catalog.photo(active(h)).map(|p| p.rating).expect("photo")
}

fn flag(h: &Headless) -> Flag {
    h.app.session.catalog.photo(active(h)).map(|p| p.flag).expect("photo")
}

fn tap(h: &mut Headless, at: Pos2) {
    send(h, vec![touch(1, "start", frac(h, at), 0.0), touch(1, "end", frac(h, at), 0.05)]);
}

#[test]
fn swipe_left_half_rates() {
    let mut h = loupe();
    let id = active(&h);
    run(&mut h, "photo.rate", json!({"rating": 2, "ids": [id.0]}));
    let from = left_half(&h);
    swipe(&mut h, from, 0.0, -200.0);
    assert_eq!(rating(&h), 3, "a swipe up on the left half raises the rating");
    assert_eq!(active(&h), id, "and stays on the photo");
    assert!(h.app.ui.cull_hud.as_ref().is_some_and(|(text, _)| text == "★★★"), "with a HUD: {:?}", h.app.ui.cull_hud);
    run(&mut h, "edit.undo", json!({}));
    assert_eq!(rating(&h), 2, "one undo step");
    let from = left_half(&h);
    swipe(&mut h, from, 0.0, 200.0);
    assert_eq!(rating(&h), 1, "a swipe down lowers it");
    assert!(matches!(h.app.ui.zoom, Zoom::Fit), "nothing zoomed");
}

#[test]
fn swipe_right_half_flags() {
    let mut h = loupe();
    let id = active(&h);
    run(&mut h, "photo.flag", json!({"flag": "none", "ids": [id.0]}));
    let r0 = rating(&h);
    let from = right_half(&h);
    swipe(&mut h, from, 0.0, -200.0);
    assert_eq!(flag(&h), Flag::Pick, "up on the right half picks");
    swipe(&mut h, from, 0.0, 200.0);
    assert_eq!(flag(&h), Flag::None, "down unflags");
    swipe(&mut h, from, 0.0, 200.0);
    assert_eq!(flag(&h), Flag::Reject, "and down again rejects");
    assert!(h.app.ui.cull_hud.as_ref().is_some_and(|(text, _)| text == "Rejected"), "{:?}", h.app.ui.cull_hud);
    assert_eq!(rating(&h), r0, "the rating is left alone");
    assert_eq!(active(&h), id);
}

#[test]
fn sideways_swipe_navigates_at_fit() {
    let mut h = loupe();
    let vis = h.app.session.visible_cloned();
    run(&mut h, "library.select", json!({"ids": [vis[1].0]}));
    let r0 = rating(&h);
    let c = canvas(&h);
    swipe(&mut h, c.center(), -250.0, 0.0);
    assert_eq!(active(&h), vis[2], "a swipe to the left shows the next photo");
    let c = canvas(&h);
    swipe(&mut h, c.center(), 250.0, 0.0);
    assert_eq!(active(&h), vis[1], "to the right the previous one");
    assert_eq!(rating(&h), r0);
}

#[test]
fn zoomed_drag_still_pans() {
    let mut h = loupe();
    let id = active(&h);
    run(&mut h, "photo.rate", json!({"rating": 2, "ids": [id.0]}));
    run(&mut h, "view.navigate", json!({"zoom": {"percent": 300.0}, "pan": [0.5, 0.5]}));
    let before = h.app.ui.pan;
    let from = left_half(&h);
    swipe(&mut h, from, 0.0, -200.0);
    assert_ne!(h.app.ui.pan, before, "a one-finger drag pans a zoomed photo");
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "and stays zoomed");
    assert_eq!(rating(&h), 2, "it does not rate");
    let c = canvas(&h);
    swipe(&mut h, c.center(), -250.0, 0.0);
    assert_eq!(active(&h), id, "nor move to another photo");
}

#[test]
fn single_tap_toggles_controls() {
    let mut h = loupe();
    assert_eq!(h.app.ui.lights_out, LightsOut::On);
    let c = canvas(&h).center();
    tap(&mut h, c);
    assert!(h.step_until(T, |h| h.app.ui.lights_out == LightsOut::Off), "a tap hides the controls");
    assert!(matches!(h.app.ui.zoom, Zoom::Fit), "a tap is not a click zoom");
    tap(&mut h, c);
    assert!(h.step_until(T, |h| h.app.ui.lights_out == LightsOut::On), "a second tap brings them back");
    // a double tap zooms, and leaves the controls as they were
    let at = frac(&h, c);
    send(&mut h, vec![touch(1, "start", at, 0.0), touch(1, "end", at, 0.05), touch(1, "start", at, 0.15), touch(1, "end", at, 0.2)]);
    for _ in 0..40 {
        h.step();
    }
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "a double tap zooms: {:?}", h.app.ui.zoom);
    assert_eq!(h.app.ui.lights_out, LightsOut::On);
}

#[test]
fn two_finger_tap_cycles_overlay() {
    let mut h = loupe();
    assert_eq!(h.app.ui.info_overlay, InfoOverlay::Off);
    let c = canvas(&h).center();
    let (a, b) = (frac(&h, c - egui::vec2(60.0, 0.0)), frac(&h, c + egui::vec2(60.0, 0.0)));
    send(&mut h, vec![touch(1, "start", a, 0.0), touch(2, "start", b, 0.0), touch(1, "end", a, 0.1), touch(2, "end", b, 0.1)]);
    assert_eq!(h.app.ui.info_overlay, InfoOverlay::Basic, "two fingers tapped cycle the info overlay");
    assert!(matches!(h.app.ui.zoom, Zoom::Fit));
    for _ in 0..40 {
        h.step();
    }
    assert_eq!(h.app.ui.lights_out, LightsOut::On, "and are not a single tap");
}

#[test]
fn cull_mode_next_unflagged() {
    let mut h = loupe();
    let vis = h.app.session.visible_cloned();
    assert!(vis.len() >= 4, "the demo has photos");
    run(&mut h, "photo.flag", json!({"flag": "none", "ids": vis.iter().map(|p| p.0).collect::<Vec<_>>()}));
    run(&mut h, "photo.flag", json!({"flag": "pick", "ids": [vis[1].0]}));
    run(&mut h, "photo.flag", json!({"flag": "reject", "ids": [vis[2].0]}));
    run(&mut h, "library.select", json!({"ids": [vis[0].0]}));
    let r = run(&mut h, "view.cullMode", json!({}));
    assert_eq!(r["cullMode"], true, "{r}");
    let has = |h: &Headless, id: &str| h.app.widgets.iter().any(|(w, _)| w == id);
    assert!(has(&h, "cull:nextUnflagged") && has(&h, "filmstrip"), "the cull bar and the filmstrip show");
    assert!(!h.app.widgets.iter().any(|(w, _)| w == "region:titlebar" || w == "icon:filter"), "no top bar");
    let r = h.request("ui.clickWidget", json!({"id": "cull:nextUnflagged"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert_eq!(active(&h), vis[3], "skips the pick and the reject");
    // the gestures work in Cull Mode too
    let from = right_half(&h);
    swipe(&mut h, from, 0.0, -200.0);
    assert_eq!(flag(&h), Flag::Pick);
    // a long press on a filmstrip photo compares it with the active one
    let film = h.app.widgets.iter().find(|(w, _)| *w == format!("film:{}", vis[0].0)).map(|(_, r)| *r).expect("filmstrip cell");
    let at = frac(&h, film.center());
    send(&mut h, vec![touch(1, "start", at, 0.0), touch(1, "move", at, 1.0), touch(1, "move", at, 1.2), touch(1, "end", at, 1.3)]);
    h.step();
    assert_eq!(h.app.ui.view, ViewMode::Compare, "holding a filmstrip photo compares");
    assert_eq!(h.app.ui.compare, Some((vis[3].0, vis[0].0)));
    assert!(h.app.ui.cull_mode);
    // Back: to the loupe on the photo being culled (alone), then out of Cull Mode
    run(&mut h, "view.back", json!({}));
    assert_eq!(h.app.ui.view, ViewMode::Detail);
    assert!(h.app.ui.cull_mode);
    assert_eq!(active(&h), vis[3], "back on the photo being culled, not the candidate");
    assert_eq!(h.app.session.selection.ids.len(), 1, "alone");
    run(&mut h, "view.back", json!({}));
    assert!(!h.app.ui.cull_mode && h.app.ui.view == ViewMode::Detail, "Back leaves Cull Mode for the loupe");
}

#[test]
fn cull_mode_needs_a_photo_and_next_unflagged_says_when_there_is_none() {
    let mut h = loupe();
    let vis = h.app.session.visible_cloned();
    run(&mut h, "photo.flag", json!({"flag": "pick", "ids": vis.iter().map(|p| p.0).collect::<Vec<_>>()}));
    let r = h.request("engine.execute", json!({"command": "view.nextUnflagged", "params": {}}), T);
    assert_eq!(r["ok"], false, "{r}");
    run(&mut h, "library.selectNone", json!({}));
    h.app.session.selection = Default::default();
    let r = h.request("engine.execute", json!({"command": "view.cullMode", "params": {"on": true}}), T);
    assert_eq!(r["ok"], false, "{r}");
    assert!(!h.app.ui.cull_mode);
}

#[test]
fn a_mouse_click_still_zooms_and_a_mouse_drag_does_not_rate() {
    let mut h = loupe();
    let id = active(&h);
    run(&mut h, "photo.rate", json!({"rating": 2, "ids": [id.0]}));
    let from = left_half(&h);
    let r = h.request("ui.drag", json!({"x": from.x, "y": from.y, "toX": from.x, "toY": from.y - 200.0}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert_eq!(rating(&h), 2, "a mouse drag is not a swipe");
    let c = canvas(&h).center();
    let r = h.request("ui.click", json!({"x": c.x, "y": c.y}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "a click zooms: {:?}", h.app.ui.zoom);
    assert_eq!(h.app.ui.lights_out, LightsOut::On);
}

#[test]
fn leaving_the_loupe_another_way_ends_cull_mode() {
    let mut h = loupe();
    run(&mut h, "view.cullMode", json!({"on": true}));
    assert!(h.app.ui.cull_mode);
    // the grid by its shortcut's command, not by Done or Back
    run(&mut h, "view.gridToggle", json!({}));
    assert!(matches!(h.app.ui.view, ViewMode::PhotoGrid | ViewMode::SquareGrid));
    assert!(!h.app.ui.cull_mode, "Cull Mode ends with the loupe");
}

#[test]
fn assisted_culling_from_the_cull_bar_keeps_the_selection() {
    let mut h = loupe();
    let vis = h.app.session.visible_cloned();
    let picked: Vec<u64> = vis.iter().take(3).map(|p| p.0).collect();
    run(&mut h, "library.select", json!({"ids": picked}));
    run(&mut h, "view.cullMode", json!({"on": true}));
    let r = h.request("ui.clickWidget", json!({"id": "cull:assisted"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert!(matches!(h.app.ui.dialog, Some(crate::state::Dialog::Cull { visible: true, .. })), "the dialog scores the photos in view");
    let mut sel: Vec<u64> = h.app.session.selection.ids.iter().map(|p| p.0).collect();
    sel.sort_unstable();
    let mut want = picked.clone();
    want.sort_unstable();
    assert_eq!(sel, want, "the selection is left as it was");
}

#[test]
fn a_tap_on_a_widget_over_the_photo_is_the_widget_s() {
    let mut h = loupe();
    // the Navigator, over the zoomed photo: a tap there pans and does not also hide the controls
    run(&mut h, "view.navigate", json!({"zoom": {"percent": 300.0}, "pan": [0.5, 0.5]}));
    let nav = h.app.widgets.iter().find(|(w, _)| w == "canvas:navigator").map(|(_, r)| *r).expect("the navigator shows zoomed in");
    let at = nav.left_top() + (nav.max - nav.min) * 0.25;
    tap(&mut h, at);
    for _ in 0..40 {
        h.step();
    }
    assert!(h.app.ui.pan.0 < 0.4 && h.app.ui.pan.1 < 0.4, "the navigator panned: {:?}", h.app.ui.pan);
    assert_eq!(h.app.ui.lights_out, LightsOut::On, "and the tap did not also hide the controls");
    // a double tap there does not zoom the photo to fit either
    let at = frac(&h, at);
    send(&mut h, vec![touch(1, "start", at, 0.0), touch(1, "end", at, 0.05), touch(1, "start", at, 0.15), touch(1, "end", at, 0.2)]);
    for _ in 0..40 {
        h.step();
    }
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "{:?}", h.app.ui.zoom);
    assert_eq!(h.app.ui.lights_out, LightsOut::On);
    // the filter pill in the canvas corner clears the filters and leaves the controls alone
    run(&mut h, "view.zoomFit", json!({}));
    let id = active(&h);
    h.app.session.filter.text = h.app.session.catalog.photo(id).map(|p| p.file_name.clone()).expect("photo");
    assert!(h.settle(T));
    let pill = h.app.widgets.iter().find(|(w, _)| w == "loupe:filters").map(|(_, r)| *r).expect("the filter pill shows");
    tap(&mut h, pill.center());
    for _ in 0..40 {
        h.step();
    }
    assert!(h.app.session.filter.text.is_empty(), "the pill cleared the filter");
    assert_eq!(h.app.ui.lights_out, LightsOut::On);
    // the photo itself still takes a tap
    let c = canvas(&h).center();
    tap(&mut h, c);
    assert!(h.step_until(T, |h| h.app.ui.lights_out == LightsOut::Off), "a tap on the photo hides the controls");
}
