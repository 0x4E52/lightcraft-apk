//! Return presses OK in Copy Settings (⇧⌘C) and Paste Selected Settings (⇧⌘V); Esc cancels (#735).

use std::time::Duration;

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::headless::Headless;
use crate::state::Dialog;

const SETTLE: Duration = Duration::from_secs(120);
const T: Duration = Duration::from_secs(10);

/// The demo library with an edited first photo selected; returns the first two photos' ids.
fn edited() -> (Headless, [u64; 2]) {
    let services = crate::Services { png: None, ..Default::default() };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    let mut h = Headless::new(app, [1200.0, 900.0], 1.0);
    let ids: Vec<u64> = h.app.session.visible_cloned().iter().take(2).map(|p| p.0).collect();
    let ids = [ids[0], ids[1]];
    exec(&mut h, "library.select", json!({"ids": [ids[0]]}));
    exec(&mut h, "develop.set", json!({"values": {"light.exposure": 1.0, "color.vibrance": 30}}));
    (h, ids)
}

fn exec(h: &mut Headless, command: &str, params: Value) {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
}

fn key(h: &mut Headless, k: Value) {
    let r = h.request("ui.key", k.clone(), T);
    assert_eq!(r["ok"], true, "{k}: {r}");
    h.settle(SETTLE);
}

fn copy_dialog(h: &mut Headless) {
    key(h, json!({"key": "C", "cmd": true, "shift": true}));
    assert!(matches!(h.app.ui.dialog, Some(Dialog::CopySettings { .. })), "dialog not open: {:?}", h.app.ui.dialog);
}

/// Copies light and colour from the first photo, selects the second and opens ⇧⌘V.
fn paste_dialog(h: &mut Headless, ids: [u64; 2]) {
    exec(h, "develop.copy", json!({"groups": ["light", "color"]}));
    exec(h, "library.select", json!({"ids": [ids[1]]}));
    key(h, json!({"key": "V", "cmd": true, "shift": true}));
    assert!(matches!(h.app.ui.dialog, Some(Dialog::PasteSettings { .. })), "dialog not open: {:?}", h.app.ui.dialog);
}

fn exposure(h: &Headless, id: u64) -> f64 {
    h.app.session.develop_of(PhotoId(id)).map(|d| d.light.exposure).unwrap_or_default()
}

#[test]
fn return_copies_the_chosen_settings() {
    let (mut h, _) = edited();
    copy_dialog(&mut h);
    assert!(h.app.session.clipboard.is_none());
    key(&mut h, json!({"key": "Enter"}));
    assert!(h.app.ui.dialog.is_none(), "Return closes the dialog");
    assert!(h.app.session.clipboard.is_some(), "Return copies the settings");
}

#[test]
fn escape_closes_copy_settings_without_copying() {
    let (mut h, _) = edited();
    copy_dialog(&mut h);
    key(&mut h, json!({"key": "Escape"}));
    assert!(h.app.ui.dialog.is_none(), "Esc closes the dialog");
    assert!(h.app.session.clipboard.is_none(), "Esc copies nothing");
}

#[test]
fn return_pastes_into_the_selected_photos() {
    let (mut h, ids) = edited();
    paste_dialog(&mut h, ids);
    key(&mut h, json!({"key": "Enter"}));
    assert!(h.app.ui.dialog.is_none(), "Return closes the dialog");
    assert_eq!(exposure(&h, ids[1]), 1.0, "Return pastes the settings");
}

#[test]
fn escape_closes_paste_settings_without_pasting() {
    let (mut h, ids) = edited();
    paste_dialog(&mut h, ids);
    key(&mut h, json!({"key": "Escape"}));
    assert!(h.app.ui.dialog.is_none(), "Esc closes the dialog");
    assert_eq!(exposure(&h, ids[1]), 0.0, "Esc pastes nothing");
}

/// Paste Selected Settings with Masks offers "Recompute subject and sky on each photo": checked, the paste is
/// followed by Update AI Masks on the photos pasted into, in the background, and its outcome is a toast. Without the
/// SAM 3 model here, the pasted Describe selection is kept and the toast says so; the heuristic Subject mask follows
/// the photo by itself.
#[test]
fn paste_with_recompute_option() {
    let (mut h, ids) = edited();
    let seg = serde_json::to_value(lightcraft_develop::SegMask::from_logits(4, &[1.0; 16])).unwrap();
    exec(&mut h, "mask.add", json!({"kind": "prompt", "text": "subject", "seg": seg}));
    exec(&mut h, "mask.add", json!({"kind": "subject"}));
    exec(&mut h, "develop.copy", json!({"groups": ["masks"]}));
    exec(&mut h, "library.select", json!({"ids": [ids[1]]}));
    key(&mut h, json!({"key": "V", "cmd": true, "shift": true}));
    assert!(matches!(h.app.ui.dialog, Some(Dialog::PasteSettings { recompute_ai: false, .. })), "{:?}", h.app.ui.dialog);
    let r = h.request("ui.clickWidget", json!({"id": "check:pasteRecompute"}), T);
    assert_eq!(r["ok"], true, "the option is there when Masks is pasted: {r}");
    h.settle(SETTLE);
    assert!(matches!(h.app.ui.dialog, Some(Dialog::PasteSettings { recompute_ai: true, .. })), "{:?}", h.app.ui.dialog);
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    let t = std::time::Instant::now();
    while h.app.session.batch_ai.running() || h.app.session.batch_ai.has_outcome() {
        assert!(t.elapsed() < SETTLE, "Update AI Masks never ended");
        h.step();
    }
    let masks = &h.app.session.develop_of(PhotoId(ids[1])).unwrap().masks;
    assert_eq!(masks.len(), 2, "the masks were pasted");
    let toast = h.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.contains("Update AI Masks") && toast.contains("kept their AI selections"), "{toast}");
    // without Masks in the paste there is nothing to recompute: no option
    exec(&mut h, "develop.copy", json!({"groups": ["light"]}));
    key(&mut h, json!({"key": "V", "cmd": true, "shift": true}));
    let r = h.request("ui.clickWidget", json!({"id": "check:pasteRecompute"}), T);
    assert_ne!(r["ok"], true, "{r}");
}

/// APK-11b's memory gate holds for Update AI Masks too: a caller (control channel, `mcp --connect`) can't switch it off
/// with an empty `samUnavailable`, nor make the UI thread wait with `background: false`.
#[test]
fn update_ai_masks_keeps_the_memory_gate() {
    let services = crate::Services { png: None, min_ram_for_sam: Some(u64::MAX), ..Default::default() };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    let mut h = Headless::new(app, [1200.0, 900.0], 1.0);
    let id = h.app.session.visible_cloned()[0].0;
    exec(&mut h, "library.select", json!({"ids": [id]}));
    let seg = serde_json::to_value(lightcraft_develop::SegMask::from_logits(4, &[1.0; 16])).unwrap();
    // the app refuses a precomputed `seg` on this device like the tiles: the session puts the selection there, as a
    // paste from a phone with the memory (or a desktop catalog) would
    h.app.session.execute("mask.add", &json!({"kind": "prompt", "text": "subject", "seg": seg})).unwrap();
    h.app.session.execute("mask.add", &json!({"kind": "subject"})).unwrap();
    let before = h.app.session.develop_of(PhotoId(id)).unwrap().masks.clone();
    assert_eq!(before.len(), 2);
    let r = h.request("engine.execute", json!({"command": "mask.updateAi", "params": {"ids": [id], "samUnavailable": "", "background": false}}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["started"], true, "run in the background all the same: {r}");
    let t = std::time::Instant::now();
    while h.app.session.batch_ai.running() || h.app.session.batch_ai.has_outcome() {
        assert!(t.elapsed() < SETTLE, "Update AI Masks never ended");
        h.step();
    }
    assert_eq!(h.app.session.develop_of(PhotoId(id)).unwrap().masks, before, "the selection is kept");
    let toast = h.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.contains("1 kept their AI selections") && toast.contains("Needs a device with 8 GB of memory"), "{toast}");
    assert!(toast.contains("1 live masks need no update"), "the Subject mask is counted as live: {toast}");
}
