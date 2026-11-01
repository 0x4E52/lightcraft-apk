//! Auto Settings and Update AI Masks on a selection (`develop.auto {ids}`, `mask.updateAi`), APK-60.
//!
//! - **Auto Settings**: each photo gets its own analysis, from its own thumbnail-level source, as `develop.auto` does
//!   for the active photo. The sources are decoded and analysed on a worker thread (one photo at a time); a photo
//!   whose file can't be read is reported with the reason and skipped.
//! - **Update AI Masks**: a Subject, Sky, Background, People or Landscape mask is computed from each photo's own
//!   pixels whenever it renders, so after a paste it already follows the photo it is on (counted as `live`). What a
//!   paste carries over unchanged is a SAM 3 selection (Describe text or Object clicks with their segmentation): each
//!   is computed again on its photo by the model's own worker where the model is installed and the device has the
//!   memory for it (the app says when not: `samUnavailable`, which it always sets there, over the caller's value), else it is kept as it was and the photo is reported.
//! - **One undo step**: the results are applied on the calling (UI) thread when the last photo is done, all photos in
//!   one history entry; a selection whose setting changed meanwhile keeps the newer change for that mask. Cancelled
//!   (✕ on the activity row) or another library opened: nothing changes.
//! - **Runs** with a row in the activity stack (`Cancel::Flag`). With `background` (the app) the command returns at
//!   once and the per-frame `batch.pump` applies the outcome; otherwise (CLI, headless MCP, tests) it waits for it. The
//!   app always runs them in the background, whatever the caller sent: waiting would block its UI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_develop::{DevelopSettings, MaskShape, SegMask};
use lightcraft_pipeline::auto::AutoTone;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::activity::{Cancel, TaskGuard};
use crate::media::{SourceLevel, SourceRef};
use crate::{EngineError, Result, Session};

/// Most photos one call takes (a library-wide selection is fine; an absurd id list is refused).
pub const MAX_PHOTOS: usize = 100_000;

/// What the model gives back for one selection: the new segmentation, `None` when nothing matched.
pub(crate) type SamResult = std::result::Result<Option<SegMask>, String>;

/// A stand-in for SAM 3 (tests): computes a selection for a photo at once.
#[cfg(test)]
pub(crate) type FakeSam = Arc<dyn Fn(PhotoId, &MaskShape) -> SamResult + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Auto,
    AiMasks,
}

impl Kind {
    /// English; the frontend translates it.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Auto => "Auto Settings",
            Kind::AiMasks => "Update AI Masks",
        }
    }
}

/// A photo the job left alone, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub name: String,
    pub reason: String,
}

/// How a job ended (`batch.pump`'s `done`, the waiting command's result).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub kind: Kind,
    /// Photos the job was asked to look at.
    pub photos: usize,
    /// Photos whose settings changed.
    pub changed: usize,
    /// Photos that could not be read (or failed otherwise): nothing changed on them.
    pub skipped: Vec<Note>,
    /// Update AI Masks: photos whose SAM 3 selections were kept as they were.
    pub kept: Vec<Note>,
    /// Update AI Masks: Subject / Sky / Background / People / Landscape masks, which follow each photo by themselves.
    pub live: usize,
    pub cancelled: bool,
}

impl Summary {
    fn new(kind: Kind, photos: usize) -> Summary {
        Summary { kind, photos, changed: 0, skipped: Vec::new(), kept: Vec::new(), live: 0, cancelled: false }
    }
}

/// The batch jobs of a session: at most one runs at a time.
#[derive(Default)]
pub struct BatchAi {
    job: Option<Job>,
    /// The last job's outcome, until `batch.pump` hands it out.
    done: Option<Summary>,
    #[cfg(test)]
    pub(crate) fake_sam: Option<FakeSam>,
    /// Tests: the stand-in model never answers (it holds the senders here).
    #[cfg(test)]
    pub(crate) stalled: Option<Vec<mpsc::Sender<SamResult>>>,
    /// Tests: how long a waiting command waits (default [`crate::segment::WAIT`]).
    #[cfg(test)]
    pub(crate) wait: Option<std::time::Duration>,
}

impl BatchAi {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// A job has ended and `batch.pump` hasn't handed out its summary yet.
    pub fn has_outcome(&self) -> bool {
        self.done.is_some()
    }
}

/// What the Auto worker sends per photo.
enum Reply {
    Tone(PhotoId, AutoTone),
    Failed(PhotoId, String),
}

/// One SAM 3 selection being computed again.
struct SamRequest {
    photo: PhotoId,
    mask: u32,
    comp: usize,
    /// The shape it was computed from (without detail passes): applied only if the mask still has it.
    shape: MaskShape,
    rx: SamRx,
}

enum SamRx {
    /// A result computed elsewhere (the tests' stand-in for the model).
    #[cfg_attr(not(test), allow(dead_code))]
    Plain(mpsc::Receiver<SamResult>),
    #[cfg(feature = "sam")]
    Model(crate::segment::Pending),
}

impl SamRx {
    /// The result once it is there; a worker that went away is an error.
    fn take(&self) -> Option<SamResult> {
        match self {
            SamRx::Plain(rx) => match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("the AI model stopped before it was done".into())),
            },
            #[cfg(feature = "sam")]
            SamRx::Model(p) => p.take(),
        }
    }
}

struct Job {
    kind: Kind,
    identity: Arc<()>,
    cancel: Arc<AtomicBool>,
    guard: TaskGuard,
    total: u64,
    done: u64,
    /// Auto: the worker's replies (`None` once it has sent them all).
    rx: Option<mpsc::Receiver<Reply>>,
    tones: Vec<(PhotoId, AutoTone)>,
    /// Update AI Masks: selections still being computed, then the ones that are.
    sam: Vec<SamRequest>,
    segs: Vec<(PhotoId, u32, usize, MaskShape, SegMask)>,
    summary: Summary,
}

fn name_of(s: &Session, id: PhotoId) -> String {
    s.catalog.photo(id).map(|p| p.file_name.clone()).unwrap_or_else(|| format!("photo {}", id.0))
}

fn note(s: &Session, id: PhotoId, reason: impl Into<String>) -> Note {
    Note { id: id.0, name: name_of(s, id), reason: reason.into() }
}

fn bad(c: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: c.into(), msg: msg.into() }
}

/// The photos a call acts on: its `ids` / `id`, else the selection; each must be in the library.
fn targets(s: &Session, p: &Value, c: &str) -> Result<Vec<PhotoId>> {
    let mut ids = s.targets(p).map_err(|e| bad(c, e.to_string()))?;
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id));
    if ids.is_empty() {
        return Err(bad(c, "no photos selected"));
    }
    if ids.len() > MAX_PHOTOS {
        return Err(bad(c, format!("at most {MAX_PHOTOS} photos at once")));
    }
    if let Some(missing) = ids.iter().find(|id| s.catalog.photo(**id).is_none()) {
        return Err(bad(c, format!("no photo {}", missing.0)));
    }
    Ok(ids)
}

fn refuse_if_running(s: &Session, c: &str) -> Result<()> {
    match &s.batch_ai.job {
        Some(j) => Err(EngineError::Other(format!("{c}: {} is still running ({} of {})", j.kind.label(), j.done, j.total))),
        None => Ok(()),
    }
}

/// `develop.auto` with `ids` (or `id`, or `selection: true`): Auto Settings on each of those photos.
pub(crate) fn start_auto(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "develop.auto";
    refuse_if_running(s, c)?;
    let ids = targets(s, p, c)?;
    let mut work = Vec::with_capacity(ids.len());
    for id in &ids {
        let Some(photo) = s.catalog.photo(*id).cloned() else { continue };
        let source = s.media.source_ref(&photo, SourceLevel::Thumb);
        let header = crate::media::source_info(&photo);
        work.push((*id, source, header, photo.develop.clone()));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    run_auto_worker(work, tx, cancel.clone());
    let guard = s.activity.start("auto", Kind::Auto.label(), Cancel::Flag(cancel.clone()));
    guard.progress(0, ids.len() as u64);
    let job = Job {
        kind: Kind::Auto,
        identity: Arc::clone(&s.library_identity),
        cancel,
        guard,
        total: ids.len() as u64,
        done: 0,
        rx: Some(rx),
        tones: Vec::new(),
        sam: Vec::new(),
        segs: Vec::new(),
        summary: Summary::new(Kind::Auto, ids.len()),
    };
    s.batch_ai.job = Some(job);
    finish_or_return(s, p)
}

type AutoWork = Vec<(PhotoId, SourceRef, lightcraft_pipeline::SourceInfo, Arc<DevelopSettings>)>;

/// Analyse each photo: on a thread of its own where there are threads, else (wasm) here.
fn run_auto_worker(work: AutoWork, tx: mpsc::Sender<Reply>, cancel: Arc<AtomicBool>) {
    let job = move || {
        for (id, source, header, develop) in work {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            // a decoder bug in one file is that photo's problem, not the job's end
            let reply = match crate::guard::catch("analysing a photo for Auto Settings", || {
                source.load_source().map(|r| lightcraft_pipeline::auto::auto_tone(&r.image, &r.info_or(header), &develop))
            }) {
                Ok(Ok(tone)) => Reply::Tone(id, tone),
                Ok(Err(e)) => Reply::Failed(id, e),
                Err(e) => Reply::Failed(id, e),
            };
            if tx.send(reply).is_err() {
                return;
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        // the closure comes back when no thread can be started: then it runs here
        let job = Arc::new(std::sync::Mutex::new(Some(job)));
        let worker = Arc::clone(&job);
        let started = std::thread::Builder::new().name("auto-settings".into()).spawn(move || {
            if let Some(f) = worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                f();
            }
        });
        if started.is_err()
            && let Some(f) = job.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
        {
            f();
        }
    }
    #[cfg(target_arch = "wasm32")]
    job();
}

/// Whether SAM 3 can compute selections here; the reason when not.
fn sam_reason(s: &Session, p: &Value) -> Option<String> {
    #[cfg(test)]
    if s.batch_ai.fake_sam.is_some() || s.batch_ai.stalled.is_some() {
        return p.get("samUnavailable").and_then(Value::as_str).map(str::to_string);
    }
    if let Some(r) = p.get("samUnavailable").and_then(Value::as_str).filter(|r| !r.trim().is_empty()) {
        return Some(r.to_string());
    }
    if !crate::segment::Segmenter::AVAILABLE {
        return Some("AI masks are not available in this build".into());
    }
    s.segmenter.require_model().err().map(|e| e.to_string())
}

/// Ask for one selection to be computed again on its photo.
fn sam_submit(s: &mut Session, photo: PhotoId, shape: &MaskShape, cancel: &Arc<AtomicBool>) -> std::result::Result<SamRx, String> {
    #[cfg(test)]
    if let Some(held) = &mut s.batch_ai.stalled {
        let (tx, rx) = mpsc::channel();
        held.push(tx);
        return Ok(SamRx::Plain(rx));
    }
    #[cfg(test)]
    if let Some(fake) = s.batch_ai.fake_sam.clone() {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(fake(photo, shape));
        return Ok(SamRx::Plain(rx));
    }
    #[cfg(feature = "sam")]
    return s.segment_submit(photo, shape, cancel).map(SamRx::Model);
    #[cfg(not(feature = "sam"))]
    {
        let _ = (s, photo, shape, cancel);
        Err("AI masks are not available in this build".into())
    }
}

/// A SAM 3 selection that can be computed again: Describe text, or Object clicks.
fn recomputable(shape: &MaskShape) -> bool {
    match shape {
        MaskShape::Prompt { text, .. } => !text.trim().is_empty(),
        MaskShape::Object { hint, .. } => !hint.is_empty(),
        _ => false,
    }
}

/// Computed from each photo's pixels when it renders: nothing stored to update.
fn follows_photo(shape: &MaskShape) -> bool {
    matches!(shape, MaskShape::Subject | MaskShape::Sky | MaskShape::Background | MaskShape::People { .. } | MaskShape::Landscape { .. })
}

/// `shape` as the model computes it: no detail passes, no edge setting.
fn bare(mut shape: MaskShape) -> MaskShape {
    if let MaskShape::Object { detail, edge, .. } | MaskShape::Prompt { detail, edge, .. } = &mut shape {
        detail.clear();
        *edge = 0.0;
    }
    shape
}

/// `mask.updateAi`: compute the AI masks of each photo again.
pub(crate) fn start_ai_masks(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "mask.updateAi";
    refuse_if_running(s, c)?;
    let ids = targets(s, p, c)?;
    let reason = sam_reason(s, p);
    let cancel = Arc::new(AtomicBool::new(false));
    let guard = s.activity.start("aiMasks", Kind::AiMasks.label(), Cancel::Flag(cancel.clone()));
    let mut summary = Summary::new(Kind::AiMasks, ids.len());
    let mut sam = Vec::new();
    for id in &ids {
        let Some(d) = s.develop_of(*id) else { continue };
        let mut kept: Option<String> = None;
        for m in &d.masks {
            for (k, comp) in m.components.iter().enumerate() {
                if follows_photo(&comp.shape) {
                    summary.live += 1;
                } else if recomputable(&comp.shape) {
                    if let Some(r) = &reason {
                        kept.get_or_insert_with(|| r.clone());
                        continue;
                    }
                    match sam_submit(s, *id, &comp.shape, &cancel) {
                        Ok(rx) => {
                            sam.push(SamRequest { photo: *id, mask: m.id, comp: k, shape: bare(comp.shape.clone()), rx });
                        }
                        Err(e) => {
                            kept.get_or_insert(e);
                        }
                    }
                }
            }
        }
        if let Some(r) = kept {
            let n = note(s, *id, format!("its AI selections were kept as they were: {r}"));
            summary.kept.push(n);
        }
    }
    let total = sam.len() as u64;
    guard.progress(0, total);
    s.batch_ai.job = Some(Job {
        kind: Kind::AiMasks,
        identity: Arc::clone(&s.library_identity),
        cancel,
        guard,
        total,
        done: 0,
        rx: None,
        tones: Vec::new(),
        sam,
        segs: Vec::new(),
        summary,
    });
    finish_or_return(s, p)
}

/// With `background`: return at once (`batch.pump` finishes the job). Otherwise wait for it and return its summary.
fn finish_or_return(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("background").and_then(Value::as_bool).unwrap_or(false) {
        let (kind, photos) = s.batch_ai.job.as_ref().map_or((Kind::Auto, 0), |j| (j.kind, j.summary.photos));
        // nothing to wait for (no SAM selections at all): done at once
        step(s);
        return Ok(json!({"started": true, "kind": kind, "photos": photos}));
    }
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut wait = crate::segment::WAIT;
    #[cfg(test)]
    if let Some(w) = s.batch_ai.wait {
        wait = w;
    }
    let started = web_time::Instant::now();
    loop {
        if !step(s) {
            break;
        }
        // a model that stays alive but never answers doesn't hang the command (as `segment_wait`)
        if started.elapsed() >= wait {
            give_up_on_sam(s);
        }
        // (wasm runs the work inline: it is done by the first step)
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let done = s.batch_ai.done.take().ok_or_else(|| EngineError::Other("the batch ended without a result".into()))?;
    if let Some(e) = applied_error(&done) {
        return Err(EngineError::Other(e));
    }
    serde_json::to_value(done).map_err(|e| EngineError::Other(e.to_string()))
}

/// The selections the model hasn't answered yet are reported as skipped; the next step ends the job with the rest.
fn give_up_on_sam(s: &mut Session) {
    let Some(mut job) = s.batch_ai.job.take() else { return };
    for r in std::mem::take(&mut job.sam) {
        job.done += 1;
        let n = note(s, r.photo, "its AI selection could not be computed: the AI model took too long; try again");
        job.summary.skipped.push(n);
    }
    s.batch_ai.job = Some(job);
}

/// A summary whose commit failed carries the error as its only skipped note with id 0.
fn applied_error(s: &Summary) -> Option<String> {
    (s.changed == 0).then(|| s.skipped.iter().find(|n| n.id == 0).map(|n| n.reason.clone())).flatten()
}

/// Take in what is done; apply it all when the job ends. Returns whether a job is still running.
pub(crate) fn step(s: &mut Session) -> bool {
    let Some(mut job) = s.batch_ai.job.take() else { return false };
    let mut finished = false;
    if let Some(rx) = &job.rx {
        loop {
            match rx.try_recv() {
                Ok(Reply::Tone(id, tone)) => {
                    job.done += 1;
                    job.tones.push((id, tone));
                }
                Ok(Reply::Failed(id, reason)) => {
                    job.done += 1;
                    let n = note(s, id, format!("could not be read: {reason}"));
                    job.summary.skipped.push(n);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
    } else {
        let mut left = Vec::with_capacity(job.sam.len());
        for r in std::mem::take(&mut job.sam) {
            match r.rx.take() {
                None => left.push(r),
                Some(result) => {
                    job.done += 1;
                    match result {
                        Ok(Some(seg)) => job.segs.push((r.photo, r.mask, r.comp, r.shape, seg)),
                        Ok(None) => {
                            let what = match &r.shape {
                                MaskShape::Prompt { text, .. } => format!("nothing matching “{text}” was found; that selection was kept"),
                                _ => "the clicked object was not found; that selection was kept".into(),
                            };
                            let n = note(s, r.photo, what);
                            job.summary.kept.push(n);
                        }
                        Err(e) => {
                            let n = note(s, r.photo, format!("its AI selection could not be computed: {e}"));
                            job.summary.skipped.push(n);
                        }
                    }
                }
            }
        }
        job.sam = left;
        finished = job.sam.is_empty();
    }
    job.guard.progress(job.done, job.total);
    let cancelled = job.cancel.load(Ordering::Relaxed);
    if !finished && !cancelled {
        s.batch_ai.job = Some(job);
        return true;
    }
    let mut summary = std::mem::replace(&mut job.summary, Summary::new(job.kind, 0));
    summary.cancelled = cancelled;
    if cancelled || !Arc::ptr_eq(&job.identity, &s.library_identity) {
        // nothing changes: the photos are as they were (another library: these ids are other photos)
        summary.cancelled = true;
    } else if let Err(e) = apply(s, &mut job, &mut summary) {
        summary.changed = 0;
        summary.skipped.insert(0, Note { id: 0, name: String::new(), reason: e.to_string() });
    }
    drop(job.guard);
    s.batch_ai.done = Some(summary);
    false
}

/// Apply a finished job's results as one undo step.
fn apply(s: &mut Session, job: &mut Job, summary: &mut Summary) -> Result<()> {
    let label = job.kind.label();
    let mut ops = Vec::new();
    match job.kind {
        Kind::Auto => {
            for (id, a) in std::mem::take(&mut job.tones) {
                let Some(d) = s.develop_of(id) else { continue };
                let mut d = (*d).clone();
                d.light.exposure = a.exposure;
                d.light.contrast = a.contrast;
                d.light.highlights = a.highlights;
                d.light.shadows = a.shadows;
                d.light.whites = a.whites;
                d.light.blacks = a.blacks;
                d.color.vibrance = a.vibrance;
                d.color.saturation = a.saturation;
                if let Some(op) = s.develop_op(id, d, label) {
                    ops.push(op);
                }
            }
        }
        Kind::AiMasks => {
            let mut by_photo: Vec<(PhotoId, DevelopSettings)> = Vec::new();
            for (id, mask, comp, shape, seg) in std::mem::take(&mut job.segs) {
                let i = match by_photo.iter().position(|(x, _)| *x == id) {
                    Some(i) => i,
                    None => {
                        let Some(d) = s.develop_of(id) else { continue };
                        by_photo.push((id, (*d).clone()));
                        by_photo.len() - 1
                    }
                };
                let Some((_, d)) = by_photo.get_mut(i) else { continue };
                let Some(c) = d.masks.iter_mut().find(|m| m.id == mask).and_then(|m| m.components.get_mut(comp)) else { continue };
                // changed meanwhile (new clicks, another text): the newer change stays
                if bare(c.shape.clone()) != shape {
                    continue;
                }
                if let MaskShape::Object { seg: s0, detail, .. } | MaskShape::Prompt { seg: s0, detail, .. } = &mut c.shape {
                    *s0 = Some(seg);
                    detail.clear();
                }
            }
            for (id, d) in by_photo {
                if s.develop_of(id).is_some_and(|old| *old == d) {
                    continue;
                }
                if let Some(op) = s.develop_op(id, d, label) {
                    ops.push(op);
                }
            }
        }
    }
    summary.changed = ops.len();
    if ops.is_empty() {
        return Ok(());
    }
    s.commit(label, Op::Batch { ops })
}

/// `batch.pump`: take in what the running job has done; `done` once, when it has ended.
pub(crate) fn pump(s: &mut Session) -> Result<Value> {
    let active = step(s);
    let mut out = json!({"active": active});
    if let Some(j) = &s.batch_ai.job {
        out["running"] = json!({"kind": j.kind, "done": j.done, "total": j.total});
    }
    if let Some(d) = s.batch_ai.done.take() {
        out["done"] = serde_json::to_value(d).unwrap_or_default();
    }
    Ok(out)
}
