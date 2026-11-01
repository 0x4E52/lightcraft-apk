//! Headless tests of the phone layout (APK-33, `crate::sheet`): below the phone breakpoint the
//! tool strip becomes the edit toolbar along the bottom and the side panels open as a sheet above
//! it, laid out to the sheet's width; Back closes the sheet first.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::{RightPanel, ViewMode};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);
/// A phone held upright, in points, and its scale (a 1080 × 2400 px screen at 420 dpi).
const PHONE: [f32; 2] = [412.0, 915.0];
const PHONE_SCALE: f32 = 2.625;

fn at(size: [f32; 2], scale: f32, view: &str) -> Headless {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, size, scale);
    h.app.ui.touch_mode = true;
    let r = h.request("ui.set", json!({"view": view}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    h
}

/// Settle, with touch mode set again (a control request's own pointer activity clears it).
fn frames(h: &mut Headless) {
    h.app.ui.touch_mode = true;
    h.settle(SETTLE);
    h.step();
    h.step();
}

fn rect(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

fn tap(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "tap {id}: {r}");
    frames(h);
}

/// At 412×915 the tool strip is gone and the edit toolbar runs along the bottom; a tool opens its
/// panel as a full-width sheet above the toolbar; at a desktop width the same session lays out the
/// strip and the side panel again.
#[test]
fn side_panels_become_sheet_below_breakpoint() {
    let mut h = at(PHONE, PHONE_SCALE, "detail");
    assert!(h.app.ui.sheet.phone);
    assert!(rect(&h, "icon:edit").is_none(), "no tool strip on a phone");
    let bar = rect(&h, "panel:edit_toolbar").expect("edit toolbar");
    assert!(bar.bottom() >= PHONE[1] - 1.0 && bar.height() >= 56.0 && bar.width() >= PHONE[0] - 1.0, "the toolbar spans the bottom: {bar:?}");
    assert!(rect(&h, "panel:sheet").is_none(), "the sheet starts down");
    let edit = rect(&h, "toolbar:edit").expect("Edit in the toolbar");
    assert!(bar.contains_rect(edit), "{edit:?} in {bar:?}");

    tap(&mut h, "toolbar:edit");
    let sheet = rect(&h, "panel:sheet").expect("Edit opens the sheet");
    assert!(sheet.width() >= PHONE[0] - 1.0, "full width: {sheet:?}");
    assert!((sheet.bottom() - bar.top()).abs() <= 1.0, "the sheet sits on the toolbar: {sheet:?} {bar:?}");
    let panel = rect(&h, "panel:right_panel").expect("the Edit panel is the sheet's content");
    assert!(sheet.expand(0.5).contains_rect(panel), "{panel:?} in {sheet:?}");
    let canvas = h.app.canvas_rect.expect("photo");
    assert!(canvas.bottom() <= sheet.top() + 0.5, "the photo stays above the sheet: {canvas:?}");
    assert!(canvas.width() >= PHONE[0] - 1.0, "and gets the whole width: {canvas:?}");

    // another tool swaps the sheet's content; the shown tool's own button folds it away
    tap(&mut h, "toolbar:crop");
    assert_eq!(h.app.ui.right, RightPanel::Crop);
    assert!(rect(&h, "panel:sheet").is_some());
    tap(&mut h, "toolbar:crop");
    assert!(rect(&h, "panel:sheet").is_none(), "the active tool folds the sheet");
    assert_eq!(h.app.ui.right, RightPanel::Crop, "the panel choice stays");
    // the row scrolls sideways: the last tool sits past the screen's edge until it is scrolled to
    let info = rect(&h, "toolbar:info").expect("Info in the toolbar");
    assert!(info.left() > PHONE[0] && info.width() >= 48.0, "{info:?}");

    // the Library is a sheet too, and a command (a shortcut, an agent) opens the sheet as a tap does
    tap(&mut h, "toolbar:library");
    assert!(rect(&h, "panel:left_panel").is_some_and(|r| rect(&h, "panel:sheet").is_some_and(|s| s.expand(0.5).contains_rect(r))));
    let r = h.request("engine.execute", json!({"command": "panel.info", "params": {}}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    assert_eq!(h.app.ui.sheet.content, crate::sheet::Content::Tools);
    assert!(rect(&h, "panel:sheet").is_some() && rect(&h, "panel:right_panel").is_some(), "panel.info opened the sheet");
    assert_eq!(h.app.ui.right, RightPanel::Info);

    // a desktop width: the strip and the dock again, no toolbar or sheet
    h.size = egui::vec2(1600.0, 1000.0);
    h.pixels_per_point = 1.0;
    frames(&mut h);
    assert!(!h.app.ui.sheet.phone);
    assert!(rect(&h, "icon:edit").is_some(), "the tool strip is back");
    assert!(rect(&h, "panel:edit_toolbar").is_none() && rect(&h, "panel:sheet").is_none());
    assert!(rect(&h, "panel:right_panel").is_some_and(|r| r.left() > 1000.0), "the right panel beside the photo");
}

/// Back (Escape, Android's Back key) puts the sheet down before it leaves the photo; the desktop
/// cascade is unchanged.
#[test]
fn back_closes_sheet_first() {
    let mut h = at(PHONE, PHONE_SCALE, "detail");
    tap(&mut h, "toolbar:edit");
    assert!(h.app.ui.sheet.open);
    assert_eq!(h.app.run("view.back", json!({})), Ok(serde_json::Value::Null));
    frames(&mut h);
    assert!(!h.app.ui.sheet.open && rect(&h, "panel:sheet").is_none(), "Back closed the sheet");
    assert_eq!(h.app.ui.view, ViewMode::Detail, "and stayed on the photo");
    assert_eq!(h.app.ui.right, RightPanel::Edit, "the Edit choice is kept");
    // the Escape key goes the same way
    tap(&mut h, "toolbar:edit");
    assert!(h.app.ui.sheet.open);
    let r = h.request("ui.key", json!({"key": "Escape"}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    assert!(!h.app.ui.sheet.open);
    assert_eq!(h.app.ui.view, ViewMode::Detail);
    // an overlay drawn over the sheet (a dialog, the full-screen preview) closes before the sheet
    for (open, label) in [("view.slideshowOptions", "dialog"), ("view.fullScreenPreview", "full-screen preview")] {
        tap(&mut h, "toolbar:edit");
        assert!(h.app.ui.sheet.open);
        assert!(h.app.run(open, json!({})).is_ok(), "{open}");
        frames(&mut h);
        assert_eq!(h.app.run("view.back", json!({})), Ok(serde_json::Value::Null));
        frames(&mut h);
        assert!(h.app.ui.dialog.is_none() && !h.app.ui.fullscreen, "Back closed the {label}");
        assert!(h.app.ui.sheet.open, "and left the sheet up under the {label}");
        assert_eq!(h.app.run("view.back", json!({})), Ok(serde_json::Value::Null));
        frames(&mut h);
        assert!(!h.app.ui.sheet.open, "the next Back put the sheet down ({label})");
        assert_eq!(h.app.ui.view, ViewMode::Detail);
    }
    // with the sheet down Back leaves the photo, as on the desktop
    assert_eq!(h.app.run("view.back", json!({})), Ok(serde_json::Value::Null));
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid);

    // the desktop: Back leaves the photo straight away, the right panel stays
    let mut d = at([1600.0, 1000.0], 1.0, "detail");
    assert_eq!(d.app.run("view.back", json!({})), Ok(serde_json::Value::Null));
    assert_eq!(d.app.ui.view, ViewMode::PhotoGrid);
    assert_eq!(d.app.ui.right, RightPanel::Edit);
}

/// Every Edit section open on a phone (412×915, and a narrow 360×800): nothing the sheet's
/// panels draw reaches past the sheet's sides, neither a registered widget nor text or a shape
/// painted straight with the painter (which the sheet's clip would cut off unseen): rows wrap,
/// labels elide, sliders fill the width. Every other tool panel too.
#[test]
fn no_panel_widget_outside_sheet() {
    let mut outside = vec![];
    for (size, scale) in [(PHONE, PHONE_SCALE), ([360.0, 800.0], 3.0)] {
        let mut h = at(size, scale, "detail");
        tap(&mut h, "toolbar:edit");
        h.app.ui.single_panel = false;
        h.app.ui.open_sections = ["light", "color", "effects", "detail", "optics", "calibration"].map(String::from).to_vec();
        frames(&mut h);
        for tool in ["edit", "crop", "remove", "masking", "redeye", "versions", "activity", "keywords", "info", "presets", "library"] {
            if tool != "edit" {
                // (as a tap does; most of the row is scrolled past the screen's edge)
                crate::sheet::tap(&mut h.app, tool);
                frames(&mut h);
            }
            assert!(h.app.ui.sheet.open, "{tool} opened the sheet");
            let content = match tool {
                "presets" => crate::sheet::Content::Presets,
                "library" => crate::sheet::Content::Library,
                _ => crate::sheet::Content::Tools,
            };
            assert_eq!(h.app.ui.sheet.content, content, "{tool}");
            let sheet = h.app.ui.sheet.rect.expect("the sheet");
            assert!(sheet.width() >= size[0] - 1.0, "{tool}: the sheet spans {size:?}: {sheet:?}");
            let at = format!("{}x{}", size[0], size[1]);
            // the space the content's widgets allocated is no wider than the sheet
            let body = h.app.ui.sheet.body.expect("the sheet's body");
            if body.left() < sheet.left() - 0.5 || body.right() > sheet.right() + 0.5 {
                outside.push(format!("{at} {tool}: content {body:?}"));
            }
            let drawn = h.app.ui.sheet.drawn.clone();
            let widgets = h.app.widgets.get(drawn).expect("the sheet's widgets");
            assert!(!widgets.is_empty(), "{tool}: the sheet drew its panel ({} widgets)", widgets.len());
            for (id, r) in widgets {
                if r.left() < sheet.left() - 0.5 || r.right() > sheet.right() + 0.5 {
                    outside.push(format!("{at} {tool}: {id} {r:?}"));
                }
            }
            // and nothing it painted was cut off at a side by a clip (the sheet's or a row's)
            for (shape, clip) in &h.app.ui.sheet.cut {
                outside.push(format!("{at} {tool}: painted {shape:?} cut by {clip:?}"));
            }
        }
    }
    assert!(outside.is_empty(), "drawn past the sheet's sides:\n{}", outside.join("\n"));
}

/// APK-32b's hit-size check at 412×915 under touch density, with the sheet up: the toolbar and the
/// sheet's handle are finger-sized, and nothing new joins the allow-list.
#[test]
fn min_hit_size_48dp_at_phone() {
    let mut h = at(PHONE, PHONE_SCALE, "photoGrid");
    tap(&mut h, "toolbar:edit");
    assert!(h.app.ui.touch_density);
    assert!(rect(&h, "panel:sheet").is_some());
    for id in ["toolbar:library", "toolbar:edit", "toolbar:info", "sheet:handle"] {
        let r = rect(&h, id).unwrap_or_else(|| panic!("{id} drawn"));
        assert!(r.width() >= 48.0 && r.height() >= 48.0, "{id} {r:?}");
    }
    let mut small: Vec<String> = h.app.widgets.iter().filter(|(_, r)| r.width() < 47.5 || r.height() < 47.5).map(|(id, _)| id.to_string()).collect();
    small.sort();
    small.dedup();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/small-hit-allow.txt");
    let allowed: std::collections::HashSet<String> = std::fs::read_to_string(path)
        .expect("allow-list")
        .lines()
        .filter_map(|l| l.split('#').next())
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let bad: Vec<&String> = small.iter().filter(|id| !allowed.contains(*id)).collect();
    assert!(bad.is_empty(), "widgets below 48 pt at 412x915 not in the allow-list: {bad:?}");
}
