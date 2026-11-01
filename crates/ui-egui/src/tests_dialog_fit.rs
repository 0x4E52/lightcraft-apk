//! Issue #781: a dialog taller than the window keeps its title and buttons on screen; its options
//! scroll. On a 1366×768 screen the Export dialog ran off both ends and couldn't be confirmed.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn demo(size: [f32; 2]) -> Headless {
    let services = Services { png: None, write_shared: Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, size, 1.0);
    // let the first renders finish (a render still running at exit can crash in GPU teardown)
    h.settle(T);
    h
}

fn rect(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn open(h: &mut Headless, command: &str) {
    let r = h.request("engine.execute", json!({"command": command, "params": {}}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    assert!(h.app.ui.dialog.is_some(), "{command} opens a dialog");
    h.settle(T);
}

/// The dialog lies inside the window, and so do its buttons (`button:dialogCancel` when it has one).
fn assert_fits(h: &Headless, what: &str) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, h.size);
    let dialog = rect(h, "dialog:window");
    assert!(screen.contains_rect(dialog), "{what}: dialog {dialog:?} runs off the {screen:?} window");
    let mut buttons = vec!["button:dialogOk"];
    if h.app.widgets.iter().any(|(w, _)| w == "button:dialogCancel") {
        buttons.push("button:dialogCancel");
    }
    for b in buttons {
        let r = rect(h, b);
        assert!(dialog.contains_rect(r), "{what}: {b} {r:?} is outside the dialog {dialog:?}");
    }
}

/// The Export dialog in a window of `size`, set to write small JPEGs to a made-up folder.
fn export_dialog(size: [f32; 2]) -> Headless {
    let mut h = demo(size);
    open(&mut h, "dialog.export");
    if let Some(crate::state::Dialog::Export { full_size, resize, dir, .. }) = &mut h.app.ui.dialog {
        *full_size = false;
        *resize = lightcraft_engine::export::Resize::long_edge(64);
        *dir = "/lc-test-out".into();
    }
    h.settle(T);
    h
}

// Scenario: on a 1366×768 screen (and a shorter window) the Export dialog's buttons are on screen and work
#[test]
fn export_dialog_fits_a_short_window_and_its_button_exports() {
    for size in [[1366.0, 690.0], [1366.0, 520.0]] {
        let mut h = export_dialog(size);
        assert_fits(&h, &format!("Export at {size:?}"));
        let r = h.request("ui.clickWidget", json!({"id": "button:dialogOk"}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.step();
        h.step();
        assert!(h.app.ui.dialog.is_none(), "Export closed the dialog at {size:?}");
        assert!(h.app.export.is_some() || h.app.last_export_result.is_some(), "Export started at {size:?}");
        assert!(h.step_until(Duration::from_secs(60), |h| h.app.export.is_none()));
    }
}

// Scenario: Cancel is on screen too, and closes the dialog
#[test]
fn export_dialog_cancel_is_on_screen_in_a_short_window() {
    let mut h = export_dialog([1366.0, 690.0]);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, h.size);
    assert!(screen.contains_rect(rect(&h, "button:dialogCancel")));
    h.request("ui.clickWidget", json!({"id": "button:dialogCancel"}), T);
    h.step();
    h.step();
    assert!(h.app.ui.dialog.is_none());
    assert!(h.app.export.is_none() && h.app.last_export_result.is_none(), "nothing exported");
}

// Scenario: a window with room for it shows the Export dialog as before (no scrolling, same size)
#[test]
fn export_dialog_is_unchanged_in_a_tall_window() {
    let h = export_dialog([1600.0, 1000.0]);
    assert_fits(&h, "Export at 1600×1000");
    let dialog = rect(&h, "dialog:window");
    // (846 tall as before #781: the whole dialog, nothing scrolled away, plus the Export to row
    // since #956; 615 wide since the format row gained JPEG XL, #780. On macOS the dialog has one
    // more row, "Add to Apple Photos" (#236), so it is 30 taller there. 8 more since its footer's
    // buttons are the button component's medium size, 30 points, as the welcome's and every dialog's)
    let height = if cfg!(target_os = "macos") { 916.0 } else { 886.0 };
    assert!((dialog.height() - height).abs() <= 1.0, "{dialog:?}");
    assert!((dialog.width() - 615.2).abs() <= 1.0, "{dialog:?}");
}

// Scenario: other long dialogs fit a short window too, with their buttons on screen
#[test]
fn long_dialogs_fit_a_short_window() {
    let mut h = demo([1000.0, 320.0]);
    for command in [
        "dialog.copySettings",
        "dialog.pasteSettings",
        "dialog.createPreset",
        "dialog.contactSheet",
        "app.settings",
        "app.shortcuts",
        "app.whatsNew",
        "app.about",
        "app.systemInfo",
        "dialog.smartAlbum",
    ] {
        open(&mut h, command);
        assert_fits(&h, command);
        h.app.ui.dialog = None;
        h.settle(T);
    }
}

// Scenario: a list that scrolls on its own (the shortcuts) gets shorter instead of scrolling inside a scrolling dialog
#[test]
fn a_dialog_list_shrinks_to_fit_a_short_window() {
    let mut h = demo([1000.0, 320.0]);
    open(&mut h, "app.shortcuts");
    let shrink = h.view.ctx.data(|d| d.get_temp::<f32>(egui::Id::new("dialog-list-shrink"))).unwrap_or(0.0);
    assert!(shrink > 0.0, "the list gave up room");
    let dialog = rect(&h, "dialog:window");
    assert!(dialog.height() <= 320.0, "{dialog:?}");
}

/// Settings ▸ Performance: the GPU status line above the controls stays one line whatever it says
/// (without a driver it is a long wgpu message, e.g. on the FreeBSD CI), so the controls below it
/// don't move when the process-wide GPU state changes between two frames (a click aimed at
/// "Memory for caches" then landed on the gap above it).
#[test]
fn gpu_status_never_moves_the_performance_controls() {
    let mut h = demo([1300.0, 820.0]);
    let r = h.request("ui.key", json!({"key": ",", "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    let r = h.request("ui.clickWidget", json!({"id": "button:settingsTab-performance"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let mut seen = vec![];
    for gpu in [true, false, true] {
        // the setting is applied process-wide at the next frame (`app.gpu`), as other tests do
        h.app.ui.settings.gpu = gpu;
        h.step();
        h.step();
        seen.push((gpu, rect(&h, "button:settingsMemory-2"), lightcraft_engine::gpu::unavailable_reason()));
    }
    let first = seen[0].1;
    assert!(seen.iter().all(|(_, r, _)| *r == first), "the memory buttons moved: {seen:?}");
}

/// A phone held upright and on its side, in points, and its scale (1080 × 2400 px at 420 dpi).
const PHONE: [f32; 2] = [412.0, 915.0];
const PHONE_SCALE: f32 = 2.625;

/// The demo library on a phone: touch, so the touch density's bigger hit sizes are laid out too.
fn phone(size: [f32; 2]) -> Headless {
    let services = Services { png: None, write_shared: Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, size, PHONE_SCALE);
    // the status bar and the gesture bar, as the Android host reports them (APK-31)
    h.safe_area = Some(egui::epaint::MarginF32 { left: 0.0, right: 0.0, top: 28.0, bottom: 24.0 });
    touch_frames(&mut h);
    h
}

/// Settle with touch mode set again (a control request's own pointer activity clears it).
fn touch_frames(h: &mut Headless) {
    h.app.ui.touch_mode = true;
    h.settle(T);
    h.step();
    h.step();
}

/// Nothing a dialog lays out runs past the content rect (the window less its insets), and its
/// buttons lie inside it; what doesn't goes to `bad`.
fn check_fits_content(h: &Headless, what: &str, bad: &mut Vec<String>) {
    let content = h.view.ctx.content_rect();
    let dialog = rect(h, "dialog:window");
    if !content.expand(0.5).contains_rect(dialog) {
        bad.push(format!("{what}: dialog {dialog:?} runs off the content rect {content:?}"));
    }
    for b in ["button:dialogOk", "button:dialogCancel"] {
        if let Some((_, r)) = h.app.widgets.iter().find(|(w, _)| w == b)
            && !dialog.expand(0.5).contains_rect(*r)
        {
            bad.push(format!("{what}: {b} {r:?} is outside the dialog {dialog:?}"));
        }
    }
}

// Scenario: on a phone, upright and on its side, every dialog (and each About tab) fits the screen
#[test]
fn every_dialog_fits_a_phone() {
    let mut bad = vec![];
    for size in [PHONE, [PHONE[1], PHONE[0]]] {
        let mut h = phone(size);
        for command in [
            "dialog.export",
            "dialog.copySettings",
            "dialog.pasteSettings",
            "dialog.createPreset",
            "dialog.contactSheet",
            "app.settings",
            "app.shortcuts",
            "app.whatsNew",
            "app.systemInfo",
            "dialog.smartAlbum",
        ] {
            open(&mut h, command);
            touch_frames(&mut h);
            check_fits_content(&h, &format!("{command} at {size:?}"), &mut bad);
            h.app.ui.dialog = None;
            touch_frames(&mut h);
        }
        open(&mut h, "app.about");
        for (id, _) in crate::panels::dialogs::ABOUT_TABS {
            let r = h.request("ui.clickWidget", json!({"id": format!("button:aboutTab-{id}")}), T);
            assert_eq!(r["ok"], true, "{id}: {r}");
            touch_frames(&mut h);
            check_fits_content(&h, &format!("About ▸ {id} at {size:?}"), &mut bad);
        }
        // the Settings tabs too: its tab row wraps instead of widening the window
        h.app.ui.dialog = None;
        touch_frames(&mut h);
        open(&mut h, "app.settings");
        let tabs: Vec<String> = h.app.widgets.iter().map(|(w, _)| w.to_string()).filter(|w| w.starts_with("button:settingsTab-")).collect();
        assert!(!tabs.is_empty(), "Settings has tabs");
        for tab in tabs {
            let r = h.request("ui.clickWidget", json!({"id": tab}), T);
            assert_eq!(r["ok"], true, "{tab}: {r}");
            touch_frames(&mut h);
            check_fits_content(&h, &format!("Settings ▸ {tab} at {size:?}"), &mut bad);
        }
        h.app.ui.dialog = None;
        h.settle(T);
    }
    assert!(bad.is_empty(), "dialogs that don't fit a phone:\n{}", bad.join("\n"));
}

// Scenario: below the phone breakpoint a dialog takes the content width less its margins
#[test]
fn a_dialog_is_full_width_on_a_phone() {
    let mut h = phone(PHONE);
    open(&mut h, "app.about");
    touch_frames(&mut h);
    let content = h.view.ctx.content_rect();
    let dialog = rect(&h, "dialog:window");
    let want = content.width() - 2.0 * 12.0;
    assert!((dialog.width() - want).abs() <= 1.0, "full width less the margins: {dialog:?} in {content:?}");
    assert!((dialog.center().x - content.center().x).abs() <= 1.0, "centred: {dialog:?} in {content:?}");
}
