//! Saving what a native host keeps on disk: the library's queued changes, its view
//! (`view.json`: source, sort, selection) and the app settings (`ui.json`).
//!
//! Two moments, two functions:
//! - [`flush_periodic`] while the app runs (a timer, a frame): writes what changed and never
//!   touches an open interaction. A slider drag is begin → `develop.set` per frame → end
//!   (`panels/edit.rs`, "a drag is one undo step"); committing it mid-drag would split the drag
//!   into one undo step per frame. The web host's timer does the same.
//! - [`flush_leaving`] when the app is left or quits (the desktop's `on_exit`; Android's focus
//!   loss): commits an open interaction first, then writes everything. No frame needs to run in
//!   between, so a host can call it from a lifecycle callback.

use super::prefs::PrefsWriter;
use crate::LightcraftApp;

/// Write the library's queued changes (when there are any and saving is not backing off after a
/// failure), the view if it changed, and the app settings if they changed. An open interaction
/// stays open. The settings writer's error is returned (the library reports its own:
/// `Session::unsaved`).
pub fn flush_periodic(app: &mut LightcraftApp, prefs: &mut PrefsWriter) -> Result<(), String> {
    app.session.persist_if_dirty();
    app.session.save_view();
    prefs.save(app)
}

/// Commit an open interaction as one undo step, write the library's queued changes, the view and
/// the app settings. Every step runs even when one fails; each failure is logged and the first
/// is returned.
pub fn flush_leaving(app: &mut LightcraftApp, prefs: &mut PrefsWriter) -> Result<(), String> {
    let mut first: Option<String> = None;
    let mut failed = |what: &str, e: String| {
        // (the settings writer's own message already says what failed)
        let text = if e.starts_with(what) { e } else { format!("{what}: {e}") };
        log::error!("{text}");
        first.get_or_insert(text);
    };
    if let Err(e) = app.session.end_interaction() {
        failed("committing the edit in progress failed", e.to_string());
    }
    if let Err(e) = app.session.persist() {
        failed("saving the library failed", e.to_string());
    }
    // (logs its own errors)
    app.session.save_view();
    if let Err(e) = prefs.save(app) {
        failed("saving the app settings failed", e);
    }
    first.map_or(Ok(()), Err)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::Services;
    use serde_json::json;

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-ui-flush-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn exposure(s: &lightcraft_engine::Session, id: lightcraft_engine::catalog::PhotoId) -> Option<f64> {
        s.develop_of(id).and_then(|d| lightcraft_develop::controls::get(&d, "light.exposure"))
    }

    /// A library with the demo photos, the second one selected and a slider drag begun on it.
    fn dragging(lib: &std::path::Path) -> (LightcraftApp, lightcraft_engine::catalog::PhotoId) {
        let mut session = lightcraft_engine::Session::new().with_fs();
        session.open_library(lib, true).unwrap();
        let mut app = LightcraftApp::new(session, Services::default());
        let id = *app.session.visible().get(1).unwrap();
        app.run("library.select", json!({"ids": [id.0]})).unwrap();
        app.run("develop.beginInteraction", json!({"label": "Exposure"})).unwrap();
        app.run("develop.set", json!({"control": "light.exposure", "value": 1.25})).unwrap();
        app.run("develop.set", json!({"control": "light.exposure", "value": 1.5})).unwrap();
        assert!(app.session.interaction.is_some());
        (app, id)
    }

    /// Leaving the app mid-drag (a phone's focus loss, quitting) commits the drag and writes the
    /// view and the settings at once, with no frame in between: reopened, the edit and the
    /// selection are there.
    #[test]
    fn flush_leaving_commits_open_interaction_and_view() {
        let dir = temp("leaving");
        let lib = dir.join("lib");
        let (mut app, id) = dragging(&lib);
        let mut prefs = PrefsWriter::new(Some(dir.join("ui.json")), &app, false);
        app.ui.thumb_size += 40.0;
        flush_leaving(&mut app, &mut prefs).unwrap();
        assert!(app.session.interaction.is_none(), "the drag is committed");
        assert!(lib.join("view.json").exists());
        assert!(dir.join("ui.json").exists());
        // the process ends here (no close_library): what was flushed is what the next start sees
        drop(app);
        let mut reopened = lightcraft_engine::Session::new().with_fs();
        reopened.open_library(&lib, false).unwrap();
        assert_eq!(exposure(&reopened, id), Some(1.5));
        assert_eq!(reopened.selection.active, Some(id), "view.json restored the selection");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A periodic save in the middle of a drag leaves the drag open: no undo step is made, so the
    /// drag still ends as one.
    #[test]
    fn periodic_flush_keeps_open_interaction() {
        let dir = temp("periodic");
        let (mut app, _) = dragging(&dir.join("lib"));
        let undo = app.session.undo.len();
        let mut prefs = PrefsWriter::new(Some(dir.join("ui.json")), &app, false);
        app.ui.thumb_size += 40.0;
        flush_periodic(&mut app, &mut prefs).unwrap();
        assert!(app.session.interaction.is_some(), "the drag stays open");
        assert_eq!(app.session.undo.len(), undo, "no undo step mid-drag");
        assert!(dir.join("ui.json").exists(), "the changed settings are written");
        app.run("develop.endInteraction", json!({})).unwrap();
        assert_eq!(app.session.undo.len(), undo + 1, "the drag is one undo step");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A settings file that can't be written is reported, and the library is still saved.
    #[test]
    fn flush_leaving_reports_the_first_failure_and_saves_the_rest() {
        let dir = temp("failing");
        let lib = dir.join("lib");
        let (mut app, id) = dragging(&lib);
        // a file where the settings folder should be
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, "not a folder").unwrap();
        let mut prefs = PrefsWriter::new(Some(blocked.join("ui.json")), &app, false);
        app.ui.thumb_size += 40.0;
        let e = flush_leaving(&mut app, &mut prefs).unwrap_err();
        assert!(e.contains("app settings"), "{e}");
        drop(app);
        let mut reopened = lightcraft_engine::Session::new().with_fs();
        reopened.open_library(&lib, false).unwrap();
        assert_eq!(exposure(&reopened, id), Some(1.5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
