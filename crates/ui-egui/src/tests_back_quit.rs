//! Back and Quit where the host has one event loop and no Quit (Android, APK-17): Back (Escape)
//! closes what is open and leaves what was entered first, exactly as on the desktop, and only at
//! the root asks the host to move the app to the background; Quit is refused everywhere it can be
//! reached, and nothing offers a Quit that would do nothing.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use lightcraft_engine::activity::Cancel;
use serde_json::{Value, json};

use crate::headless::Headless;
use crate::panels::library_problem::LibraryProblem;
use crate::state::{Dialog, LightsOut, ViewMode};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

/// Services with a fake Back-to-background that counts its calls (and, as the phone, no Quit).
fn phone(calls: &Rc<Cell<u32>>) -> Services {
    let seen = calls.clone();
    Services {
        png: None,
        move_to_back: Some(Box::new(move |_| {
            seen.set(seen.get() + 1);
            Ok(Value::Null)
        })),
        quit_disabled: true,
        ..Default::default()
    }
}

fn demo(services: Services, ui: Value) -> Headless {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.settings.gpu = false;
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    let r = h.request("ui.set", ui, T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(120));
    h
}

fn key(h: &mut Headless, k: &str, extra: Value) {
    let mut p = json!({"key": k});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    let r = h.request("ui.key", p, T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
}

fn escape(h: &mut Headless) {
    key(h, "Escape", json!({}));
}

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn popup_open(h: &Headless) -> bool {
    egui::Popup::is_any_open(&h.view.ctx)
}

/// At the grid nothing is left to leave: Escape (Android's Back) asks the host, every time.
#[test]
fn back_at_root_moves_to_back() {
    let calls = Rc::new(Cell::new(0));
    let mut h = demo(phone(&calls), json!({"view": "photoGrid"}));
    escape(&mut h);
    assert_eq!(calls.get(), 1);
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid, "the app stays where it was");
    escape(&mut h);
    assert_eq!(calls.get(), 2);
    // the command itself, as an agent runs it
    assert_eq!(h.app.run("view.back", json!({})), Ok(Value::Null));
    assert_eq!(calls.get(), 3);
    // the host's refusal is the command's error
    h.app.services.move_to_back = Some(Box::new(|_| Err("no Activity".into())));
    assert_eq!(h.app.run("view.back", json!({})), Err("no Activity".to_owned()));
}

/// Every earlier step of the cascade still comes first, one per Back; the host is asked only once
/// they are all used up. Without the service (the desktop) Back at the root does nothing.
#[test]
fn back_order_unchanged_on_desktop() {
    let calls = Rc::new(Cell::new(0));
    let mut h = demo(phone(&calls), json!({"view": "detail", "right": "edit"}));
    h.app.ui.dialog = Some(Dialog::Settings { tab: "general".into() });
    h.app.ui.lights_out = LightsOut::Dim;
    h.app.ui.fullscreen = true;
    h.step();
    h.app.run("view.back", json!({})).unwrap();
    assert!(h.app.ui.dialog.is_none() && h.app.ui.lights_out == LightsOut::Dim, "the dialog first");
    h.app.run("view.back", json!({})).unwrap();
    assert!(h.app.ui.lights_out == LightsOut::On && h.app.ui.fullscreen, "then the lights");
    h.app.run("view.back", json!({})).unwrap();
    assert!(!h.app.ui.fullscreen && h.app.ui.view == ViewMode::Detail, "then full screen");
    h.app.ui.view = ViewMode::Compare;
    h.app.run("view.back", json!({})).unwrap();
    assert_eq!(h.app.ui.view, ViewMode::Detail, "Compare goes back to the loupe");
    h.app.run("view.back", json!({})).unwrap();
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid, "the loupe goes back to the grid");
    assert_eq!(calls.get(), 0, "the host was not asked while anything was left");
    h.app.run("view.back", json!({})).unwrap();
    assert_eq!(calls.get(), 1);

    // the desktop: the same steps, and the root is the end
    let mut desk = demo(Services { png: None, ..Default::default() }, json!({"view": "detail", "right": "edit"}));
    escape(&mut desk);
    assert_eq!(desk.app.ui.view, ViewMode::PhotoGrid);
    let before = desk.app.ui.clone();
    escape(&mut desk);
    assert_eq!(desk.app.run("view.back", json!({})), Ok(Value::Null));
    assert_eq!(desk.app.ui.view, before.view);
    assert!(!desk.quit_requested());
}

/// Back on an open context menu closes the menu and nothing else: the view stays and the host is
/// not asked; the next Back reaches the cascade.
#[test]
fn back_closes_open_context_menu_first() {
    let calls = Rc::new(Cell::new(0));
    let mut h = demo(phone(&calls), json!({"view": "photoGrid"}));
    let id = *h.app.session.visible().first().unwrap();
    let tile = h.app.widgets.iter().find(|(w, _)| *w == format!("thumb:{}", id.0)).map(|(_, r)| *r).unwrap();
    let c = tile.center();
    let r = h.request("ui.click", json!({"x": c.x, "y": c.y, "button": "right"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    assert!(popup_open(&h), "the photo's context menu is open");
    escape(&mut h);
    assert!(!popup_open(&h), "Back closed the menu");
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid);
    assert_eq!(calls.get(), 0, "the menu took Back");
    escape(&mut h);
    assert_eq!(calls.get(), 1);
}

/// Where the host can't quit, Quit is refused by the command, Cmd+Q, the menus and the control
/// channel, and no quit prompt opens, however the app is asked to close.
#[test]
fn quit_refused_when_disabled() {
    let calls = Rc::new(Cell::new(0));
    let mut h = demo(phone(&calls), json!({"view": "photoGrid"}));
    // the command (menus, agents)
    assert!(!crate::menus::ui_enabled(&h.app, "app.quit"));
    assert_eq!(h.app.run("app.quit", json!({})), Err(crate::menus::QUIT_DISABLED.to_owned()));
    assert!(!h.app.ui.quit);
    // the menu bar has no Quit, and the File menu still ends tidily
    let bar = crate::menubar::menu_bar(&h.app);
    let file = &bar.iter().find(|(t, _)| t == "File").unwrap().1;
    assert!(!file.iter().any(|n| matches!(n, crate::menubar::MenuNode::Item { id, .. } if id == "app.quit")));
    assert_ne!(file.last(), Some(&crate::menubar::MenuNode::Separator));
    let listed = h.request("engine.commands", json!({}), T);
    let quit = listed["result"].as_array().and_then(|v| v.iter().find(|c| c["id"] == "app.quit")).cloned().unwrap();
    assert_eq!(quit["enabled"], false, "{quit}");
    // Cmd+Q
    key(&mut h, "Q", json!({"cmd": true}));
    // (egui's own Cmd+Q closes the window unless the app turned it off)
    assert!(!h.quit_requested() && !h.app.ui.quit);
    // the control channel
    let r = h.request("app.quit", json!({}), T);
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap_or_default().contains("Quit isn't available"), "{r}");
    h.step();
    assert!(!h.quit_requested());

    // the quit-prompt path: a running task and a close request open no prompt
    let g = h.app.session.activity.start("export", "Exporting", Cancel::Yes);
    assert!(!crate::panels::notices::may_close(&mut h.app));
    assert!(h.app.quit_prompt.is_none());
    h.step();
    assert!(!has(&h, "button:quitAnyway") && !g.is_cancelled());
    // and a stray quit request (from an older path) closes nothing
    h.app.ui.quit = true;
    h.step();
    assert!(!h.quit_requested());
    assert_eq!(calls.get(), 0, "Quit is not Back");

    // the desktop quits as before
    let mut desk = demo(Services { png: None, ..Default::default() }, json!({"view": "photoGrid"}));
    assert!(crate::menus::ui_enabled(&desk.app, "app.quit"));
    assert_eq!(desk.app.run("app.quit", json!({})), Ok(Value::Null));
    desk.step();
    assert!(desk.quit_requested());
}

/// The library problem window offers no Quit where the host can't quit (a corrupt catalog or a
/// full disk never shows a button that does nothing); its other choices stay.
#[test]
fn library_problem_has_no_quit_when_disabled() {
    let calls = Rc::new(Cell::new(0));
    let mut app = LightcraftApp::new(lightcraft_engine::Session::new(), phone(&calls));
    app.library_problem = Some(LibraryProblem::new("/data/user/0/x/files/library", "catalog damaged"));
    let mut h = Headless::new(app, [1300.0, 900.0], 1.0);
    h.step();
    h.step();
    assert!(has(&h, "button:libraryRetry") && has(&h, "button:libraryTemporary"));
    assert!(!has(&h, "button:libraryQuit"));
    let r = h.request("ui.clickWidget", json!({"id": "button:libraryQuit"}), T);
    assert_eq!(r["ok"], false, "{r}");
    h.step();
    assert!(!h.quit_requested() && h.app.library_problem.is_some());

    // the desktop keeps its Quit
    let mut app = LightcraftApp::new(lightcraft_engine::Session::new(), Services { png: None, ..Default::default() });
    app.library_problem = Some(LibraryProblem::new("/home/me/Pictures/LightCraft", "catalog damaged"));
    let mut desk = Headless::new(app, [1300.0, 900.0], 1.0);
    desk.step();
    desk.step();
    assert!(has(&desk, "button:libraryQuit"));
}
