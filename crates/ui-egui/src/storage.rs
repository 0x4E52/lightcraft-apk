//! Settings ▸ Storage: what LightCraft keeps on the device, part by part, with the total the
//! system shows in its App info, and a Clear button on every part that is only a cache or a
//! download. Measuring and clearing run on a worker behind a row of the activity stack (measuring is
//! stoppable); parts that lose downloads or shared files ask first. The low-space rule trims the
//! thumbnail and denoise caches to half their limits under 1 GB free.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use crate::LightcraftApp;
use lightcraft_engine::activity::Cancel;
#[cfg(not(target_arch = "wasm32"))]
use lightcraft_engine::storage_report::Limits;
use lightcraft_engine::storage_report::{self, Part, Report};

/// The measuring in flight and the last report, shared with the worker.
#[derive(Default)]
struct Inner {
    report: Option<Report>,
    running: bool,
    /// Measure again (a part was cleared, or the user asked).
    stale: bool,
    /// The part whose Clear was tapped and waits for a confirmation.
    confirm: Option<Part>,
    /// A clear is running on a worker.
    clearing: bool,
    /// The last clear to finish, until the UI thread has told the user.
    done: Option<(Part, Result<storage_report::Cleared, String>)>,
}

/// Tests hold these to keep a worker at its start, so the activity row it owns can be looked at.
#[cfg(test)]
pub(crate) static MEASURE_GATE: Mutex<()> = Mutex::new(());
#[cfg(test)]
pub(crate) static CLEAR_GATE: Mutex<()> = Mutex::new(());

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Inner>>);

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn state_id() -> egui::Id {
    egui::Id::new("storage-section")
}

/// The shared state of the section, made on first use.
fn shared(ctx: &egui::Context) -> Shared {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with::<Shared>(state_id(), || Shared(Arc::new(Mutex::new(Inner { stale: true, ..Inner::default() })))).clone()
    })
}

/// Start measuring on a worker when the report is missing or stale.
#[cfg(not(target_arch = "wasm32"))]
fn measure_soon(app: &mut LightcraftApp, ctx: &egui::Context, state: &Shared) {
    {
        let mut s = state.lock();
        if s.running || !s.stale {
            return;
        }
        s.running = true;
        s.stale = false;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let guard = app.session.activity.start("storage", "Measuring storage", Cancel::Flag(cancel.clone()));
    let (layout, state2, ctx2) = (storage_report::Layout::of(&app.session), state.clone(), ctx.clone());
    let spawned = std::thread::Builder::new().name("lc-storage".into()).spawn(move || {
        #[cfg(test)]
        let _gate = MEASURE_GATE.lock().unwrap_or_else(PoisonError::into_inner);
        let report = storage_report::measure(&layout, Limits::default(), &cancel);
        let mut s = state2.lock();
        s.stale |= report.cancelled;
        s.report = (!report.cancelled).then_some(report).or(s.report.take());
        s.running = false;
        drop(s);
        drop(guard);
        ctx2.request_repaint();
    });
    if spawned.is_err() {
        let mut s = state.lock();
        s.running = false;
    }
}

/// Clear `part` on a worker behind a row of the activity stack; `tick` tells the user when it is done.
#[cfg(not(target_arch = "wasm32"))]
fn clear_soon(app: &mut LightcraftApp, ctx: &egui::Context, state: &Shared, part: Part) {
    if state.lock().clearing {
        return;
    }
    let layout = storage_report::Layout::of(&app.session);
    let job = match storage_report::clear_job(&mut app.session, &layout, part, Limits::default()) {
        Ok(job) => job,
        Err(e) => return app.toast(ctx, e),
    };
    state.lock().clearing = true;
    let guard = app.session.activity.start("storage", "Clearing storage", Cancel::No);
    let (state2, ctx2) = (state.clone(), ctx.clone());
    let spawned = std::thread::Builder::new().name("lc-storage-clear".into()).spawn(move || {
        #[cfg(test)]
        let _gate = CLEAR_GATE.lock().unwrap_or_else(PoisonError::into_inner);
        let res = job.run();
        let mut s = state2.lock();
        s.done = Some((part, res));
        s.clearing = false;
        s.stale = true;
        drop(s);
        drop(guard);
        ctx2.request_repaint();
    });
    if spawned.is_err() {
        state.lock().clearing = false;
        app.toast(ctx, "Could not start clearing");
    }
}

#[cfg(target_arch = "wasm32")]
fn clear_soon(_app: &mut LightcraftApp, _ctx: &egui::Context, _state: &Shared, _part: Part) {}

fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 { format!("{:.2} GB", mb / 1024.0) } else { format!("{mb:.1} MB") }
}

/// The Storage tab.
pub fn tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &crate::theme::Tokens) {
    use super::panels::settings::{heading, hint, row};
    let ctx = ui.ctx().clone();
    let state = shared(&ctx);
    #[cfg(not(target_arch = "wasm32"))]
    measure_soon(app, &ctx, &state);
    heading(ui, t, "Storage");
    let (report, running, confirm, clearing) = {
        let s = state.lock();
        (s.report.clone(), s.running, s.confirm, s.clearing)
    };
    let Some(report) = report else {
        hint(ui, t, if cfg!(target_arch = "wasm32") { "Not available in the browser." } else { "Measuring…" });
        return;
    };
    row(ui, t, "Total", |ui| {
        ui.label(egui::RichText::new(size_text(report.total)).color(t.text));
        let r = ui.add_enabled(!running, egui::Button::new(crate::i18n::tr("Refresh")));
        crate::widgets::register(ui.ctx(), "button:storageRefresh", r.rect);
        if r.clicked() {
            state.lock().stale = true;
        }
    });
    hint(
        ui,
        t,
        if report.counts_roots {
            "Everything LightCraft stores on this device, as the system's storage settings count it."
        } else {
            "The sum of the parts below."
        },
    );
    let (mut clear, mut asked, mut cancel_confirm) = (None, None, false);
    for part in &report.parts {
        if part.bytes == 0 && part.files == 0 && part.part == Part::Other {
            continue;
        }
        row(ui, t, part.part.label(), |ui| {
            let text = format!("{}{}", if part.truncated { "≥ " } else { "" }, size_text(part.bytes));
            ui.label(egui::RichText::new(text).color(t.text));
            if part.part.clearable() && confirm == Some(part.part) {
                let r = ui.add_enabled(!clearing, egui::Button::new(crate::i18n::tr("Confirm clear")));
                crate::widgets::register(ui.ctx(), format!("button:storageConfirm-{}", part.part.id()), r.rect);
                if r.clicked() {
                    clear = Some(part.part);
                }
                let r = ui.add(egui::Button::new(crate::i18n::tr("Cancel")));
                crate::widgets::register(ui.ctx(), "button:storageCancel", r.rect);
                if r.clicked() {
                    cancel_confirm = true;
                }
            } else if part.part.clearable() {
                let r = ui.add_enabled(part.bytes > 0 && !clearing, egui::Button::new(crate::i18n::tr("Clear")));
                crate::widgets::register(ui.ctx(), format!("button:storageClear-{}", part.part.id()), r.rect);
                if r.clicked() {
                    if part.part.needs_confirmation() {
                        asked = Some(part.part);
                    } else {
                        clear = Some(part.part);
                    }
                }
            }
        });
        if let Some(what) = part.part.consequence() {
            hint(ui, t, what);
        }
    }
    if cancel_confirm {
        state.lock().confirm = None;
    }
    if let Some(part) = asked {
        state.lock().confirm = Some(part);
    }
    if let Some(part) = clear {
        state.lock().confirm = None;
        clear_soon(app, &ctx, &state, part);
    }
    if running || clearing {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// How often the free space is looked at.
const LOW_SPACE_EVERY: f64 = 60.0;

/// Called every frame: tell the user a clear finished, and when the host can tell the free space, trim the
/// caches once it falls under 1 GB and say so.
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    let done = ctx.data(|d| d.get_temp::<Shared>(state_id())).and_then(|s| s.lock().done.take());
    if let Some((part, res)) = done {
        storage_report::after_clear(&mut app.session, part);
        match res {
            Ok(c) => app.toast(ctx, format!("{}: {} ({})", crate::i18n::tr("Cleared"), crate::i18n::tr(part.label()), size_text(c.bytes))),
            Err(e) => app.toast(ctx, e),
        }
    }
    low_space_tick(app, ctx);
}

/// The low-space rule (see [`tick`]).
pub fn low_space_tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(free) = app.services.free_space.as_ref() else { return };
    let now = ctx.input(|i| i.time);
    if app.storage_checked.is_some_and(|t| now - t < LOW_SPACE_EVERY) {
        return;
    }
    app.storage_checked = Some(now);
    let Some(dir) = app.session.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone()) else { return };
    let Some(mut trim) = storage_report::trim_caches_if_low(&mut app.session, &dir, free.as_ref()) else { return };
    let mut parts = Vec::new();
    if let Some(mb) = trim.thumbs_mb {
        parts.push(format!("{} {mb} MB", crate::i18n::tr("thumbnail cache limit lowered to")));
    }
    if let Some(mb) = trim.denoise_mb {
        parts.push(format!("{} {mb} MB", crate::i18n::tr("denoise cache trimmed to")));
    }
    let note = format!("{} ({} MB): {}.", crate::i18n::tr("Storage is nearly full"), trim.free_mb, parts.join("; "));
    if let Some(job) = trim.take_job() {
        let guard = app.session.activity.start("storage", "Trimming caches", Cancel::No);
        let spawned = std::thread::Builder::new().name("lc-storage-trim".into()).spawn(move || {
            job();
            drop(guard);
        });
        if spawned.is_err() {
            // the limit is lowered already; the next write prunes
            app.toast(ctx, "Could not start trimming");
        }
    }
    app.toast_for(ctx, note, 10.0);
}
