//! APK-58: presets by touch. In the phone layout the Presets sheet stays up; a tap previews a preset on the photo
//! (`HoverPreview`, nothing committed), a second tap or Apply commits, Cancel or Back drops the preview. The desktop's
//! hover preview is unchanged.

use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(120);
const PHONE: [f32; 2] = [412.0, 915.0];

fn phone() -> Headless {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, PHONE, 2.625);
    h.app.ui.touch_mode = true;
    let r = h.request("ui.set", json!({"view": "detail"}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    let r = h.request("ui.clickWidget", json!({"id": "toolbar:presets"}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    assert!(h.app.ui.sheet.open && h.app.ui.presets, "the Presets sheet is up");
    // pulled up by its handle, so several rows show beside the preview's slider and buttons
    h.app.ui.sheet.fraction = 0.85;
    frames(&mut h);
    h
}

/// The preset rows whose centre is inside the sheet: (widget id, preset id, display name).
fn rows(h: &Headless) -> Vec<(String, String, String)> {
    let sheet = rect(h, "panel:sheet");
    let mut found: Vec<_> = h
        .app
        .widgets
        .iter()
        .filter(|(w, r)| w.starts_with("preset:") && sheet.contains(r.center()))
        .filter_map(|(w, r)| {
            let id = w.strip_prefix("preset:")?;
            let p = h.app.session.presets.iter().find(|p| p.id == id)?;
            Some((r.top(), w.clone(), id.to_owned(), crate::i18n::builtin_label(&p.name, p.builtin).to_owned()))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, w, i, n)| (w, i, n)).collect()
}

fn frames(h: &mut Headless) {
    h.app.ui.touch_mode = true;
    h.settle(T);
    h.step();
    h.step();
}

fn rect(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

/// A finger tap on the centre of widget `id`, as `ui.touch` sends it (fractions of the image rect).
fn tap(h: &mut Headless, id: &str) {
    let c = rect(h, id).center();
    let img = h.app.image_rect.expect("a photo is shown");
    let (x, y) = (f64::from((c.x - img.left()) / img.width()), f64::from((c.y - img.top()) / img.height()));
    let ev = |phase: &str, t: f64| json!({"id": 1, "phase": phase, "x": x, "y": y, "t": t});
    let r = h.request("ui.touch", json!({"events": [ev("start", 0.0), ev("end", 0.05)]}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(h);
}

fn label(h: &Headless) -> Option<String> {
    h.app.hover_preview.as_ref().map(|p| p.label.clone())
}

fn history_len(h: &Headless) -> usize {
    let id = h.app.session.active().expect("active");
    h.app.session.catalog.photo(id).map(|p| p.history.len()).expect("photo")
}

#[test]
fn tap_previews_without_committing() {
    let mut h = phone();
    let before = history_len(&h);
    let (w1, id1, name1) = rows(&h).swap_remove(0);
    tap(&mut h, &w1);
    assert_eq!(label(&h), Some(format!("Preset: {name1}")));
    assert_eq!(h.app.ui.preset_previewing.as_deref(), Some(id1.as_str()));
    assert_eq!(history_len(&h), before, "no history entry");
    assert!(h.app.session.undo.is_empty());
    assert!(h.app.ui.sheet.open, "the sheet stays open");
    // another tap moves the preview to that preset
    // (the amount slider and Apply took room: the rows moved down)
    let (w2, id2, _) = rows(&h).into_iter().rev().find(|r| r.1 != id1).expect("another row");
    tap(&mut h, &w2);
    assert_eq!(h.app.ui.preset_previewing.as_deref(), Some(id2.as_str()));
    assert!(h.app.session.undo.is_empty() && h.app.ui.sheet.open);
}

#[test]
fn second_tap_applies() {
    let mut h = phone();
    let (w1, id1, name1) = rows(&h).swap_remove(0);
    tap(&mut h, &w1);
    assert!(h.app.session.undo.is_empty());
    let (w1, _, _) = rows(&h).into_iter().find(|r| r.1 == id1).expect("the previewed row");
    tap(&mut h, &w1);
    assert_eq!(h.app.ui.preset_previewing, None);
    assert_eq!(h.app.session.undo.len(), 1, "one undo step");
    assert!(h.app.session.undo.last().is_some_and(|e| e.label == format!("Preset: {name1}")));
    assert!(h.app.ui.sheet.open, "the sheet stays open for the next try");
    // the Apply button commits too, at the amount slider's value (typed into the slider)
    let (w2, id2, _) = rows(&h).into_iter().rev().find(|r| r.1 != id1).expect("another row");
    tap(&mut h, &w2);
    let active = h.app.session.active().expect("active");
    let before = (*h.app.session.develop_of(active).expect("develop")).clone();
    let full = h.app.hover_preview.as_ref().expect("preview").settings.clone();
    let r = h.request("ui.clickWidget", json!({"id": "sliderValue:presetAmount"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.request("ui.text", json!({"text": "50"}), T);
    h.request("ui.key", json!({"key": "Enter"}), T);
    frames(&mut h);
    assert!((h.app.ui.preset_preview_amount - 50.0).abs() < 1e-9, "{}", h.app.ui.preset_preview_amount);
    let pr = h.app.session.presets.iter().find(|p| p.id == id2).expect("preset").clone();
    let half = pr.apply(&before, 0.5);
    let shown = h.app.hover_preview.as_ref().expect("preview").settings.clone();
    assert_eq!(shown, half, "the preview follows the amount");
    assert_ne!(shown, full, "and differs from the 100% look");
    let r = h.request("ui.clickWidget", json!({"id": "presetApply"}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    assert_eq!(h.app.session.undo.len(), 2);
    assert_eq!(h.app.ui.preset_previewing, None);
    assert_eq!(*h.app.session.develop_of(active).expect("develop"), half, "committed at 50%, not 100%");
}

#[test]
fn back_cancels_preview() {
    let mut h = phone();
    let (w1, _, _) = rows(&h).swap_remove(0);
    tap(&mut h, &w1);
    assert!(h.app.ui.preset_previewing.is_some());
    assert_eq!(h.app.run("view.back", json!({})), Ok(Value::Null));
    frames(&mut h);
    assert_eq!(h.app.ui.preset_previewing, None);
    assert!(h.app.hover_preview.is_none());
    assert!(h.app.ui.sheet.open, "Back dropped the preview, not the sheet");
    assert!(h.app.session.undo.is_empty());
    // Cancel does the same
    let (w1, _, _) = rows(&h).swap_remove(0);
    tap(&mut h, &w1);
    let r = h.request("ui.clickWidget", json!({"id": "presetCancel"}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    assert_eq!(h.app.ui.preset_previewing, None);
    // the next Back closes the sheet
    assert_eq!(h.app.run("view.back", json!({})), Ok(Value::Null));
    frames(&mut h);
    assert!(!h.app.ui.sheet.open);
}

#[test]
fn desktop_hover_preview_unchanged() {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.request("ui.set", json!({"view": "detail"}), T);
    h.request("engine.execute", json!({"command": "panel.presets"}), T);
    h.settle(T);
    h.request("ui.hoverWidget", json!({"id": "preset:lc.warm-glow"}), T);
    h.settle(T);
    h.step();
    assert_eq!(label(&h).as_deref(), Some("Preset: Warm Glow"));
    assert!(h.app.ui.preset_previewing.is_none(), "hover never sets a tap preview");
    assert!(h.app.session.undo.is_empty());
    // a click still applies at once
    let r = h.request("ui.clickWidget", json!({"id": "preset:lc.warm-glow"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    assert_eq!(h.app.session.undo.len(), 1);
}

/// A preview left behind by closing the sheet must not swallow the next Back or come back later.
#[test]
fn closed_sheet_drops_preview() {
    let mut h = phone();
    let (w1, _, _) = rows(&h).swap_remove(0);
    tap(&mut h, &w1);
    assert!(h.app.ui.preset_previewing.is_some());
    // the sheet closes by its toolbar button, not by Back
    let r = h.request("ui.clickWidget", json!({"id": "toolbar:presets"}), T);
    assert_eq!(r["ok"], true, "{r}");
    frames(&mut h);
    assert!(!h.app.ui.sheet.open);
    assert_eq!(h.app.ui.preset_previewing, None, "no preview without the sheet");
    // Back does its next job (leaves the photo), instead of being swallowed
    assert_eq!(h.app.ui.view, crate::state::ViewMode::Detail);
    assert_eq!(h.app.run("view.back", json!({})), Ok(Value::Null));
    frames(&mut h);
    assert_eq!(h.app.ui.view, crate::state::ViewMode::PhotoGrid);
}

/// What is drawn over the sheet closes before the preview is dropped.
#[test]
fn back_closes_dialog_before_preview() {
    let mut h = phone();
    let (w1, _, _) = rows(&h).swap_remove(0);
    tap(&mut h, &w1);
    h.app.ui.lights_out = crate::state::LightsOut::Dim;
    assert_eq!(h.app.run("view.back", json!({})), Ok(Value::Null));
    assert_eq!(h.app.ui.lights_out, crate::state::LightsOut::On, "lights-out went first");
    assert!(h.app.ui.preset_previewing.is_some(), "the preview waits");
    assert_eq!(h.app.run("view.back", json!({})), Ok(Value::Null));
    assert_eq!(h.app.ui.preset_previewing, None);
    assert!(h.app.ui.sheet.open);
}
