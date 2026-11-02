//! Phone ↔ computer handoff without a cloud (APK-62): photos edited on one device reach another
//! device's library with their edits, and back, through files the user moves (a share, a USB
//! cable or drive, a folder their own sync tool keeps).
//!
//! **Hand off** ([`prepare`] + [`HandoffJob::run`], `export.handoff`) writes the selected
//! photos' originals, untouched, with an XMP sidecar beside each (Export ▸ Original + Settings),
//! into a folder, and then [`MANIFEST`], which lists every original with its content hash, size,
//! the photo's id in the library it came from and when the photo last changed there. The
//! manifest is written last: a folder without it is a handoff that didn't finish.
//!
//! **Receive** ([`HandoffInput`] + [`scan_with`], `library.scanHandoff`; then [`apply`],
//! `library.receiveHandoff`) reads a handoff folder as Synchronize Folder reads a library folder
//! ([`crate::sync`]):
//!
//! - every original is hashed again; one that is missing or whose content no longer matches its
//!   hash is reported as damaged and left alone;
//! - an original whose content the library already has (by content hash, whatever its name or
//!   folder here: a photo renamed or moved is still found) is a **match**. Its sidecar is compared
//!   with the library: one that says nothing new is the same photo; one saved after the photo last
//!   changed here is **newer**; anything else is **older** and is never read;
//! - an original the library doesn't have is **new** and is imported (copied into the library),
//!   with its sidecar's settings.
//!
//! Reading newer settings is never the default (Synchronize Folder's rule): the person picks
//! them from the list. A photo changed in the library since the scan is refused, so a choice
//! never lands on something else than what was shown. Reading is one undo step.
//!
//! Limits: times are each device's local clock, so two devices whose clocks or time zones differ
//! by more than the time between two edits can disagree on which is newer (the list shows both
//! times); the library records when a photo's develop settings changed, not when its rating or
//! metadata did. Virtual copies and generated demo photos are not handed off (reported).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lightcraft_catalog::{Catalog, Photo, PhotoId, Source};
use serde::{Deserialize, Serialize};

use crate::sidecar::SidecarData;
use crate::{EngineError, Result, Session};

/// The manifest's file name in a handoff folder.
pub const MANIFEST: &str = "lightcraft-handoff.json";
/// The manifest's format version this build writes and reads (a newer one is refused).
pub const FORMAT: u32 = 1;
/// The largest manifest read.
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
/// The most photos a manifest may list.
const MAX_ENTRIES: usize = 200_000;

/// The handoff folder's table of contents ([`MANIFEST`]).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    /// [`FORMAT`].
    pub format: u32,
    /// The app that wrote it ("LightCraft").
    pub app: String,
    /// When it was written (the sending device's clock, ISO 8601 local).
    pub created: String,
    pub photos: Vec<Entry>,
}

/// One original in a handoff folder.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// The original's file name in the folder.
    pub file: String,
    /// Its XMP sidecar's file name in the folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<String>,
    /// The content hash of the original's bytes (as the library's duplicate check computes it).
    pub hash: String,
    pub size: u64,
    /// The photo's id in the library it was handed off from.
    pub id: u64,
    /// When the photo last changed in that library (imported or edited; ISO 8601 local).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<String>,
}

fn invalid(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

/// When photo `p` last changed in its library: the later of its import and its last edit.
pub fn changed_at(p: &Photo) -> Option<&str> {
    [Some(p.imported.as_str()), p.edited.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(|t| lightcraft_catalog::stacks::iso_seconds(t).map(|s| (s, t)))
        .max_by_key(|(s, _)| *s)
        .map(|(_, t)| t)
}

/// The content a hash stands for: a Duplicate's own hash is the original's plus `:dup<id>`.
fn base_hash(h: &str) -> &str {
    h.split(':').next().unwrap_or(h)
}

// ---------------------------------------------------------------------------------------------
// Hand off

/// A handoff set up from the session ([`prepare`]); [`HandoffJob::run`] reads and writes the files
/// without it, so it can run on a worker thread.
pub struct HandoffJob {
    dir: String,
    items: Vec<crate::export::PreparedExport>,
    /// (photo id, when it last changed), in the order of `items`.
    photos: Vec<(PhotoId, Option<String>)>,
    opts: crate::export::ExportOptions,
    created: String,
    skipped: Vec<(String, String)>,
}

/// What a handoff wrote.
#[derive(Clone, Debug, Default, Serialize)]
pub struct HandoffWritten {
    pub dir: String,
    /// The manifest's path (empty when nothing was written).
    pub manifest: String,
    /// Photos handed off.
    pub photos: usize,
    /// Every file written: originals, their sidecars, then the manifest.
    pub files: Vec<String>,
    /// (photo, why) for photos that are not handed off.
    pub skipped: Vec<(String, String)>,
    /// (photo, why) for photos whose files couldn't be written.
    pub failed: Vec<(String, String)>,
    /// Stopped before every photo was written (the manifest lists those that were).
    pub cancelled: bool,
}

/// Set up the handoff of `ids` into folder `dir` (made when missing). A folder that already holds
/// a handoff is refused: its manifest would be replaced. Reads no file.
pub fn prepare(s: &mut Session, ids: &[PhotoId], dir: &str) -> Result<HandoffJob> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Err(invalid("missing `dir`: the folder to hand the photos off into"));
    }
    if ids.is_empty() {
        return Err(invalid("no photos to hand off: select some"));
    }
    if Path::new(dir).join(MANIFEST).exists() {
        return Err(invalid(format!("{dir} already holds a handoff: choose another folder")));
    }
    let mut keep = Vec::new();
    let mut photos = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        let Some(p) = s.catalog.photo(*id) else {
            skipped.push((format!("photo {}", id.0), "no such photo".to_string()));
            continue;
        };
        let why = match &p.source {
            _ if p.copy_of.is_some() => Some("a virtual copy: hand off the photo it was copied from"),
            Source::File { .. } => None,
            _ => Some("a generated demo photo has no original file"),
        };
        if let Some(why) = why {
            skipped.push((p.file_name.clone(), why.to_string()));
            continue;
        }
        keep.push(*id);
        photos.push((*id, changed_at(p).map(str::to_string)));
    }
    // the original as it is, location and all: the other device must find the same content
    let opts = crate::export::ExportOptions {
        format: crate::export::ExportFormat::Original,
        naming: "{name}".into(),
        conflict: crate::export::Conflict::Unique,
        metadata: crate::export::MetadataPolicy::All,
        remove_location: false,
        ..Default::default()
    };
    let items = crate::export::prepare_batch(s, &keep, &opts).map_err(EngineError::Other)?;
    Ok(HandoffJob { dir: dir.to_string(), items, photos, opts, created: (s.clock)(), skipped })
}

impl HandoffJob {
    /// How many photos it hands off.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Write the originals and sidecars, then the manifest. Stops before the next photo once
    /// `cancel` is set (the manifest then lists the photos written). `progress(done, of)` is
    /// called before each photo.
    pub fn run(self, cancel: &AtomicBool, progress: &mut dyn FnMut(usize, usize)) -> Result<HandoffWritten> {
        let HandoffJob { dir, items, photos, opts, created, skipped } = self;
        let mut out = HandoffWritten { dir: dir.clone(), skipped, ..Default::default() };
        if items.is_empty() {
            return Err(invalid("nothing to hand off: none of the photos has an original file"));
        }
        std::fs::create_dir_all(&dir).map_err(|e| invalid(format!("{dir}: {e}")))?;
        let total = items.len();
        // (path → (hash, size)) of every file written
        let mut hashes: HashMap<String, (String, u64)> = HashMap::new();
        let mut write = |path: &str, bytes: &[u8]| -> std::result::Result<(), String> {
            crate::export::write_file(path, bytes)?;
            hashes.insert(path.to_string(), (lightcraft_preview::hash_bytes(bytes).to_string(), bytes.len() as u64));
            Ok(())
        };
        let to = crate::export::Destination { dir: dir.clone(), exact: None };
        let exists = |p: &str| Path::new(p).exists();
        let mut stopped = false;
        let results = crate::export::run_batch(items, &opts, &to, &mut write, &exists, false, &mut |done, _| {
            progress(done, total);
            if cancel.load(Ordering::Relaxed) {
                stopped = true;
            }
            !stopped
        })
        .map_err(EngineError::Other)?;
        out.cancelled = stopped;
        let mut entries = Vec::new();
        for ((id, edited), r) in photos.into_iter().zip(results.iter()) {
            let Some(path) = r.get("path").and_then(serde_json::Value::as_str) else {
                let file = r.get("file").and_then(serde_json::Value::as_str).unwrap_or_default();
                let why = r.get("error").and_then(serde_json::Value::as_str).unwrap_or("not written");
                out.failed.push((file.to_string(), why.to_string()));
                continue;
            };
            let Some((hash, size)) = hashes.get(path).cloned() else { continue };
            let sidecar = r
                .get("sidecars")
                .and_then(serde_json::Value::as_array)
                .and_then(|a| a.first())
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            out.files.push(path.to_string());
            out.files.extend(sidecar.clone());
            entries.push(Entry { file: file_name(path), sidecar: sidecar.as_deref().map(file_name), hash, size, id: id.0, edited });
        }
        out.photos = entries.len();
        if entries.is_empty() {
            return Ok(out);
        }
        let manifest = Manifest { format: FORMAT, app: "LightCraft".into(), created, photos: entries };
        let text = serde_json::to_vec_pretty(&manifest).map_err(|e| invalid(e.to_string()))?;
        let path = Path::new(&dir).join(MANIFEST).to_string_lossy().into_owned();
        crate::export::write_file(&path, &text).map_err(EngineError::Other)?;
        out.files.push(path.clone());
        out.manifest = path;
        Ok(out)
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// [`prepare`] and [`HandoffJob::run`] in turn (the CLI, MCP and scripts).
pub fn hand_off(s: &mut Session, ids: &[PhotoId], dir: &str) -> Result<HandoffWritten> {
    prepare(s, ids, dir)?.run(&AtomicBool::new(false), &mut |_, _| {})
}

// ---------------------------------------------------------------------------------------------
// Receive

/// What a scan of a handoff folder needs, taken from the session so it can run on a worker
/// thread.
pub struct HandoffInput {
    dir: String,
    library: Arc<Catalog>,
}

impl HandoffInput {
    pub fn new(s: &Session, dir: &str) -> Result<HandoffInput> {
        let dir = dir.trim();
        if dir.is_empty() {
            return Err(invalid("missing `dir`: the handoff folder"));
        }
        Ok(HandoffInput { dir: dir.to_string(), library: Arc::new(s.catalog.clone()) })
    }
}

/// An original of the handoff the library doesn't have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandoffNew {
    pub path: String,
    pub name: String,
}

/// An original of the handoff the library has (by content), and how their settings compare.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandoffMatch {
    /// The library's photo.
    pub id: u64,
    /// The library photo's file name.
    pub name: String,
    /// The original in the handoff folder.
    pub path: String,
    /// When the photo last changed on the device it came from, and here.
    pub handoff_edited: Option<String>,
    pub library_edited: Option<String>,
}

/// What a handoff folder holds for this library ([`scan_with`]).
#[derive(Clone, Debug, Default, Serialize)]
pub struct HandoffPlan {
    pub dir: String,
    /// When the handoff was written (the sending device's clock).
    pub created: String,
    /// Originals to import.
    pub new: Vec<HandoffNew>,
    /// Photos whose handoff settings are newer than the library's: read only when chosen.
    pub newer: Vec<HandoffMatch>,
    /// Photos whose library settings are newer (or as new): never read.
    pub older: Vec<HandoffMatch>,
    /// Photos the handoff says nothing new about.
    pub same: usize,
    /// (file, why) for originals or sidecars that are missing, unreadable or changed since the
    /// handoff was written.
    pub damaged: Vec<(String, String)>,
    /// The newer photos' sidecars, read and parsed.
    #[serde(skip)]
    sidecars: HashMap<u64, SidecarData>,
    /// The matched photos as the scan saw them: a choice is refused for one changed since.
    #[serde(skip)]
    seen: HashMap<u64, Arc<Photo>>,
}

/// Whether `name` is a plain file name: no folder, no `..`, nothing absolute.
fn plain_file(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(std::path::Component::Normal(_)), None)) && !name.contains(['/', '\\'])
}

/// Read and check the manifest of handoff folder `dir`.
pub fn read_manifest(dir: &str) -> Result<Manifest> {
    let path = Path::new(dir).join(MANIFEST);
    let meta = std::fs::metadata(&path).map_err(|_| invalid(format!("{dir} is not a handoff folder: it has no {MANIFEST}")))?;
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(invalid(format!("{}: too large for a handoff manifest", path.display())));
    }
    let text = std::fs::read(&path).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    let m: Manifest = serde_json::from_slice(&text).map_err(|e| invalid(format!("{}: not a handoff manifest ({e})", path.display())))?;
    if m.format > FORMAT {
        return Err(invalid(format!("{}: written by a newer LightCraft (format {}): update LightCraft to receive it", path.display(), m.format)));
    }
    if m.photos.len() > MAX_ENTRIES {
        return Err(invalid(format!("{}: lists more than {MAX_ENTRIES} photos", path.display())));
    }
    Ok(m)
}

/// Read handoff folder `input.dir` against the library (see the module docs). Reads the disk,
/// never the session. Stops early (with what it has) once `cancel` is set; `progress(done, of)`
/// after each original.
pub fn scan_with(input: HandoffInput, cancel: &AtomicBool, progress: &mut dyn FnMut(usize, usize)) -> Result<HandoffPlan> {
    let HandoffInput { dir, library } = input;
    let manifest = read_manifest(&dir)?;
    // the library's photos by content (a photo's own record before its virtual copies)
    let mut by_hash: HashMap<&str, Vec<&Arc<Photo>>> = HashMap::new();
    for p in library.photos().filter(|p| p.in_library() && p.copy_of.is_none()) {
        if let Some(h) = p.content_hash.as_deref().map(base_hash).filter(|h| !h.is_empty()) {
            by_hash.entry(h).or_default().push(p);
        }
    }
    let mut plan = HandoffPlan { dir: dir.clone(), created: manifest.created.clone(), ..Default::default() };
    let total = manifest.photos.len();
    let mut taken = HashSet::new();
    for (i, e) in manifest.photos.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(i, total);
        if !plain_file(&e.file) || e.sidecar.as_deref().is_some_and(|s| !plain_file(s)) {
            plan.damaged.push((e.file.clone(), "not a file name in the handoff folder".into()));
            continue;
        }
        let path = Path::new(&dir).join(&e.file);
        let shown = path.to_string_lossy().into_owned();
        match crate::import_move::hash_file(&path) {
            Ok(h) if h.to_string() == e.hash => {}
            Ok(_) => {
                plan.damaged.push((shown, "its content changed since it was handed off".into()));
                continue;
            }
            Err(err) => {
                plan.damaged.push((shown, err.to_string()));
                continue;
            }
        }
        // the library's photo with this content: the one with the manifest's id when there are
        // several (a handoff coming back to the library it left), else the first not yet taken
        let found = by_hash.get(e.hash.as_str()).and_then(|ps| {
            ps.iter().find(|p| p.id.0 == e.id && !taken.contains(&p.id)).or_else(|| ps.iter().find(|p| !taken.contains(&p.id))).copied()
        });
        let Some(p) = found else {
            plan.new.push(HandoffNew { path: shown, name: e.file.clone() });
            continue;
        };
        taken.insert(p.id);
        let Some(sc_name) = &e.sidecar else {
            plan.same += 1;
            continue;
        };
        let sc_path = Path::new(&dir).join(sc_name);
        let sc = match crate::sync::read_sidecar_file(&sc_path, crate::crs::Target::for_photo(p)) {
            Ok(sc) => sc,
            Err(err) => {
                plan.damaged.push((sc_path.to_string_lossy().into_owned(), err));
                continue;
            }
        };
        if !crate::sync::sidecar_differs(&library, p, &sc) {
            plan.same += 1;
            continue;
        }
        let here = changed_at(p);
        let m = HandoffMatch {
            id: p.id.0,
            name: p.file_name.clone(),
            path: shown,
            handoff_edited: e.edited.clone(),
            library_edited: here.map(str::to_string),
        };
        let secs = |t: Option<&str>| t.and_then(lightcraft_catalog::stacks::iso_seconds);
        // newer only when both times are known and the handoff's is later: an unknown time is
        // never a reason to overwrite
        let newer = matches!((secs(e.edited.as_deref()), secs(here)), (Some(a), Some(b)) if a > b);
        plan.seen.insert(p.id.0, Arc::clone(p));
        if newer {
            plan.sidecars.insert(p.id.0, sc);
            plan.newer.push(m);
        } else {
            plan.older.push(m);
        }
    }
    progress(total, total);
    Ok(plan)
}

/// [`HandoffInput::new`] and [`scan_with`] in turn.
pub fn scan(s: &Session, dir: &str) -> Result<HandoffPlan> {
    scan_with(HandoffInput::new(s, dir)?, &AtomicBool::new(false), &mut |_, _| {})
}

/// What [`apply`] does with a plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HandoffChoice {
    /// Import the new originals (copied into the library).
    pub import_new: bool,
    /// The photos whose newer settings are read (each must be listed as newer).
    pub read: Vec<PhotoId>,
}

/// What [`apply`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HandoffReport {
    pub imported: usize,
    pub read: usize,
    /// (photo, why) for photos asked for that were not read.
    pub refused: Vec<(u64, String)>,
    /// (file or photo, why) for what failed.
    pub failed: Vec<(String, String)>,
}

/// How a handoff's new originals come in: copied into the library's own folder (a handoff folder
/// is a carrier, often on a drive that goes away), or added where they are in a library without
/// one.
pub fn import_mode(s: &Session) -> crate::import::ImportMode {
    if s.library.as_ref().is_some_and(|l| l.on_disk) { crate::import::ImportMode::Copy } else { crate::import::ImportMode::Add }
}

/// The new originals' paths of a plan (for an import run elsewhere, e.g. in the background).
pub fn new_paths(plan: &HandoffPlan) -> Vec<String> {
    plan.new.iter().map(|n| n.path.clone()).collect()
}

/// Read the chosen newer settings into the library (one undo step, "Receive Handoff"), then,
/// with `import_new`, import the new originals ([`import_mode`], their sidecars read).
/// Settings the library has newer are never read, whatever is asked; nor are those of a photo
/// changed since the scan.
pub fn apply(s: &mut Session, mut plan: HandoffPlan, choice: &HandoffChoice) -> Result<HandoffReport> {
    let mut report = HandoffReport::default();
    let undo0 = s.undo.len();
    let mut read = Vec::new();
    let mut asked = HashSet::new();
    for id in &choice.read {
        if !asked.insert(*id) {
            continue;
        }
        let why = if plan.older.iter().any(|m| m.id == id.0) {
            Some("the library's settings are as new or newer: kept")
        } else if !plan.newer.iter().any(|m| m.id == id.0) {
            Some("not listed as newer in this handoff")
        } else if !plan.seen.get(&id.0).is_some_and(|seen| s.catalog.photo(*id).is_some_and(|now| Arc::ptr_eq(seen, now) || **seen == **now)) {
            Some("changed in the library since the handoff was read: read it again")
        } else {
            None
        };
        if let Some(why) = why {
            report.refused.push((id.0, why.to_string()));
            continue;
        }
        let path = plan.newer.iter().find(|m| m.id == id.0).map(|m| m.path.clone()).unwrap_or_default();
        match plan.sidecars.remove(&id.0) {
            Some(sc) => read.push((*id, path, Ok(sc))),
            None => report.refused.push((id.0, "its settings were already read".into())),
        }
    }
    let (n, failed) = crate::sync::commit_sidecars(s, "Receive Handoff", read);
    report.read = n;
    report.failed.extend(failed);
    if choice.import_new && !plan.new.is_empty() {
        let opts = crate::import::ImportOptions { mode: import_mode(s), ..Default::default() };
        match crate::import::import_with(s, &new_paths(&plan), &opts) {
            Ok(r) => {
                report.imported = r.imported.len();
                report.failed.extend(r.failed);
            }
            Err(e) => report.failed.push((plan.dir.clone(), e.to_string())),
        }
    }
    let steps = s.undo.len().saturating_sub(undo0);
    s.merge_undo(steps, "Receive Handoff");
    Ok(report)
}

/// The handoff folder a hand-off into `base` makes: `base/LightCraft Handoff <time>` (the time
/// from `now`, ISO 8601), with ` 2`, ` 3`… when that folder is there already.
pub fn folder_in(base: &str, now: &str) -> PathBuf {
    let stamp: String = now
        .chars()
        .take(19)
        .map(|c| {
            if c == ':' {
                '-'
            } else if c == 'T' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let stamp = stamp.trim();
    let name = if stamp.is_empty() { "LightCraft Handoff".to_string() } else { format!("LightCraft Handoff {stamp}") };
    let first = Path::new(base).join(&name);
    if !first.exists() {
        return first;
    }
    (2..1000).map(|n| Path::new(base).join(format!("{name} {n}"))).find(|p| !p.exists()).unwrap_or(first)
}
