//! File ▸ Hand Off Photos… and File ▸ Receive Handoff… (APK-62): photos go from one device's
//! library to another's with their edits, and back, through files the person moves — a share, a
//! USB cable or drive, a folder their own sync tool keeps — with nothing silently lost. The format
//! and the matching rules are `lightcraft_engine::handoff`'s.
//!
//! Hand off writes the selection into a new folder `LightCraft Handoff <time>`: in the host's
//! folder for originals with sidecars (Android: `Documents/LightCraft`,
//! [`crate::control::export_folder`]) or, where the host has none, in a folder the person picks.
//! The files are written on a worker that holds the activity row "Handing off photos"; with the
//! host's share sheet ([`crate::Services::share`]) they are then offered to share.
//!
//! Receive reads a handoff folder on a worker ("Reading handoff"), then its dialog lists what it
//! found: new photos (imported, copied into the library, by default), photos whose handed-off
//! settings are newer (each read only when ticked: never the default) and photos whose library
//! settings are as new or newer (listed, never read).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::pick::{PickRequest, Picked};
use crate::state::{Dialog, HandoffRow};
use crate::theme::Tokens;

/// The activity row of a hand-off.
pub const SEND_LABEL: &str = "Handing off photos";
/// The activity row of reading a handoff folder.
pub const READ_LABEL: &str = "Reading handoff";

/// `file.handoff {ids?, dir?, into?}`: hand the photos (default the selection) off into `dir`, or
/// into a new folder in `into`, the host's folder for originals, or a folder the person picks.
pub(crate) fn send(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.tasks.is_running(SEND_LABEL) {
        return Err(crate::i18n::tr("A handoff is already being written").to_string());
    }
    let ids: Vec<lightcraft_engine::catalog::PhotoId> = match p.get("ids").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .map(|v| v.as_u64().map(lightcraft_engine::catalog::PhotoId))
            .collect::<Option<_>>()
            .ok_or_else(|| "`ids` must be photo ids".to_string())?,
        None => app.session.selected(),
    };
    if ids.is_empty() {
        return Err(crate::i18n::tr("Select the photos to hand off").to_string());
    }
    let now = (app.session.clock)();
    let dir = match (p.get("dir").and_then(Value::as_str), p.get("into").and_then(Value::as_str)) {
        (Some(d), _) if !d.trim().is_empty() => d.trim().to_string(),
        (_, Some(base)) if !base.trim().is_empty() => folder_in(base.trim(), &now),
        _ if app.services.export_dir_for.is_some() => folder_in(&crate::control::export_folder(&app.services, crate::ExportKind::Document), &now),
        _ => {
            let req = PickRequest::folder(crate::i18n::tr("Hand Off Photos…"));
            match crate::pick::ask(app, "file.handoff", p, "into", req, |s| s.pick_folder.as_mut().and_then(|f| f()).map(|x| vec![x])) {
                Picked::Now(v) => match v.into_iter().next() {
                    Some(base) => folder_in(&base, &now),
                    None => return Ok(Value::Null),
                },
                Picked::Later => return Ok(json!({"choosing": true})),
                Picked::Unavailable => return Err(crate::i18n::tr("Choose a folder: pass `dir`").to_string()),
            }
        }
    };
    let job = lightcraft_engine::handoff::prepare(&mut app.session, &ids, &dir).map_err(|e| e.to_string())?;
    let n = job.len();
    // the row's ✕ stops the job before its next photo (the manifest lists those written)
    let (row, cancel, handle) = cancellable_row(app, "export", SEND_LABEL);
    crate::tasks::spawn_with_row(
        app,
        SEND_LABEL,
        Some(row),
        move || {
            let mut progress = |done: usize, of: usize| handle.progress(count(done), count(of));
            lightcraft_engine::guard::catch("handoff", || job.run(&cancel, &mut progress).map_err(|e| e.to_string()))
                .unwrap_or_else(|_| Err("the handoff stopped unexpectedly (see the log)".to_string()))
        },
        sent,
    )?;
    Ok(json!({"dir": dir, "photos": n}))
}

/// An activity row of `kind` the person can stop with ✕: the row, the cancel flag it sets and a
/// handle the worker reports its progress through.
fn cancellable_row(
    app: &LightcraftApp,
    kind: &'static str,
    label: &str,
) -> (lightcraft_engine::activity::TaskGuard, Arc<AtomicBool>, lightcraft_engine::activity::TaskHandle) {
    let cancel = Arc::new(AtomicBool::new(false));
    let row = app.session.activity.start(kind, label, lightcraft_engine::activity::Cancel::Flag(cancel.clone()));
    let handle = row.handle();
    (row, cancel, handle)
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn folder_in(base: &str, now: &str) -> String {
    lightcraft_engine::handoff::folder_in(base, now).to_string_lossy().into_owned()
}

/// A hand-off ended: say so, keep the result for `ui.inspect`, and offer its files to share.
fn sent(app: &mut LightcraftApp, ctx: &egui::Context, r: Result<lightcraft_engine::handoff::HandoffWritten, String>) {
    match r {
        Ok(w) => {
            for (what, why) in w.skipped.iter().chain(&w.failed) {
                log::warn!("handoff: {what}: {why}");
            }
            let mut text = format!("{} {} → {}", crate::i18n::tr("Photos handed off:"), w.photos, w.dir);
            if !w.failed.is_empty() || !w.skipped.is_empty() {
                text = format!("{text} · {} {}", crate::i18n::tr("Not handed off"), w.failed.len() + w.skipped.len());
            }
            if w.cancelled {
                text = format!("{text} · {}", crate::i18n::tr("Cancelled"));
            }
            app.ui.status = text.clone();
            app.toast(ctx, text);
            app.handoff_last = serde_json::to_value(&w).ok();
            if app.services.share.is_some()
                && !w.files.is_empty()
                && let Err(e) = crate::share::start(app, w.files)
            {
                app.toast(ctx, e);
            }
        }
        Err(e) => {
            log::warn!("handoff: {e}");
            app.ui.status = e.clone();
            app.toast_for(ctx, e, 12.0);
        }
    }
}

/// `file.receiveHandoff {dir?}`: read a handoff folder (picked when not given) in the background,
/// then show what it holds for the library.
pub(crate) fn receive(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.tasks.is_running(READ_LABEL) {
        return Err(crate::i18n::tr("A handoff is already being read").to_string());
    }
    let dir = match p.get("dir").and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => d.to_string(),
        None => {
            let req = PickRequest::folder(crate::i18n::tr("Receive Handoff…"));
            match crate::pick::ask(app, "file.receiveHandoff", p, "dir", req, |s| s.pick_folder.as_mut().and_then(|f| f()).map(|x| vec![x])) {
                Picked::Now(v) => match v.into_iter().next() {
                    Some(d) => d,
                    None => return Ok(Value::Null),
                },
                Picked::Later => return Ok(json!({"choosing": true})),
                Picked::Unavailable => return Err(crate::i18n::tr("Choose a folder: pass `dir`").to_string()),
            }
        }
    };
    let input = lightcraft_engine::handoff::HandoffInput::new(&app.session, &dir).map_err(|e| e.to_string())?;
    // the row's ✕ stops the reading: nothing is listed then, as nothing was read in full
    let (row, cancel, handle) = cancellable_row(app, "import", READ_LABEL);
    let stop = cancel.clone();
    crate::tasks::spawn_with_row(
        app,
        READ_LABEL,
        Some(row),
        move || {
            let mut progress = |done: usize, of: usize| handle.progress(count(done), count(of));
            lightcraft_engine::guard::catch("handoff", || {
                lightcraft_engine::handoff::scan_with(input, &cancel, &mut progress).map_err(|e| e.to_string())
            })
            .unwrap_or_else(|_| Err("reading the handoff stopped unexpectedly (see the log)".to_string()))
        },
        move |app, ctx, r| match r {
            Ok(_) if stop.load(Ordering::Relaxed) => {
                app.ui.status = crate::i18n::tr("Reading the handoff was cancelled").to_string();
            }
            Ok(plan) => open(app, plan),
            Err(e) => {
                app.ui.status = e.clone();
                app.toast_for(ctx, e, 12.0);
            }
        },
    )?;
    Ok(json!({"reading": dir}))
}

/// Show what handoff `plan` holds in the Receive Handoff dialog.
pub(crate) fn open(app: &mut LightcraftApp, plan: lightcraft_engine::handoff::HandoffPlan) {
    let row = |m: &lightcraft_engine::handoff::HandoffMatch| HandoffRow {
        id: m.id,
        name: m.name.clone(),
        handoff_edited: m.handoff_edited.clone().unwrap_or_default(),
        library_edited: m.library_edited.clone().unwrap_or_default(),
    };
    app.ui.dialog = Some(Dialog::ReceiveHandoff {
        dir: plan.dir.clone(),
        new: plan.new.len(),
        newer: plan.newer.iter().map(row).collect(),
        older: plan.older.iter().map(row).collect(),
        same: plan.same,
        damaged: plan.damaged.iter().map(|(f, why)| format!("{f}: {why}")).collect(),
        import_new: true,
        read: Vec::new(),
    });
    app.handoff_plan = Some(plan);
}

/// The dialog's body.
pub fn body(ui: &mut egui::Ui, dlg: &mut Dialog) {
    let Dialog::ReceiveHandoff { dir, new, newer, older, same, damaged, import_new, read } = dlg else { return };
    let t = Tokens::get(ui.ctx());
    ui.set_min_width(420.0);
    ui.label(egui::RichText::new(dir.as_str()).color(t.text_dim));
    ui.add_space(6.0);
    let count = |text: &str, n: usize| format!("{} ({n})", crate::i18n::tr(text));
    let r = ui.add_enabled(*new > 0, egui::Checkbox::new(import_new, count("Import new photos", *new)));
    crate::widgets::register(ui.ctx(), "check:handoffImportNew", r.rect);
    let dim = |ui: &mut egui::Ui, s: &str| ui.label(egui::RichText::new(s).color(t.text_dim));
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        if !newer.is_empty() {
            ui.add_space(4.0);
            ui.label(count("Newer settings in the handoff: tick the photos to read", newer.len()));
            for (i, row) in newer.iter().enumerate() {
                let mut on = read.contains(&row.id);
                let text = format!("{}  ·  {} → {}", row.name, shown_time(&row.library_edited), shown_time(&row.handoff_edited));
                let r = ui.checkbox(&mut on, text);
                crate::widgets::register(ui.ctx(), format!("check:handoffRead{i}"), r.rect);
                if r.changed() {
                    if on {
                        read.push(row.id);
                    } else {
                        read.retain(|id| *id != row.id);
                    }
                }
            }
        }
        if !older.is_empty() {
            ui.add_space(4.0);
            dim(ui, &count("Kept: the library's settings are as new or newer", older.len()));
            for row in older.iter() {
                dim(ui, &format!("{}  ·  {} / {}", row.name, shown_time(&row.library_edited), shown_time(&row.handoff_edited)));
            }
        }
        if !damaged.is_empty() {
            ui.add_space(4.0);
            dim(ui, &count("Skipped: missing, changed or unreadable files", damaged.len()));
            for d in damaged.iter() {
                dim(ui, d);
            }
        }
    });
    ui.add_space(6.0);
    if *same > 0 {
        dim(ui, &count("Already up to date", *same));
    }
    if !newer.is_empty() {
        dim(ui, crate::i18n::tr("Reading replaces the photo's rating, labels, metadata and edits with the handoff's. Edit → Undo puts them back."));
    }
}

/// An ISO time as the dialog shows it ("2026-10-01 10:00"), or "unknown".
fn shown_time(t: &str) -> String {
    if t.trim().is_empty() {
        return crate::i18n::tr("unknown").to_string();
    }
    t.chars().take(16).map(|c| if c == 'T' { ' ' } else { c }).collect()
}

/// Receive with the dialog's choices: the ticked newer settings are read now (no file is read:
/// the scan read them), as one undo step; the new photos are imported in the background.
pub fn confirm(app: &mut LightcraftApp, dlg: &Dialog) -> Result<Value, String> {
    let Dialog::ReceiveHandoff { dir, import_new, read, .. } = dlg else { return Err("not a handoff".into()) };
    let plan = match app.handoff_plan.take() {
        Some(plan) if plan.dir == *dir => plan,
        _ => return Err(crate::i18n::tr("The handoff was not read: receive it again").to_string()),
    };
    let new = if *import_new { lightcraft_engine::handoff::new_paths(&plan) } else { Vec::new() };
    // each new photo keeps the sending device's time of its last change
    let received = lightcraft_engine::handoff::import_options(&app.session, &plan).received;
    // every reason the import could be refused, checked before anything is read
    if !new.is_empty()
        && let Err(e) = crate::import::can_start_received(app)
    {
        app.handoff_plan = Some(plan);
        return Err(e);
    }
    let choice = lightcraft_engine::handoff::HandoffChoice {
        import_new: false,
        read: read.iter().copied().map(lightcraft_engine::catalog::PhotoId).collect(),
    };
    let mut report = None;
    app.session
        .execute_fn("library.receiveHandoff", |s| {
            report = Some(lightcraft_engine::handoff::apply(s, plan, &choice)?);
            Ok(Value::Null)
        })
        .map_err(|e| e.to_string())?;
    let r = report.unwrap_or_default();
    for (id, why) in &r.refused {
        log::warn!("receive handoff: photo {id}: {why}");
    }
    let importing = new.len();
    if !new.is_empty() {
        let mode = match lightcraft_engine::handoff::import_mode(&app.session) {
            lightcraft_engine::import::ImportMode::Add => "add",
            _ => "copy",
        };
        crate::import::start_received(app, new, json!({"mode": mode}), received)?;
    }
    Ok(json!({"read": r.read, "refused": r.refused, "failed": r.failed, "importing": importing}))
}
