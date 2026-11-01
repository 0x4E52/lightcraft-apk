//! The system clipboard as a host service (Android, APK-28): every copy reaches it, every paste
//! reads it, and the UI thread never waits for its answer.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use serde_json::json;

use crate::clipboard::ClipboardFns;
use crate::headless::Headless;
use crate::state::{Dialog, ViewMode};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

/// A fake system clipboard: what was set, how often a paste was asked for, and what it answers
/// after `delay` more `take_paste` calls.
#[derive(Default)]
struct Fake {
    set: Vec<String>,
    requests: u32,
    text: String,
    delay: u32,
    /// The answer being prepared: the `take_paste` calls left before it is there.
    due: Option<u32>,
    fail: bool,
}

fn service(fake: &Rc<RefCell<Fake>>) -> ClipboardFns {
    let (set, request, take) = (fake.clone(), fake.clone(), fake.clone());
    ClipboardFns {
        set: Box::new(move |text: &str| {
            let mut f = set.borrow_mut();
            if f.fail {
                return Err("the clipboard is not available".into());
            }
            f.set.push(text.to_owned());
            Ok(())
        }),
        request_paste: Box::new(move || {
            let mut f = request.borrow_mut();
            f.requests += 1;
            f.due = Some(f.delay);
        }),
        take_paste: Box::new(move || {
            let mut f = take.borrow_mut();
            match f.due {
                Some(0) => {
                    f.due = None;
                    Some(f.text.clone())
                }
                Some(n) => {
                    f.due = Some(n - 1);
                    None
                }
                None => None,
            }
        }),
    }
}

fn demo(fake: &Rc<RefCell<Fake>>) -> Headless {
    let services = Services { png: None, clipboard: Some(service(fake)), ..Default::default() };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.settings.gpu = false;
    app.ui.view = ViewMode::PhotoGrid;
    let mut h = Headless::new(app, [1200.0, 760.0], 1.0);
    h.settle(SETTLE);
    h
}

fn click(h: &mut Headless, id: &str, button: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id, "button": button}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.settle(SETTLE);
}

/// Copy in a text field's menu puts the selection on the system clipboard, not only egui's.
#[test]
fn text_field_copy_uses_clipboard_service() {
    let fake = Rc::new(RefCell::new(Fake::default()));
    let mut h = demo(&fake);
    click(&mut h, "field:search", "left");
    let r = h.request("ui.text", json!({"text": "travel"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    for (id, button) in [("field:search", "right"), ("field:search:selectAll", "left"), ("field:search", "right"), ("field:search:copy", "left")] {
        click(&mut h, id, button);
    }
    assert_eq!(fake.borrow().set, vec!["travel".to_owned()]);
    assert_eq!(h.app.copied.as_deref(), Some("travel"), "ui.inspect still names the copy");
    // a clipboard that refuses says so, never silently
    fake.borrow_mut().fail = true;
    for (id, button) in [("field:search", "right"), ("field:search:copy", "left")] {
        click(&mut h, id, button);
    }
    let toast = h.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.contains("the clipboard is not available"), "{toast:?}");
}

/// Paste in a text field's menu reads the system clipboard through the service (egui's host
/// clipboard is not asked).
#[test]
fn paste_reads_clipboard_service() {
    let fake = Rc::new(RefCell::new(Fake { text: "wedding".into(), ..Default::default() }));
    let mut h = demo(&fake);
    h.view.clipboard = "not this".into();
    for (id, button) in [("field:search", "right"), ("field:search:paste", "left")] {
        click(&mut h, id, button);
    }
    assert_eq!(fake.borrow().requests, 1);
    assert_eq!(h.app.ui.search, "wedding");
    assert_eq!(h.app.session.filter.text, "wedding", "and the library is filtered by it");
    // an empty clipboard pastes nothing and leaves no paste waiting
    fake.borrow_mut().text.clear();
    for (id, button) in [("field:search", "right"), ("field:search:paste", "left")] {
        click(&mut h, id, button);
    }
    assert_eq!(fake.borrow().requests, 2);
    assert_eq!(h.app.ui.search, "wedding");
    assert_eq!(h.app.paste_pending, None);
}

/// A dialog's Copy button (`ctx.copy_text`) reaches the system clipboard.
#[test]
fn dialog_copy_reaches_clipboard_service() {
    let fake = Rc::new(RefCell::new(Fake::default()));
    let mut h = demo(&fake);
    h.app.ui.dialog = Some(Dialog::SystemInfo { rows: vec![("Version".into(), "1.0".into()), ("GPU".into(), "none".into())] });
    h.settle(SETTLE);
    click(&mut h, "dialog:systemInfoCopy", "left");
    assert_eq!(fake.borrow().set, vec!["Version: 1.0\nGPU: none\n".to_owned()]);
}

/// A clipboard that answers late never holds a frame up: frames go on (asked for while the
/// answer is due), and the text is pasted in the frame after it arrives.
#[test]
fn paste_never_blocks_ui_thread() {
    let fake = Rc::new(RefCell::new(Fake { text: "late".into(), delay: 6, ..Default::default() }));
    let mut h = demo(&fake);
    click(&mut h, "field:search", "right");
    let r = h.request("ui.clickWidget", json!({"id": "field:search:paste", "button": "left"}), T);
    assert_eq!(r["ok"], true, "{r}");
    // the click, then the menu's action: the request goes out at the end of a frame
    let mut frames = 0;
    while fake.borrow().requests == 0 {
        h.step();
        frames += 1;
        assert!(frames < 20, "the paste was never requested");
    }
    assert!(h.app.paste_pending.is_some(), "waiting for the answer");
    // every frame completes while the answer is due, and the app keeps asking for frames
    let mut waited = 0;
    while h.app.ui.search.is_empty() {
        h.step();
        waited += 1;
        assert!(waited < 30, "the late answer never arrived");
    }
    assert!(waited >= 6, "the answer came {waited} frames later, after the service's delay");
    assert_eq!(h.app.ui.search, "late");
    assert_eq!(h.app.paste_pending, None);
    assert_eq!(fake.borrow().requests, 1);
}
