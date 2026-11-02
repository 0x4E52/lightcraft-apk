//! APK-53: a panic the previous run recorded opens a small window at the next start, with Copy
//! Report, Share Report and Dismiss; a clean exit shows nothing; nothing is sent anywhere.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::panels::last_crash::{MAX_REPORT_BYTES, PreviousCrash, take_previous};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-ui-lastcrash-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn headless(services: Services) -> Headless {
    let mut h = Headless::new(LightcraftApp::new(lightcraft_engine::Session::with_demo(), services), [1000.0, 700.0], 1.0);
    h.step();
    h.step();
    h
}

const PANIC: &str = "1760000000 [worker] panic at crates/x/src/lib.rs:3: boom";

#[test]
fn last_crash_window_after_panic_log() {
    let dir = temp_dir("window");
    let log = dir.join("panic.log");
    std::fs::write(&log, format!("{PANIC}\n")).expect("write panic log");
    // the host reads (and empties) the panic log before it installs the hook again
    let panics = take_previous(&log).expect("a panic was recorded");
    assert!(!log.exists(), "the crash is offered once");
    let shared: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = shared.clone();
    // like the Android host: only files in the export folder can be shared
    let docs = dir.join("Documents").to_string_lossy().into_owned();
    let allowed = docs.clone();
    let share: crate::share::ShareFn = Arc::new(move |paths: &[String], _| {
        if let Some(p) = paths.iter().find(|p| !p.starts_with(&allowed)) {
            return Err(format!("Can't share {p}: it isn't in the device's shared storage"));
        }
        sink.lock().expect("lock").extend(paths.iter().cloned());
        Ok(())
    });
    let internal_log = dir.join("lightcraft.1.log");
    std::fs::write(&internal_log, "an earlier log line\n").expect("write log");
    let mut h = headless(Services {
        previous_crash: Some(PreviousCrash { panics, log: Some(internal_log.to_string_lossy().into_owned()) }),
        share: Some(share),
        export_dir_for: Some(Box::new(move |_: crate::ExportKind| docs.clone())),
        ..Default::default()
    });
    for b in ["button:lastCrashCopy", "button:lastCrashShare", "button:lastCrashDismiss"] {
        assert!(has(&h, b), "{b}");
    }
    let r = h.request("ui.inspect", json!({}), T);
    assert!(r["result"]["lastCrash"]["panics"].as_str().is_some_and(|p| p.contains("boom")), "{r}");
    // Share Report copies the report (panics and the previous log) into the export folder and shares that
    let r = h.request("ui.clickWidget", json!({"id": "button:lastCrashShare"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    let copy = dir.join("Documents").join("LightCraft crash report.txt");
    assert_eq!(*shared.lock().expect("lock"), [copy.to_string_lossy().into_owned()], "the copy in the export folder, not the log");
    let text = std::fs::read_to_string(&copy).expect("report copy");
    assert!(text.contains("boom") && text.contains("an earlier log line"), "{text}");
    // Dismiss closes it for good
    let r = h.request("ui.clickWidget", json!({"id": "button:lastCrashDismiss"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    assert!(h.app.last_crash.is_none() && !has(&h, "button:lastCrashCopy"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_window_after_clean_exit() {
    let dir = temp_dir("clean");
    let log = dir.join("panic.log");
    // no panic log, an empty one and a blank one: nothing to offer
    assert_eq!(take_previous(&log), None);
    std::fs::write(&log, "").expect("write");
    assert_eq!(take_previous(&log), None);
    std::fs::write(&log, "\n  \n").expect("write");
    assert_eq!(take_previous(&log), None);
    let h = headless(Services::default());
    assert!(h.app.last_crash.is_none());
    assert!(!has(&h, "button:lastCrashDismiss"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_copies_panic_text() {
    let mut h = headless(Services { previous_crash: Some(PreviousCrash { panics: PANIC.into(), log: None }), ..Default::default() });
    h.settle(T);
    // no share sheet and no log: Share Report is offered disabled, Copy still works
    let r = h.request("ui.clickWidget", json!({"id": "button:lastCrashCopy", "button": "left"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
    let copied = h.app.copied.clone().unwrap_or_default();
    assert!(copied.contains(PANIC) && copied.contains("crashed on its last run"), "{copied}");
    assert!(h.app.last_crash.is_some(), "Copy keeps the window open");
}

#[test]
fn a_long_panic_log_keeps_its_end_at_a_line_start() {
    let dir = temp_dir("long");
    let log = dir.join("panic.log");
    let text: String = (0..2000).map(|i| format!("{i} [t] panic at é.rs:1: boom é é é\n")).collect();
    std::fs::write(&log, &text).expect("write");
    let kept = take_previous(&log).expect("kept");
    assert!(kept.len() <= MAX_REPORT_BYTES && kept.starts_with(|c: char| c.is_ascii_digit()) && kept.ends_with("é é é"), "{}", kept.len());
    assert!(kept.contains("1999 [t]"));
    let _ = std::fs::remove_dir_all(&dir);
}
