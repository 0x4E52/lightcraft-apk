//! File ▸ Back Up Library… and Restore Library from Backup… for a library folder the user can lose
//! with the app (Android: internal storage, deleted by uninstall or Clear data; decision 9), in
//! the shared format of [`lightcraft_engine::backup`].
//!
//! Back Up ([`device_backup`]): the library's files are read on the UI thread (the catalog as it
//! is in memory, [`backup_dir::Plan::of`]), a library over the format's limits (4 GB, no zip64)
//! is refused, the free space in the host's folder for documents
//! (Android: `Documents/LightCraft`, `Services::export_dir_for`) is checked against what the zip
//! needs, and a worker streams the zip there under the activity row "Backing up the library"
//! (cancellable). When it is written the file goes to the share sheet (`file.share`, where the
//! host has one), so it can be kept off the phone.
//!
//! Restore ([`device_restore`]): the backup is chosen in the in-app chooser (the Inbox, where
//! shared files land), and a worker under "Restoring the library" verifies every entry before
//! anything is written, then writes the files into `<library>.restoring` beside the library.
//! Only then, on the UI thread, a marker (`<library>.restore.json`, [`Marker`]) records the swap
//! before anything moves, the library is closed and moved aside to `<library>.before-restore`,
//! the restored folder takes its place, the marker says so, and the library is opened again
//! (Android: behind its loading frame, `Services::reopen_library`). Once it is open
//! ([`finish_pending`]) the originals are pointed at their new place, and the previous library is
//! deleted; when it failed to open, the previous library is put back. A process that died
//! anywhere in between is put right before the library is opened at the next start
//! ([`recover`]). Nothing deletes a previous library the marker doesn't vouch for: one found
//! without a marker is kept under another name.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use lightcraft_engine::activity::{Cancel, TaskGuard};
use lightcraft_engine::backup_dir::{self, BACKUP_LABEL, RESTORE_LABEL, Written};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::pick::{PickRequest, Picked};
use crate::{LibraryAction, LightcraftApp};

/// What a worker answers.
enum Outcome {
    Backup(Result<Written, String>),
    Restore(Result<Staged, String>),
}

/// A restore whose files are written beside the library, waiting to be swapped in.
struct Staged {
    staging: PathBuf,
    /// The backed-up library folder (the catalog's paths of the originals start with it).
    from_dir: String,
    originals: usize,
}

/// Which job a [`Task`] runs.
enum Job {
    Backup,
    /// A restore writing into this staging folder.
    Restore(PathBuf),
}

/// A backup or restore on its worker, with its activity row (gone with the task).
pub struct Task {
    job: Job,
    rx: Receiver<Outcome>,
    cancel: Arc<AtomicBool>,
    _guard: TaskGuard,
}

/// How far a restore's swap got (`<library>.restore.json`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Written before anything moves: the library may be in its place, beside it as
    /// `previous`, or (the process died between the two moves) only there.
    Moving,
    /// The restored library is in place and the previous one waits in `previous`. (A marker
    /// without a phase, from before phases were recorded, means this.)
    #[default]
    Swapped,
    /// An interrupted or failed restore was undone at the start: the previous library is back,
    /// to be told once it opened.
    RolledBack,
}

/// What `<library>.restore.json` records while a restore swaps the library and the restored
/// one is opened. It is written before the first move, so every state a crash can leave is
/// recognised at the next start ([`recover`]) and nothing is deleted unless it is known.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Marker {
    /// The library folder (the restored one once swapped).
    pub library: String,
    /// Where the library that was there before waits.
    pub previous: String,
    /// The folder the restore was written into.
    pub staging: String,
    /// The backed-up library folder.
    pub from_dir: String,
    pub phase: Phase,
}

fn read_marker(lib: &Path) -> Option<Marker> {
    let marker: Marker = serde_json::from_slice(&std::fs::read(marker_path(lib)).ok()?).ok()?;
    (Path::new(&marker.library) == lib).then_some(marker)
}

fn write_marker(lib: &Path, marker: &Marker) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(marker).map_err(|e| e.to_string())?;
    lightcraft_engine::catalog::safe_file::write_atomic(&marker_path(lib), &bytes).map_err(|e| e.to_string())
}

fn remove_marker(lib: &Path) {
    if let Err(e) = std::fs::remove_file(marker_path(lib))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("restore: removing {}: {e}", marker_path(lib).display());
    }
}

/// Remove a folder that only held a restore's copy (`.restoring`, `.restore-failed`): the user
/// still has the backup it came from.
fn remove_copy(dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("restore: removing {}: {e}", dir.display());
    }
}

/// `<library><suffix>` beside the library folder.
fn beside(lib: &Path, suffix: &str) -> PathBuf {
    let name = lib.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "library".into());
    lib.with_file_name(format!("{name}{suffix}"))
}

/// The folder a restore writes into.
pub fn staging_dir(lib: &Path) -> PathBuf {
    beside(lib, ".restoring")
}
/// Where the library waits while a restored one opens.
pub fn previous_dir(lib: &Path) -> PathBuf {
    beside(lib, ".before-restore")
}
/// The restore's marker file.
pub fn marker_path(lib: &Path) -> PathBuf {
    beside(lib, ".restore.json")
}
/// Where a restored library that didn't open is moved before the previous one comes back
/// (removed as soon as it is back, or at the next start).
pub fn failed_dir(lib: &Path) -> PathBuf {
    beside(lib, ".restore-failed")
}

/// Before the library at `lib` is opened (every host's `open_library_session`): settle a restore
/// the process didn't live to finish, from its marker. The library missing with the previous one
/// beside it (died between the two moves, or a rollback whose second move failed) → the previous
/// one is moved back. Marked as moving but nothing moved yet → the restore is dropped. Both moves
/// done → it continues as swapped ([`finish_pending`] once the library is open). Without a marker
/// a `.restore-failed` left behind is removed. Never deletes the library or the previous one.
pub fn recover(lib: &Path) {
    let Some(mut marker) = read_marker(lib) else {
        remove_copy(&failed_dir(lib));
        return;
    };
    let previous = PathBuf::from(&marker.previous);
    let staging = if marker.staging.is_empty() { staging_dir(lib) } else { PathBuf::from(&marker.staging) };
    let (has_lib, has_previous) = (lib.exists(), previous.exists());
    let next = if !has_lib && has_previous {
        if let Err(e) = std::fs::rename(&previous, lib) {
            // (the marker stays: the next start tries again)
            log::error!("restore: putting the previous library back from {}: {e}", previous.display());
            return;
        }
        log::warn!("restore: an unfinished restore was undone; the previous library is back");
        remove_copy(&staging);
        Phase::RolledBack
    } else if marker.phase != Phase::Moving {
        return;
    } else if has_lib && !has_previous {
        log::warn!("restore: a restore stopped before the library moved; nothing was changed");
        remove_copy(&staging);
        Phase::RolledBack
    } else if has_lib && !staging.exists() {
        Phase::Swapped
    } else {
        // (can't tell which folder is which: keep both, and the marker for whoever looks)
        log::error!("restore: an unfinished restore left {} and {}; both are kept", lib.display(), previous.display());
        return;
    };
    marker.phase = next;
    if let Err(e) = write_marker(lib, &marker) {
        log::error!("restore: recording the restore's state: {e}");
    }
}

/// The open library folder, or why there is none to back up or restore.
fn library_dir(app: &LightcraftApp) -> Result<PathBuf, String> {
    app.session.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone()).ok_or_else(|| "No library folder is open".to_string())
}

fn busy(app: &LightcraftApp) -> Result<(), String> {
    if app.library_backup.is_some() {
        return Err(crate::i18n::tr("A library backup or restore is already running").to_owned());
    }
    Ok(())
}

/// Start `work` on a worker under its activity row: kind `export` for a backup (a file written
/// out of the library), `import` for a restore (photos and a catalog brought in).
fn spawn(app: &mut LightcraftApp, job: Job, work: impl FnOnce(&AtomicBool) -> Outcome + Send + 'static) -> Result<(), String> {
    let (name, kind, label) = match job {
        Job::Backup => ("library-backup", "export", BACKUP_LABEL),
        Job::Restore(_) => ("library-restore", "import", RESTORE_LABEL),
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let guard = app.session.activity.start(kind, label, Cancel::Flag(cancel.clone()));
    let (tx, rx) = std::sync::mpsc::channel();
    let repaint = app.tasks.repaint.clone();
    let flag = cancel.clone();
    let work = move || {
        let out = work(&flag);
        let _ = tx.send(out);
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name(name.into()).spawn(work).map_err(|e| format!("Can't start {label}: {e}"))?;
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        work();
    }
    app.library_backup = Some(Task { job, rx, cancel, _guard: guard });
    Ok(())
}

/// The service behind File ▸ Back Up Library… on a device.
pub fn device_backup() -> LibraryAction {
    Box::new(|app: &mut LightcraftApp, _: &Value| start_backup(app))
}

/// The service behind File ▸ Restore Library from Backup… on a device.
pub fn device_restore() -> LibraryAction {
    Box::new(start_restore)
}

fn now_secs() -> i64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Back up the open library into the host's folder for documents, on a worker.
pub fn start_backup(app: &mut LightcraftApp) -> Result<Value, String> {
    busy(app)?;
    library_dir(app)?;
    let plan = backup_dir::Plan::of(&mut app.session)?;
    // (before the space check: no amount of space makes it fit)
    plan.check_limits()?;
    let dir = PathBuf::from(crate::control::export_folder(&app.services, crate::ExportKind::Document));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    let free = app.services.free_space.as_ref().and_then(|f| f(&dir));
    backup_dir::check_space(crate::i18n::tr("the backup"), plan.needed_bytes(), free)?;
    let out = dir.join(backup_dir::backup_name(now_secs()));
    let (originals, in_place) = (plan.originals(), plan.in_place());
    let target = out.clone();
    spawn(app, Job::Backup, move |cancel| Outcome::Backup(backup_dir::write(&plan, &target, cancel)))?;
    Ok(json!({"started": true, "path": out.to_string_lossy(), "originals": originals, "inPlace": in_place}))
}

/// Restore the backup `{path}` (asked for in the file chooser when missing) into the library's
/// place, on a worker; the library is swapped once every entry checked out.
pub fn start_restore(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    busy(app)?;
    let lib = library_dir(app)?;
    let path = match p.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        Some(path) => path.to_string(),
        None => {
            let request = PickRequest::file(
                crate::i18n::tr("Restore Library from Backup (replaces the current library)"),
                crate::i18n::tr("LightCraft library backup"),
                &["zip"],
            );
            match crate::pick::ask(app, "file.restoreLibrary", p, "path", request, |_| None) {
                Picked::Later => return Ok(json!({"choosing": true})),
                Picked::Now(paths) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return Ok(json!({"cancelled": true})),
                },
                Picked::Unavailable => return Err("file.restoreLibrary: `path` (the backup's .zip) is required".into()),
            }
        }
    };
    let zip = PathBuf::from(&path);
    let len = std::fs::metadata(&zip).map_err(|e| format!("Can't open {path}: {e}"))?.len();
    let free = lib.parent().and_then(|parent| app.services.free_space.as_ref().and_then(|f| f(parent)));
    // (the restored files are at most the zip's size)
    backup_dir::check_space(crate::i18n::tr("the restore"), len, free)?;
    let staging = staging_dir(&lib);
    let job_staging = staging.clone();
    spawn(app, Job::Restore(staging.clone()), move |cancel| {
        let restored = backup_dir::verify_file(&zip, cancel).and_then(|v| {
            backup_dir::extract(&zip, &v, &job_staging, cancel)?;
            Ok(Staged { staging: job_staging, from_dir: v.manifest.library_dir.clone(), originals: v.originals })
        });
        Outcome::Restore(restored)
    })?;
    Ok(json!({"started": true, "path": path}))
}

/// Per frame: a finished backup → say where it is and offer it to the share sheet; a finished
/// restore → swap it in and open it. Also finishes a restore swapped in before this library
/// opened ([`finish_pending`]), once.
pub fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    if !app.restore_checked && (app.session.library.is_some() || app.library_problem.is_some()) {
        app.restore_checked = true;
        if let Some(text) = finish_pending(app) {
            app.ui.status = text.clone();
            app.toast_for(ctx, text, 8.0);
        }
    }
    let Some(task) = &app.library_backup else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let outcome = match task.rx.try_recv() {
        Ok(o) => o,
        Err(TryRecvError::Empty) => return,
        // (the worker died: say which job, and drop what a restore had written)
        Err(TryRecvError::Disconnected) => match &task.job {
            Job::Backup => Outcome::Backup(Err("it stopped unexpectedly".into())),
            Job::Restore(staging) => {
                remove_copy(staging);
                Outcome::Restore(Err("it stopped unexpectedly".into()))
            }
        },
    };
    let cancelled = task.cancel.load(Ordering::Relaxed);
    // (the row goes with the task)
    app.library_backup = None;
    let text = match outcome {
        Outcome::Backup(Ok(w)) => backed_up(app, &w),
        Outcome::Restore(Ok(staged)) => match swap_in(app, &staged) {
            Ok(text) => text,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staged.staging);
                format!("Restore failed: {e}. Nothing was changed: the current library is still in use.")
            }
        },
        Outcome::Backup(Err(_)) | Outcome::Restore(Err(_)) if cancelled => return,
        Outcome::Backup(Err(e)) => format!("Backup failed: {e}"),
        Outcome::Restore(Err(e)) => {
            format!("Restore failed: {e}. Nothing was changed: the current library is still in use.")
        }
    };
    app.ui.status = text.clone();
    app.toast_for(ctx, text, 8.0);
}

/// The backup is written: what to tell, and the share sheet for it.
fn backed_up(app: &mut LightcraftApp, w: &Written) -> String {
    let mut text = format!(
        "Library backed up to {path} ({size}, {n} originals). Keep a copy off this device.",
        path = w.path.display(),
        size = backup_dir::size_text(w.bytes),
        n = w.originals
    );
    if w.in_place > 0 {
        text.push(' ');
        text.push_str(&format!("{n} photos added in place are listed by path, not copied.", n = w.in_place));
    }
    if !w.missing.is_empty() {
        text.push(' ');
        text.push_str(&format!("{n} originals were missing and are not in it.", n = w.missing.len()));
    }
    if app.services.share.is_some()
        && let Err(e) = crate::share::start(app, vec![w.path.to_string_lossy().into_owned()])
    {
        log::warn!("library backup: share: {e}");
    }
    text
}

/// A free `<library>.before-restore-<n>` for a previous library left from an earlier restore
/// that isn't known to be finished: kept, never deleted.
fn kept_name(lib: &Path) -> PathBuf {
    let stamp = now_secs();
    (0..1000u32)
        .map(|n| beside(lib, &if n == 0 { format!(".before-restore-{stamp}") } else { format!(".before-restore-{stamp}-{n}") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| beside(lib, &format!(".before-restore-{stamp}-last")))
}

/// Swap the restored folder in for the library and open it (the previous one waits beside it).
/// The marker is written first and moved along with each step, so a crash anywhere leaves a
/// state [`recover`] puts right.
fn swap_in(app: &mut LightcraftApp, staged: &Staged) -> Result<String, String> {
    let lib = library_dir(app)?;
    let previous = previous_dir(&lib);
    if marker_path(&lib).exists() {
        return Err("an earlier restore isn't finished yet; restart LightCraft and try again".into());
    }
    // a previous library without a marker may be the only copy of one: keep it under another name
    let mut kept = None;
    if previous.exists() {
        let aside = kept_name(&lib);
        std::fs::rename(&previous, &aside).map_err(|e| format!("can't move {} aside: {e}", previous.display()))?;
        log::warn!("restore: kept an earlier previous library as {}", aside.display());
        kept = Some(aside);
    }
    app.session.close_library().map_err(|e| format!("the library couldn't be saved first: {e}"))?;
    // nothing may write into the folder while it moves
    app.session.library = None;
    let reopen_current = |app: &mut LightcraftApp| {
        if let Err(e) = app.session.open_library(&lib, false) {
            log::error!("restore: reopening the current library: {e}");
        }
    };
    let mut marker = Marker {
        library: lib.to_string_lossy().into_owned(),
        previous: previous.to_string_lossy().into_owned(),
        staging: staged.staging.to_string_lossy().into_owned(),
        from_dir: staged.from_dir.clone(),
        phase: Phase::Moving,
    };
    if let Err(e) = write_marker(&lib, &marker) {
        reopen_current(app);
        return Err(format!("can't record the restore: {e}"));
    }
    if let Err(e) = std::fs::rename(&lib, &previous) {
        remove_marker(&lib);
        reopen_current(app);
        return Err(format!("can't move the library aside: {e}"));
    }
    if let Err(e) = std::fs::rename(&staged.staging, &lib) {
        // (when the library can't come back the marker stays, and the next start moves it back)
        if std::fs::rename(&previous, &lib).is_ok() {
            remove_marker(&lib);
        }
        reopen_current(app);
        return Err(format!("can't put the restored library in place: {e}"));
    }
    marker.phase = Phase::Swapped;
    if let Err(e) = write_marker(&lib, &marker) {
        // the marker still says Moving: put everything back, or leave it to the next start
        if std::fs::rename(&lib, &staged.staging).is_ok() && std::fs::rename(&previous, &lib).is_ok() {
            remove_marker(&lib);
        }
        reopen_current(app);
        return Err(format!("can't record the restore: {e}"));
    }
    let lib_text = lib.to_string_lossy().into_owned();
    let mut text = match app.services.reopen_library.as_mut() {
        // (the host opens it off this thread; the new app finishes the restore)
        Some(reopen) => reopen(&lib_text).map(|()| format!("Opening the restored library ({n} originals)…", n = staged.originals))?,
        None => {
            let opened = app.session.open_library(&lib, false).map_err(|e| e.to_string());
            if let Err(e) = &opened {
                app.library_problem = Some(crate::panels::library_problem::LibraryProblem::new(&lib_text, e.clone()));
            }
            app.restore_checked = true;
            finish_pending(app).unwrap_or_default()
        }
    };
    if let Some(kept) = kept {
        text.push_str(&format!(" An earlier restore's previous library was kept in {}.", kept.display()));
    }
    Ok(text)
}

/// A restore swapped in before the open library was opened: point its originals at their new
/// place and delete the previous library, or, when it didn't open, put the previous one back.
/// Returns what to tell the user (`None`: no restore was pending).
pub fn finish_pending(app: &mut LightcraftApp) -> Option<String> {
    let lib = app.session.library.as_ref().map(|l| l.dir.clone()).or_else(|| app.library_problem.as_ref().map(|p| PathBuf::from(&p.path)))?;
    // (a host that didn't settle it before opening)
    recover(&lib);
    let marker = read_marker(&lib)?;
    let previous = PathBuf::from(&marker.previous);
    match marker.phase {
        Phase::RolledBack => {
            remove_marker(&lib);
            remove_copy(&failed_dir(&lib));
            return Some(crate::i18n::tr("The restore didn't finish, so the previous library is in use. Nothing was lost.").to_owned());
        }
        // (both folders kept, see `recover`)
        Phase::Moving => return Some(format!("A restore didn't finish; the library before it is kept in {}.", previous.display())),
        Phase::Swapped => {}
    }
    if app.library_problem.is_some() || app.session.library.is_none() {
        // the backup's library didn't open: the one before comes back
        app.session.library = None;
        let failed = failed_dir(&lib);
        remove_copy(&failed);
        if let Err(e) = std::fs::rename(&lib, &failed) {
            // (nothing moved; the marker stays and the next start tries again)
            return Some(format!("The restored library didn't open and the previous one couldn't be put back yet: {e}"));
        }
        if let Err(e) = std::fs::rename(&previous, &lib) {
            // (the marker stays: the next start finds no library and moves the previous one back)
            return Some(format!("The restored library didn't open and the previous one couldn't be put back yet: {e}"));
        }
        remove_marker(&lib);
        remove_copy(&failed);
        let lib_text = lib.to_string_lossy().into_owned();
        match app.services.reopen_library.as_mut() {
            Some(reopen) => {
                let _ = reopen(&lib_text);
            }
            None => {
                if app.session.open_library(&lib, false).is_ok() {
                    app.library_problem = None;
                }
            }
        }
        return Some(crate::i18n::tr("The backup's library didn't open, so the previous library is back.").to_owned());
    }
    let moved = backup_dir::rebase(&mut app.session, Path::new(&marker.from_dir));
    if let Err(e) = &moved {
        // (the marker stays: the next start tries again)
        log::error!("restore: pointing the originals at the restored library: {e}");
        return Some(format!("The library was restored, but its photos couldn't be linked yet: {e}"));
    }
    if let Err(e) = std::fs::remove_dir_all(&previous)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("restore: removing the previous library {}: {e}", previous.display());
    }
    remove_marker(&lib);
    app.session.refresh_view();
    Some(format!("Library restored: {n} photos.", n = app.session.catalog.len()))
}
