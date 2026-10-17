//! What a phone host limits: the one-time library notice and Object / Describe masks on a device
//! without the memory for SAM 3 (`Services::library_notice`, `Services::min_ram_for_sam`).
//! The reopen hook of the library problem window is tested next to it, in
//! `panels/library_problem.rs`.

use crate::headless::Headless;
use crate::panels::masking::{sam_ram_reason, start_describe, start_object};
use crate::panels::notices;
use crate::state::Dialog;
use crate::{LightcraftApp, Services, UiState};
use std::time::Duration;

const GB: u64 = 1 << 30;
const NOTICE: &str = "Your library is stored inside LightCraft. Uninstalling it deletes the library.";

fn temp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lc-phone-limits-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An app with a library open on disk, as after the host's loader.
fn app_with_library(dir: &std::path::Path, services: Services) -> LightcraftApp {
    let mut session = lightcraft_engine::Session::new().with_fs();
    session.open_library(dir.join("library"), true).unwrap();
    LightcraftApp::new(session, services)
}

#[test]
fn library_notice_shown_once() {
    let dir = temp("notice");
    let mut app = app_with_library(&dir, Services { library_notice: Some(NOTICE.into()), ..Default::default() });
    notices::logic(&mut app);
    assert_eq!(app.library_notice.as_deref(), Some(NOTICE), "told on the first frame with a library");
    assert!(app.notices.is_empty(), "information is not a settings problem: {:?}", app.notices);
    assert!(!app.ui.library_notice_shown, "not marked as told until the user closes it");
    notices::logic(&mut app);
    assert_eq!(app.library_notice.as_deref(), Some(NOTICE), "still up");
    notices::library_notice_ok(&mut app);
    assert!(app.library_notice.is_none() && app.ui.library_notice_shown);
    notices::logic(&mut app);
    assert!(app.library_notice.is_none(), "not again while the app runs");

    // the flag is saved with the settings: the next start does not tell it again
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(saved["libraryNoticeShown"], true);
    let next_dir = temp("notice-next");
    let mut next = app_with_library(&next_dir, Services { library_notice: Some(NOTICE.into()), ..Default::default() });
    next.ui = serde_json::from_value::<UiState>(saved).unwrap();
    notices::logic(&mut next);
    assert!(next.library_notice.is_none() && next.notices.is_empty(), "{:?}", next.notices);

    // settings from before the flag existed read as "not shown"
    let old = serde_json::from_value::<UiState>(serde_json::json!({})).unwrap();
    assert!(!old.library_notice_shown);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&next_dir);
}

#[test]
fn library_notice_waits_for_a_library_and_is_a_host_choice() {
    let dir = temp("notice-wait");
    // the desktop says nothing
    let mut desktop = app_with_library(&dir.join("a"), Services::default());
    notices::logic(&mut desktop);
    assert!(desktop.library_notice.is_none() && !desktop.ui.library_notice_shown);
    // no library open yet (the problem session in memory): not told, and not marked as told
    let mut problem = LightcraftApp::new(lightcraft_engine::Session::new(), Services { library_notice: Some(NOTICE.into()), ..Default::default() });
    notices::logic(&mut problem);
    assert!(problem.library_notice.is_none() && !problem.ui.library_notice_shown);
    // nor while the problem window is up over a library
    let mut app = app_with_library(&dir.join("b"), Services { library_notice: Some(NOTICE.into()), ..Default::default() });
    app.library_problem = Some(crate::panels::library_problem::LibraryProblem::new("/lib", "damaged"));
    notices::logic(&mut app);
    assert!(app.library_notice.is_none() && !app.ui.library_notice_shown);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn library_notice_is_not_a_settings_problem_window() {
    let dir = temp("notice-window");
    let app = app_with_library(&dir, Services { library_notice: Some(NOTICE.into()), png: None, ..Default::default() });
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    h.step();
    h.step();
    h.step();
    let has = |h: &Headless, id: &str| h.app.widgets.iter().any(|(w, _)| w == id);
    assert!(has(&h, "button:libraryNoticeOk"), "the notice is shown with its own window");
    assert!(!has(&h, "button:noticeOk"), "not in the \"Settings file problem\" window");
    assert!(!h.app.ui.library_notice_shown, "shown, not yet read");
    assert_eq!(h.request("ui.clickWidget", serde_json::json!({"id": "button:libraryNoticeOk"}), Duration::from_secs(20))["ok"], true);
    for _ in 0..4 {
        h.step();
    }
    assert!(h.app.library_notice.is_none() && !has(&h, "button:libraryNoticeOk"), "OK closes it");
    assert!(h.app.ui.library_notice_shown, "and OK is what marks it as told");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The host's text goes through the catalogs like every other built-in label: a catalog entry for it
/// is used, and a text without one stays as it is.
#[test]
fn library_notice_text_is_translated() {
    use crate::i18n::{Locale, set_language};
    // "Settings file problem" is a catalog key (another window's title), standing in for the notice
    set_language(Locale::Ja);
    let translated = notices::library_notice_text("Settings file problem");
    let unknown = notices::library_notice_text(NOTICE);
    set_language(Locale::En);
    assert_eq!(translated, "設定ファイルの問題");
    assert_eq!(unknown, NOTICE, "no entry: the source text");
    assert_eq!(notices::library_notice_text("Settings file problem"), "Settings file problem");
}

#[test]
fn sam_disabled_below_min_ram() {
    // the pure check: no limit asked for, enough memory, too little, unknown
    assert_eq!(sam_ram_reason(None, Some(2 * GB)), None, "the desktop asks for no limit");
    assert_eq!(sam_ram_reason(None, None), None);
    assert_eq!(sam_ram_reason(Some(8 * GB), Some(8 * GB)), None);
    assert_eq!(sam_ram_reason(Some(8 * GB), Some(16 * GB)), None);
    assert_eq!(sam_ram_reason(Some(8 * GB), Some(8 * GB - 1)), Some("Needs a device with 8 GB of memory"));
    assert_eq!(sam_ram_reason(Some(8 * GB), Some(4 * GB)), Some("Needs a device with 8 GB of memory"));
    assert!(sam_ram_reason(Some(8 * GB), None).is_some(), "a device that can't tell is not trusted with several GB");

    // the app: a limit nothing meets (whatever memory this machine has, known or not)
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { min_ram_for_sam: Some(u64::MAX), ..Default::default() });
    assert_eq!(app.sam_unavailable_reason(), Some("Needs a device with 8 GB of memory"));
    let id = *app.session.visible().first().unwrap();
    app.run("library.select", serde_json::json!({"ids": [id.0]})).unwrap();
    let ctx = egui::Context::default();
    let masks = |app: &LightcraftApp| app.session.develop_of(id).map_or(0, |d| d.masks.len());
    let before = masks(&app);
    start_object(&mut app, &ctx, "new");
    start_describe(&mut app, "new");
    assert_eq!(masks(&app), before, "no mask was started");
    assert!(app.ui.dialog.is_none() && app.ui.describe.is_none() && app.ui.tool != "object", "nor a download offer or a Describe field");

    // the setup dialog, opened by another way in, gives the reason and nothing to confirm
    app.offer_sam_download(None);
    let dlg = app.ui.dialog.clone().unwrap();
    assert!(matches!(dlg, Dialog::SamModel { .. }));
    assert_eq!(crate::panels::dialogs::confirm_dialog(&mut app, &dlg).unwrap_err(), "Needs a device with 8 GB of memory");
    assert!(!app.session.segmenter.download_status().running);

    // every way of dispatching by id is refused with the reason, and changes nothing
    let reason = "Needs a device with 8 GB of memory";
    for (command, params) in [
        ("mask.add", serde_json::json!({"kind": "object"})),
        ("mask.add", serde_json::json!({"kind": "prompt", "text": "a dog"})),
        ("mask.addComponent", serde_json::json!({"kind": "object", "op": "add"})),
        ("mask.objectPoint", serde_json::json!({"x": 0.5, "y": 0.5})),
        ("mask.refineDetail", serde_json::json!({})),
        ("segment.prepare", serde_json::json!({})),
        ("segment.model.download", serde_json::json!({"acknowledged": true})),
    ] {
        assert_eq!(app.run(command, params).unwrap_err(), reason, "{command}");
    }
    assert_eq!(masks(&app), before);
    assert!(!app.session.segmenter.download_status().running);
    // other masks are not SAM
    assert!(app.sam_command_refusal("mask.add", &serde_json::json!({"kind": "linear"})).is_none());
    // and the command lists report them as off
    let listed = crate::control::all_commands(&app);
    let enabled = |id: &str| listed.as_array().unwrap().iter().find(|c| c["id"] == id).map(|c| c["enabled"].clone());
    assert_eq!(enabled("segment.model.download"), Some(serde_json::json!(false)));
    assert_eq!(enabled("mask.objectPoint"), Some(serde_json::json!(false)));

    // no limit: unchanged
    let desktop = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services::default());
    assert_eq!(desktop.sam_unavailable_reason(), None);
}
