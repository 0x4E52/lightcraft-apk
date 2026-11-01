//! The in-app file chooser (`pick_browser`, APK-24): hosts without file dialogs (Android) pick
//! from the folders they name, never outside them, and commands whose file can't live there say
//! why they are off.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::pick_browser::{self, inside_roots};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(60);

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-ui-pick-browser-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An Inbox and a library under `base`, as the Android host names them.
fn roots(base: &Path) -> Vec<(String, PathBuf)> {
    let (inbox, library) = (base.join("inbox"), base.join("library"));
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::create_dir_all(&library).unwrap();
    vec![("Inbox".into(), inbox), ("Library".into(), library)]
}

fn phone(base: &Path, services: Services) -> Headless {
    let services = Services { pick_roots: roots(base), ..services };
    let mut h = Headless::new(LightcraftApp::new(lightcraft_engine::Session::with_demo(), services), [1200.0, 800.0], 1.0);
    h.settle(SETTLE);
    h
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.settle(SETTLE);
}

/// Hand-written for this test (not a third-party preset).
const PRESET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    crs:PresetType="Normal" crs:Exposure2012="+0.30" crs:Shadows2012="+20"/></rdf:RDF></x:xmpmeta>"#;

/// File ▸ Import Presets without system dialogs: the in-app chooser opens over the host's places,
/// lists only what the command can import, and the preset chosen in the Inbox is imported.
#[test]
fn presets_import_uses_in_app_picker() {
    let base = temp_dir("presets");
    let mut h = phone(&base, Services::default());
    let inbox = base.join("inbox");
    std::fs::write(inbox.join("Soft Lift.xmp"), PRESET).unwrap();
    std::fs::write(inbox.join("notes.pdf"), b"%PDF").unwrap();
    std::fs::write(inbox.join(".hidden.xmp"), PRESET).unwrap();
    std::fs::create_dir_all(inbox.join("more")).unwrap();

    assert!(crate::menus::ui_enabled(&h.app, "file.importPresets"));
    assert_eq!(h.app.run("file.importPresets", json!({})).unwrap(), Value::Null);
    assert!(h.app.pick_browser.is_some(), "the in-app chooser is up");
    assert_eq!(h.app.pending_picks.len(), 1);
    // asking again while it is up opens no second one
    assert_eq!(h.app.run("file.importPresets", json!({})).unwrap(), Value::Null);
    assert_eq!(h.app.pending_picks.len(), 1);
    h.settle(SETTLE);
    assert!(h.app.pick_browser.as_ref().unwrap().dir().is_none(), "two places: it starts on the list of them");

    click(&mut h, "pickBrowser:root:Inbox");
    let b = h.app.pick_browser.as_ref().unwrap();
    assert_eq!(b.dir(), Some(inbox.canonicalize().unwrap().as_path()));
    assert_eq!(b.names(), ["more", "Soft Lift.xmp"], "folders first; other types and hidden files left out");
    assert!(b.answer().is_err() && !b.ready(), "nothing chosen yet");

    click(&mut h, "pickBrowser:entry:Soft Lift.xmp");
    assert!(h.app.pick_browser.as_ref().unwrap().ready(), "a file chosen: confirm is on");
    click(&mut h, "pickBrowser:confirm");
    assert!(h.app.pick_browser.is_none(), "closed with the answer");
    assert!(h.app.pending_picks.is_empty());
    assert!(h.app.session.presets.iter().any(|p| p.name == "Soft Lift" && !p.builtin), "the preset is imported");

    // Cancel (and Back) close it and import nothing
    h.app.run("file.importPresets", json!({})).unwrap();
    h.settle(SETTLE);
    click(&mut h, "pickBrowser:cancel");
    assert!(h.app.pick_browser.is_none() && h.app.pending_picks.is_empty());
    h.app.run("file.importPresets", json!({})).unwrap();
    h.app.run("view.back", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(h.app.pick_browser.is_none() && h.app.pending_picks.is_empty(), "Back closes the chooser first");
    let _ = std::fs::remove_dir_all(&base);
}

/// Nothing outside the places can be shown or answered: `..`, a path elsewhere and symbolic
/// links (also one planted in the Inbox pointing out) are refused, and listings leave links out.
#[test]
fn in_app_picker_stays_inside_roots() {
    let base = temp_dir("roots");
    let roots = roots(&base);
    let inbox = base.join("inbox");
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.xmp"), PRESET).unwrap();
    std::fs::create_dir_all(inbox.join("sub")).unwrap();
    std::fs::write(inbox.join("sub/ok.xmp"), PRESET).unwrap();

    assert_eq!(inside_roots(&roots, &inbox.join("sub/ok.xmp")).unwrap(), inbox.canonicalize().unwrap().join("sub/ok.xmp"));
    assert!(inside_roots(&roots, &inbox.join("../outside/secret.xmp")).is_err(), "`..` refused");
    assert!(inside_roots(&roots, &inbox.join("sub/../sub/ok.xmp")).is_err(), "`..` refused anywhere");
    assert_eq!(inside_roots(&roots, &inbox.join("./sub")).unwrap(), inbox.canonicalize().unwrap().join("sub"), "a `.` is no step");
    assert!(inside_roots(&roots, &outside.join("secret.xmp")).is_err(), "outside every place");
    assert!(inside_roots(&roots, Path::new("/etc")).is_err());
    assert!(inside_roots(&roots, &inbox.join("missing.xmp")).is_err(), "only what exists");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, inbox.join("door")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.xmp"), inbox.join("secret.xmp")).unwrap();
        assert!(inside_roots(&roots, &inbox.join("door/secret.xmp")).is_err(), "a link to a folder outside");
        assert!(inside_roots(&roots, &inbox.join("secret.xmp")).is_err(), "a link to a file outside");
        let req = crate::pick::PickRequest::files(None, "Presets", crate::pick::PRESET_EXTENSIONS);
        let (names, more) = pick_browser::list(&inbox, &req).unwrap();
        assert_eq!(names.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["sub"], "links are not listed");
        assert!(!more);
    }

    // the chooser itself: opening a path outside keeps the folder it shows and says why
    let mut h = phone(&base, Services::default());
    h.app.run("file.importPresets", json!({})).unwrap();
    let b = h.app.pick_browser.as_mut().unwrap();
    b.open(&inbox).unwrap();
    assert!(b.open(&inbox.join("..")).is_err());
    assert!(b.open(&outside).is_err());
    assert_eq!(b.dir(), Some(inbox.canonicalize().unwrap().as_path()));
    b.open(&inbox.join("sub")).unwrap();
    b.toggle("ok.xmp");
    assert_eq!(b.answer().unwrap(), [inbox.canonicalize().unwrap().join("sub/ok.xmp").to_string_lossy().into_owned()]);

    // a save's name is one plain name, and gets the type's extension
    assert_eq!(pick_browser::save_name(" Mine ", &["lcpreset"]).unwrap(), "Mine.lcpreset");
    assert_eq!(pick_browser::save_name("Mine.LCPRESET", &["lcpreset"]).unwrap(), "Mine.LCPRESET");
    for bad in ["", "../x", "a/b", "a\\b", ".hidden", "nul\0", &"é".repeat(70)] {
        assert!(pick_browser::save_name(bad, &["txt"]).is_err(), "{bad:?}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// Where the in-app chooser stands in for file dialogs, a command whose file can't live in its
/// places is off, its menu item is drawn off with the reason as its tooltip, and run anyway
/// (control channel, MCP) it refuses with that reason; commands it can serve are on without the
/// desktop's dialogs. The desktop is unchanged.
#[test]
fn disabled_without_picker_explains() {
    let base = temp_dir("disabled");
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { pick_roots: roots(&base), ..Default::default() });
    for id in ["file.importLightroom", "dialog.slideshowExport", "app.newLibrary", "app.openLibrary", "file.exportKeywords"] {
        assert!(!crate::menus::ui_enabled(&app, id), "{id}");
        let why = pick_browser::unavailable(&app.services, id).unwrap();
        assert!(why.starts_with("Not available on Android: "), "{why}");
        assert_eq!(app.run(id, json!({})), Err(why.to_string()), "{id}");
        assert!(app.pick_browser.is_none(), "{id}: no chooser opened");
    }
    for id in ["file.importPresets", "file.addPhotos", "file.addFolder", "file.importKeywords"] {
        assert!(crate::menus::ui_enabled(&app, id), "{id}");
        assert!(pick_browser::unavailable(&app.services, id).is_none(), "{id}");
    }

    // the File menu draws the item off, and hovering it shows why
    let mut h = phone(&base, Services::default());
    click(&mut h, "menu:File");
    h.step();
    assert!(h.app.widgets.iter().any(|(w, _)| w == "menuitem-off:file.importLightroom"), "drawn disabled");
    assert!(!h.app.widgets.iter().any(|(w, _)| w == "menuitem:file.importLightroom"));
    assert_eq!(h.request("ui.hoverWidget", json!({"id": "menuitem-off:file.importLightroom"}), T)["ok"], true);
    let shown = h.step_until(Duration::from_secs(5), |h| h.painted_text().iter().any(|t| t == pick_browser::LIGHTROOM_REASON));
    assert!(shown, "the tooltip says why: {:?}", h.painted_text().iter().filter(|t| t.contains("Android")).collect::<Vec<_>>());

    // a host with its own dialogs (the desktop) names no places: nothing changes
    let desk = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Default::default());
    assert!(!pick_browser::in_use(&desk.services));
    assert!(pick_browser::unavailable(&desk.services, "file.importLightroom").is_none());
    assert!(crate::menus::ui_enabled(&desk, "file.importLightroom"));
    let _ = std::fs::remove_dir_all(&base);
}

/// Settings' own folder and file buttons that the in-app chooser can't serve (Open / New
/// Library…, the watched folder, the monitor profile) are drawn off with why, on the page and as
/// the disabled button's tooltip.
#[test]
fn settings_buttons_say_why_without_picker() {
    let base = temp_dir("settings");
    let services = Services { pick_roots: roots(&base), ..Services::default() };
    let mut h = Headless::new(LightcraftApp::new(lightcraft_engine::Session::with_demo(), services), [1400.0, 3000.0], 1.0);
    h.settle(SETTLE);
    for (tab, why) in [
        ("general", pick_browser::ONE_LIBRARY_REASON),
        ("import", pick_browser::AUTO_IMPORT_REASON),
        ("display", pick_browser::DISPLAY_PROFILE_REASON),
    ] {
        let r = h.request("engine.execute", json!({"command": "app.settings", "params": {"tab": tab}}), T);
        assert_eq!(r["ok"], true, "{r}");
        h.settle(SETTLE);
        let shown = h.step_until(Duration::from_secs(5), |h| h.painted_text().iter().any(|t| t == why));
        assert!(shown, "{tab}: {why}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// The library problem window's Choose Another Library… goes through the in-app chooser and
/// takes only an existing library, which the host opens behind its loading frame (as Try Again):
/// the Inbox, a folder inside the library and the library that couldn't be opened are refused
/// with why, and no library is created there.
#[test]
fn library_problem_choose_uses_in_app_picker() {
    let base = temp_dir("problem");
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = asked.clone();
    let services = Services {
        reopen_library: Some(Box::new(move |p| {
            log.borrow_mut().push(p.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut h = phone(&base, services);
    let (inbox, library) = (base.join("inbox"), base.join("library"));
    // the broken library, a folder inside it, and a library someone left in the Inbox
    std::fs::write(library.join(lightcraft_catalog::journal::LOG), b"broken").unwrap();
    std::fs::create_dir_all(library.join("Originals")).unwrap();
    let other = inbox.join("Other Library");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join(lightcraft_catalog::journal::SNAPSHOT), b"").unwrap();
    h.app.library_problem = Some(crate::panels::library_problem::LibraryProblem::new(library.to_string_lossy(), "replay failed"));
    h.settle(SETTLE);
    let choose = |h: &mut Headless, root: &str, sub: Option<&str>| {
        click(h, "button:libraryChoose");
        assert!(h.app.pick_browser.is_some(), "Choose opens the in-app chooser");
        click(h, &format!("pickBrowser:root:{root}"));
        if let Some(sub) = sub {
            click(h, &format!("pickBrowser:entry:{sub}"));
        }
        click(h, "pickBrowser:confirm");
        assert!(h.app.pick_browser.is_none());
        h.app.library_problem.as_ref().map(|p| p.error.clone()).unwrap_or_default()
    };

    let err = choose(&mut h, "Inbox", None);
    assert!(err.contains("no LightCraft library here"), "{err}");
    assert!(!lightcraft_engine::library::is_library(&inbox), "no library created in the Inbox");
    let err = choose(&mut h, "Library", Some("Originals"));
    assert!(err.contains("no LightCraft library here"), "{err}");
    assert!(!lightcraft_engine::library::is_library(&library.join("Originals")), "none created inside the library");
    let err = choose(&mut h, "Library", None);
    assert!(err.contains("Try Again"), "the broken library again: {err}");
    assert!(asked.borrow().is_empty(), "nothing was handed to the host");
    assert!(std::fs::read_dir(&inbox).unwrap().count() == 1 && std::fs::read_dir(&library).unwrap().count() == 2, "nothing written");

    choose(&mut h, "Inbox", Some("Other Library"));
    let other = other.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(*asked.borrow(), [other], "the host reopens the library chosen");
    assert!(h.app.library_problem.is_some(), "the window stays until the host's new session replaces this one");
    let _ = std::fs::remove_dir_all(&base);
}
