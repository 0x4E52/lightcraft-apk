//! File ▸ Share… (`file.share`, APK-27): export the selected photos where other apps see them, then
//! hand the files to the system's share sheet.
//!
//! The host gives [`crate::Services::share`] (Android: a MediaStore lookup of each file's content
//! URI, then `Intent.createChooser(ACTION_SEND / ACTION_SEND_MULTIPLE)`). The command exports with
//! the previous export's settings (an original with its sidecar as a JPEG instead: a share sends one
//! file per photo) into the host's folder for that kind of file
//! ([`crate::control::export_folder`]), and once the export has written its files
//! ([`crate::export_task::poll`] → [`poll`]) calls the service on a worker thread that holds the
//! activity row "Preparing to share" (cancellable) for the whole lookup. `{paths}` shares those
//! files as they are.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, TryRecvError};

use lightcraft_engine::activity::{Cancel, TaskGuard};
use serde_json::{Value, json};

use crate::LightcraftApp;

/// The activity row of the lookup.
pub const LABEL: &str = "Preparing to share";
/// Why `file.share` is refused without the host's share sheet.
pub const UNAVAILABLE: &str = "Sharing is not available here";
/// At most this many files are shared at once.
pub const MAX_FILES: usize = 500;

/// Hands files (absolute paths) to the share sheet; called on a worker thread, checking the flag
/// between files (set: stop, nothing shared). `Err` is the message to show.
pub type ShareFn = Arc<dyn Fn(&[String], &AtomicBool) -> Result<(), String> + Send + Sync>;

/// A share on its worker, and its activity row.
pub struct ShareTask {
    rx: Receiver<Result<(), String>>,
    cancel: Arc<AtomicBool>,
    _guard: TaskGuard,
}

/// `file.share {paths?}`: with paths, share those files; without, export the selection for sharing
/// (in the background where the host can) and share its files once written.
pub(crate) fn run(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.services.share.is_none() {
        return Err(UNAVAILABLE.into());
    }
    if app.share.is_some() || app.share_after_export {
        return Err("Already preparing to share".into());
    }
    if let Some(paths) = p.get("paths") {
        let paths: Vec<String> = paths
            .as_array()
            .ok_or("file.share: `paths` must be a list of files")?
            .iter()
            .map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).ok_or("file.share: each path must be a file"))
            .collect::<Result<_, _>>()?;
        return start(app, paths);
    }
    let prev = app.session.last_export.clone().unwrap_or_else(|| json!({}));
    let mut params = lightcraft_engine::export::ExportOptions::known_keys_only(&prev);
    let Some(o) = params.as_object_mut() else { return Err("file.share: the previous export's settings are unreadable".into()) };
    // one file per photo: an original's sidecar stays behind, as a JPEG goes
    if lightcraft_engine::export::ExportOptions::from_json(&prev).format == lightcraft_engine::export::ExportFormat::Original {
        o.insert("format".into(), json!("jpeg"));
    }
    o.remove("path");
    o.remove("destinationMode");
    if let Some(ids) = p.get("ids") {
        o.insert("ids".into(), ids.clone());
    }
    let format = lightcraft_engine::export::ExportOptions::from_json(&params).format;
    let dir = crate::control::export_folder(&app.services, crate::ExportKind::of_format(format));
    params["dir"] = json!(dir);
    params["background"] = json!(true);
    let last = app.session.last_export.clone();
    // the export's files are read from here when it ends: never an earlier export's
    app.last_export_result = None;
    let r = crate::control::export_active(app, &params);
    // a share is not an export the user chose: Export with Previous keeps the one before
    app.session.last_export = last;
    let out = r?;
    if out.get("background").and_then(Value::as_bool) == Some(true) {
        app.share_after_export = true;
        return Ok(json!({"exporting": true}));
    }
    start(app, written(out.get("files")))
}

/// The paths a finished export wrote (its per-photo results).
pub(crate) fn written(files: Option<&Value>) -> Vec<String> {
    files
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(|f| f.get("path").and_then(Value::as_str).map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Share `paths` on a worker; `Err` without the host's share sheet, without files, or while a
/// share runs.
pub(crate) fn start(app: &mut LightcraftApp, paths: Vec<String>) -> Result<Value, String> {
    let Some(share) = app.services.share.clone() else { return Err(UNAVAILABLE.into()) };
    if app.share.is_some() {
        return Err("Already preparing to share".into());
    }
    if paths.is_empty() {
        return Err("Nothing to share: no file was exported".into());
    }
    if paths.len() > MAX_FILES {
        return Err(format!("Share at most {MAX_FILES} files at once"));
    }
    let n = paths.len();
    let cancel = Arc::new(AtomicBool::new(false));
    let guard = app.session.activity.start("export", LABEL, Cancel::Flag(cancel.clone()));
    let (tx, rx) = std::sync::mpsc::channel();
    let repaint = app.tasks.repaint.clone();
    let flag = cancel.clone();
    let job = move || {
        // a panic in the host's share is an error, never a dead worker with a row left behind
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| share(&paths, &flag)))
            .unwrap_or_else(|_| Err("Sharing failed unexpectedly".to_owned()));
        let _ = tx.send(answer);
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name("share".into()).spawn(job).map_err(|e| format!("Can't share: {e}"))?;
    #[cfg(target_arch = "wasm32")]
    job();
    app.share = Some(ShareTask { rx, cancel, _guard: guard });
    Ok(json!({"sharing": n}))
}

/// Per frame: File ▸ Share's export finished → share its files; the share's answer → its error, if
/// any (the share sheet itself is the success).
pub fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.share_after_export && app.export.is_none() {
        app.share_after_export = false;
        let result = app.last_export_result.clone().unwrap_or_default();
        if result.get("cancelled").and_then(Value::as_bool) != Some(true) {
            let paths = written(result.get("files"));
            // (an export that wrote nothing has said why in its own toast)
            if !paths.is_empty()
                && let Err(e) = start(app, paths)
            {
                app.toast(ctx, e);
            }
        }
    }
    let Some(task) = &app.share else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let answer = match task.rx.try_recv() {
        Ok(answer) => answer,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("Sharing stopped unexpectedly".to_owned()),
    };
    let cancelled = task.cancel.load(std::sync::atomic::Ordering::Relaxed);
    // (the row goes with the task)
    app.share = None;
    if let Err(e) = answer {
        if cancelled {
            return;
        }
        log::warn!("file.share: {e}");
        app.ui.status = e.clone();
        app.toast(ctx, e);
    }
}
