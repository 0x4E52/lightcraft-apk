//! What LightCraft keeps on a device, part by part: the Storage page (Settings) and the
//! `library.storage` / `library.clearStorage` commands.
//!
//! A [`Layout`] names the folders of each [`Part`]; [`measure`] walks them (bounded by depth and by
//! entries, cancellable, symlinks never followed) and [`clear`] empties the parts that are only
//! caches or downloads. The total is the walk of the host's private folders (`roots`: on Android
//! `files/` and `cache/`), so it agrees with Settings ▸ Apps ▸ LightCraft ▸ Storage within the
//! system's own rounding; whatever no part claims is listed as "Other". Without roots the total is
//! the sum of the parts.
//!
//! The folders that may be measured and cleared are the session's own and what the host registered once
//! ([`HostDirs`], set by the app from its services): a command's parameters never name a folder, so no
//! caller can point a clear at its own files.
//!
//! [`trim_caches_if_low`] is the low-space rule: under [`LOW_SPACE_BYTES`] free, the thumbnail and
//! denoise caches are trimmed to half their limits. [`free_bytes`] is the probe hosts give it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::Session;

/// Free space under which the caches are trimmed.
pub const LOW_SPACE_BYTES: u64 = 1 << 30;

/// What the host registered: its private folders (the total counts them), its log folder and its Inbox.
/// Only these (plus the session's library and model folders) are ever measured or emptied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostDirs {
    pub roots: Vec<PathBuf>,
    pub logs: Option<PathBuf>,
    pub inbox: Option<PathBuf>,
}

/// How far and how wide one walk goes. A bigger tree is counted up to the limit and flagged
/// `truncated`.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_depth: usize,
    pub max_entries: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_depth: 24, max_entries: 2_000_000 }
    }
}

/// One line of the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Part {
    /// The catalog and everything else in the library folder that no other part claims.
    Library,
    /// Photos imported with "Copy into library".
    Originals,
    /// Thumbnail and preview cache.
    Thumbnails,
    SmartPreviews,
    DenoiseCache,
    /// SAM, face and denoise models.
    Models,
    /// The GeoNames place names.
    Places,
    Logs,
    Inbox,
    /// Short-lived files (`config::scratch_dir`).
    Temp,
    /// Inside the counted folders but in none of the parts above.
    Other,
}

impl Part {
    pub const ALL: [Part; 11] = [
        Part::Library,
        Part::Originals,
        Part::Thumbnails,
        Part::SmartPreviews,
        Part::DenoiseCache,
        Part::Models,
        Part::Places,
        Part::Logs,
        Part::Inbox,
        Part::Temp,
        Part::Other,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Part::Library => "library",
            Part::Originals => "originals",
            Part::Thumbnails => "thumbnails",
            Part::SmartPreviews => "smartPreviews",
            Part::DenoiseCache => "denoiseCache",
            Part::Models => "models",
            Part::Places => "places",
            Part::Logs => "logs",
            Part::Inbox => "inbox",
            Part::Temp => "temp",
            Part::Other => "other",
        }
    }

    pub fn from_id(id: &str) -> Option<Part> {
        Part::ALL.into_iter().find(|p| p.id() == id)
    }

    /// English; the frontend translates it.
    pub fn label(self) -> &'static str {
        match self {
            Part::Library => "Library (catalog)",
            Part::Originals => "Copied originals",
            Part::Thumbnails => "Thumbnail and preview cache",
            Part::SmartPreviews => "Smart previews",
            Part::DenoiseCache => "Denoise cache",
            Part::Models => "AI models",
            Part::Places => "Place names",
            Part::Logs => "Logs",
            Part::Inbox => "Shared files waiting to be imported",
            Part::Temp => "Temporary files",
            Part::Other => "Other",
        }
    }

    /// Whether clearing it loses nothing the user made. Originals and the library never; place names and
    /// temporary files are small and not offered.
    pub fn clearable(self) -> bool {
        matches!(self, Part::Thumbnails | Part::SmartPreviews | Part::DenoiseCache | Part::Models | Part::Logs | Part::Inbox)
    }

    /// Whether the user is asked first: clearing loses downloads or files that are not kept anywhere else.
    pub fn needs_confirmation(self) -> bool {
        matches!(self, Part::Models | Part::Inbox | Part::SmartPreviews)
    }

    /// What stops working (or is made again) once the part is cleared; English.
    pub fn consequence(self) -> Option<&'static str> {
        match self {
            Part::Thumbnails => Some("Thumbnails and previews are made again as you browse."),
            Part::SmartPreviews => Some("Photos whose original is offline can't be edited until it is back."),
            Part::DenoiseCache => Some("Denoised pictures are made again when wanted."),
            Part::Models => Some("AI masks, face recognition and AI Denoise stop working until their models are downloaded again."),
            Part::Logs => Some("Earlier log files are gone; a bug report has less to show."),
            Part::Inbox => Some("Shared files not imported yet are lost."),
            _ => None,
        }
    }
}

/// The folders of each part, and the private folders whose total the report gives.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub library: Option<PathBuf>,
    pub smart_previews: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub models: Vec<PathBuf>,
    pub logs: Option<PathBuf>,
    pub inbox: Option<PathBuf>,
    pub temp: Option<PathBuf>,
    /// The host's private folders to count in the total.
    pub roots: Vec<PathBuf>,
}

impl Layout {
    /// The session's folders plus what the host registered (`Session::storage_host`).
    pub fn of(s: &Session) -> Layout {
        let lib = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone());
        let mut models: Vec<PathBuf> =
            [s.denoise.models_dir.clone(), s.face_models_dir.clone(), s.segmenter.dir.clone()].into_iter().flatten().collect();
        models.sort();
        models.dedup();
        let nested: Vec<PathBuf> = models.iter().filter(|m| models.iter().any(|o| o != *m && m.starts_with(o))).cloned().collect();
        models.retain(|m| !nested.contains(m));
        Layout {
            smart_previews: lib.as_ref().map(|d| s.media.smart_dir.clone().unwrap_or_else(|| crate::smart::dir(d))),
            library: lib,
            config: crate::config::config_dir(),
            models,
            logs: s.storage_host.logs.clone(),
            inbox: s.storage_host.inbox.clone(),
            // LightCraft's own scratch folder only: the bare system temp folder is other programs' too
            temp: crate::config::injected_scratch_dir(),
            roots: s.storage_host.roots.clone(),
        }
    }

    fn lib_sub(&self, name: &str) -> Option<PathBuf> {
        self.library.as_ref().map(|l| l.join(name))
    }

    /// The folders a part is made of.
    pub fn dirs(&self, part: Part) -> Vec<PathBuf> {
        let one = |d: Option<PathBuf>| d.into_iter().collect::<Vec<_>>();
        match part {
            Part::Library => one(self.library.clone()),
            Part::Originals => one(self.lib_sub("Originals")),
            Part::Thumbnails => one(self.lib_sub("thumbs")),
            Part::SmartPreviews => one(self.smart_previews.clone()),
            Part::DenoiseCache => one(self.lib_sub(crate::denoise::DIR)),
            Part::Models => self.models.clone(),
            Part::Places => one(self.config.as_ref().map(|c| c.join("geonames"))),
            Part::Logs => one(self.logs.clone()),
            Part::Inbox => one(self.inbox.clone()),
            Part::Temp => one(self.temp.clone()),
            Part::Other => vec![],
        }
    }

    /// Folders inside the library folder that belong to other parts, so [`Part::Library`] leaves them out.
    fn skipped(&self, part: Part) -> Vec<PathBuf> {
        if part != Part::Library {
            return vec![];
        }
        [Part::Originals, Part::Thumbnails, Part::SmartPreviews, Part::DenoiseCache].into_iter().flat_map(|p| self.dirs(p)).collect()
    }

    fn inside_roots(&self, dir: &Path) -> bool {
        self.roots.iter().any(|r| dir.starts_with(r))
    }
}

/// The measured size of one part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartSize {
    pub part: Part,
    pub bytes: u64,
    pub files: u64,
    /// The walk hit a limit or could not read a folder: the figure is a lower bound.
    pub truncated: bool,
}

/// Every part, and the total.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub parts: Vec<PartSize>,
    pub total: u64,
    /// Whether `total` is the walk of the host's private folders (else the sum of the parts).
    pub counts_roots: bool,
    /// Measuring was cancelled: the figures are partial.
    pub cancelled: bool,
}

impl Report {
    pub fn get(&self, part: Part) -> Option<&PartSize> {
        self.parts.iter().find(|p| p.part == part)
    }
}

struct Walk {
    bytes: u64,
    files: u64,
    entries: usize,
    truncated: bool,
}

/// Bytes and files under `root` (a file counts as itself), skipping `skip`. Never follows a symlink; a
/// folder it cannot read marks the result truncated.
fn walk(root: &Path, skip: &[PathBuf], limits: Limits, cancel: &AtomicBool) -> Walk {
    let mut out = Walk { bytes: 0, files: 0, entries: 0, truncated: false };
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((path, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            out.truncated = true;
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_file() {
            out.bytes = out.bytes.saturating_add(meta.len());
            out.files = out.files.saturating_add(1);
            continue;
        }
        if !meta.is_dir() || skip.contains(&path) {
            continue;
        }
        if depth >= limits.max_depth {
            out.truncated = true;
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&path) else {
            out.truncated = true;
            continue;
        };
        for e in rd {
            if out.entries >= limits.max_entries {
                out.truncated = true;
                break;
            }
            out.entries = out.entries.saturating_add(1);
            let Ok(e) = e else {
                out.truncated = true;
                continue;
            };
            stack.push((e.path(), depth.saturating_add(1)));
        }
    }
    out
}

fn measure_part(layout: &Layout, part: Part, limits: Limits, cancel: &AtomicBool) -> PartSize {
    let skip = layout.skipped(part);
    let mut size = PartSize { part, bytes: 0, files: 0, truncated: false };
    for dir in layout.dirs(part) {
        let w = walk(&dir, &skip, limits, cancel);
        size.bytes = size.bytes.saturating_add(w.bytes);
        size.files = size.files.saturating_add(w.files);
        size.truncated |= w.truncated;
    }
    size
}

/// Measure every part (stops early when `cancel` is set). Meant for a worker thread.
pub fn measure(layout: &Layout, limits: Limits, cancel: &AtomicBool) -> Report {
    let mut parts: Vec<PartSize> = Part::ALL.into_iter().filter(|p| *p != Part::Other).map(|p| measure_part(layout, p, limits, cancel)).collect();
    // roots inside another root are already counted by it
    let roots: Vec<&PathBuf> = layout.roots.iter().filter(|r| !layout.roots.iter().any(|o| o != *r && r.starts_with(o))).collect();
    let counts_roots = !roots.is_empty();
    let mut other = PartSize { part: Part::Other, bytes: 0, files: 0, truncated: false };
    let total = if counts_roots {
        let (mut bytes, mut files, mut truncated) = (0u64, 0u64, false);
        for r in roots {
            let w = walk(r, &[], limits, cancel);
            bytes = bytes.saturating_add(w.bytes);
            files = files.saturating_add(w.files);
            truncated |= w.truncated;
        }
        // parts whose folders are all inside a root are part of `bytes`
        let claimed = parts
            .iter()
            .filter(|p| layout.dirs(p.part).iter().all(|d| layout.inside_roots(d)))
            .fold((0u64, 0u64), |a, p| (a.0.saturating_add(p.bytes), a.1.saturating_add(p.files)));
        other.bytes = bytes.saturating_sub(claimed.0);
        other.files = files.saturating_sub(claimed.1);
        other.truncated = truncated;
        bytes
    } else {
        parts.iter().fold(0u64, |a, p| a.saturating_add(p.bytes))
    };
    parts.push(other);
    // (a part with no folder here, such as a library outside the roots, still lists, at zero)
    Report { parts, total, counts_roots, cancelled: cancel.load(Ordering::Relaxed) }
}

/// What a clear removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleared {
    pub files: u64,
    pub bytes: u64,
}

/// Remove what is inside `dir` (the folder stays). Returns what went; an error names the first
/// thing that could not be removed.
fn empty_dir(dir: &Path, limits: Limits) -> Result<Cleared, String> {
    let never = AtomicBool::new(false);
    let mut cleared = Cleared::default();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(cleared),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut first_error = None;
    for e in rd.flatten().take(limits.max_entries) {
        let path = e.path();
        let w = walk(&path, &[], limits, &never);
        let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
        let res = if is_dir { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match res {
            Ok(()) => {
                cleared.files = cleared.files.saturating_add(w.files);
                cleared.bytes = cleared.bytes.saturating_add(w.bytes);
            }
            Err(err) => {
                first_error.get_or_insert_with(|| format!("{}: {err}", path.display()));
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(cleared),
    }
}

/// Empty the log folder but keep the file being written: the logger holds it open, so deleting it would send
/// the next lines to a file nobody can see. That one is truncated; the rotated files go.
fn empty_logs(dir: &Path, limits: Limits) -> Result<Cleared, String> {
    let current = dir.join("lightcraft.log");
    let kept = std::fs::symlink_metadata(&current).ok().filter(|m| m.is_file()).map(|m| m.len());
    let mut cleared = Cleared::default();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(cleared),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut first_error = None;
    for e in rd.flatten().take(limits.max_entries) {
        let path = e.path();
        if path == current {
            continue;
        }
        let w = walk(&path, &[], limits, &AtomicBool::new(false));
        let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
        let res = if is_dir { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match res {
            Ok(()) => {
                cleared.files = cleared.files.saturating_add(w.files);
                cleared.bytes = cleared.bytes.saturating_add(w.bytes);
            }
            Err(err) => {
                first_error.get_or_insert_with(|| format!("{}: {err}", path.display()));
            }
        }
    }
    if let Some(len) = kept.filter(|l| *l > 0) {
        match std::fs::OpenOptions::new().write(true).truncate(true).open(&current) {
            Ok(_) => cleared.bytes = cleared.bytes.saturating_add(len),
            Err(err) => {
                first_error.get_or_insert_with(|| format!("{}: {err}", current.display()));
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(cleared),
    }
}

/// A clear prepared on the session's thread, to run anywhere ([`ClearJob::run`]).
pub struct ClearJob(Box<dyn FnOnce() -> Result<Cleared, String> + Send>);

impl ClearJob {
    pub fn run(self) -> Result<Cleared, String> {
        (self.0)()
    }
}

/// Remove the model folders (one per model) inside `dir`, keeping loose files such as `sam3-mirrors.txt` and
/// the settings. Symlinks are left alone.
fn remove_model_folders(dir: &Path, limits: Limits) -> Result<Cleared, String> {
    let never = AtomicBool::new(false);
    let mut cleared = Cleared::default();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(cleared),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut first_error = None;
    for e in rd.flatten().take(limits.max_entries) {
        let path = e.path();
        // `.downloads` holds a download that may be running
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let w = walk(&path, &[], limits, &never);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                cleared.files = cleared.files.saturating_add(w.files);
                cleared.bytes = cleared.bytes.saturating_add(w.bytes);
            }
            Err(err) => {
                first_error.get_or_insert_with(|| format!("{}: {err}", path.display()));
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(cleared),
    }
}

/// Prepare to empty a clearable part: the session's own caches are told now (memory, the denoise index) and
/// the file work is returned to run on a worker. The library, originals, place names and temporary files are
/// refused. AI models: only the model folders inside LightCraft's settings folder go; a folder named by an
/// override (such as `LIGHTCRAFT_SAM3_DIR`) and loose files next to the models stay.
pub fn clear_job(s: &mut Session, layout: &Layout, part: Part, limits: Limits) -> Result<ClearJob, String> {
    if !part.clearable() {
        return Err(format!("{} can't be cleared here", part.label()));
    }
    let dirs = layout.dirs(part);
    Ok(ClearJob(match part {
        Part::Thumbnails => {
            s.media.clear_sources();
            let rendered = s.media.rendered.clone();
            Box::new(move || {
                let mut before = Cleared::default();
                for d in &dirs {
                    let w = walk(d, &[], limits, &AtomicBool::new(false));
                    before.files = before.files.saturating_add(w.files);
                    before.bytes = before.bytes.saturating_add(w.bytes);
                }
                rendered.clear();
                Ok(before)
            })
        }
        Part::DenoiseCache => {
            let dir = s.denoise_reset();
            Box::new(move || {
                let (files, bytes) = dir.map_or((0, 0), |d| crate::denoise::remove_products(&d));
                Ok(Cleared { files: files as u64, bytes })
            })
        }
        Part::Models => {
            // work in flight must not meet a missing model
            s.denoise_reset();
            let config = layout.config.clone();
            Box::new(move || {
                let mut total = Cleared::default();
                for d in dirs.iter().filter(|d| config.as_ref().is_some_and(|c| d.starts_with(c))) {
                    let c = remove_model_folders(d, limits)?;
                    total.files = total.files.saturating_add(c.files);
                    total.bytes = total.bytes.saturating_add(c.bytes);
                }
                Ok(total)
            })
        }
        _ => Box::new(move || {
            let mut total = Cleared::default();
            for d in &dirs {
                let c = if part == Part::Logs { empty_logs(d, limits)? } else { empty_dir(d, limits)? };
                total.files = total.files.saturating_add(c.files);
                total.bytes = total.bytes.saturating_add(c.bytes);
            }
            Ok(total)
        }),
    }))
}

/// After a clear finished: let the session look at the folder again.
pub fn after_clear(s: &mut Session, part: Part) {
    match part {
        Part::DenoiseCache => s.denoise.touch(),
        Part::Models => {
            crate::cmd::denoise::models_cleared(s);
            crate::cmd::face_models::models_cleared(s);
        }
        _ => {}
    }
}

/// Empty a clearable part now, on the calling thread (the commands; the app's Storage page runs
/// [`clear_job`] on a worker).
pub fn clear(s: &mut Session, layout: &Layout, part: Part, limits: Limits) -> Result<Cleared, String> {
    let cleared = clear_job(s, layout, part, limits)?.run();
    after_clear(s, part);
    cleared
}

/// Free bytes (available to this app) on the volume holding `path`; `None` when it can't be told (no such
/// folder, or a platform without the call).
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    let st = rustix::fs::statvfs(path).ok()?;
    Some(st.f_bavail.saturating_mul(st.f_frsize))
}

/// See the unix version.
#[cfg(not(unix))]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// What the low-space rule decided ([`trim_caches_if_low`]): the figures for a notice, and the file work to run
/// on a worker ([`Trim::take_job`]).
pub struct Trim {
    /// Free space on the library's volume, in MB.
    pub free_mb: u64,
    /// The thumbnail cache's new limit in MB, when it was lowered.
    pub thumbs_mb: Option<u64>,
    /// The denoise cache's half limit in MB, when it had to shrink to it.
    pub denoise_mb: Option<u64>,
    job: Option<Box<dyn FnOnce() + Send>>,
}

impl Trim {
    /// The thumbnail pruning (scans the cache and deletes down to the new limit), for any thread.
    pub fn take_job(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        self.job.take()
    }

    /// Run the file work on the calling thread (tests, the commands).
    pub fn run_now(&mut self) {
        if let Some(job) = self.take_job() {
            job();
        }
    }
}

/// The low-space rule: when the volume of `probe_path` has under [`LOW_SPACE_BYTES`] free, halve the thumbnail
/// and denoise cache limits. Cheap on the calling thread: the limits change and the few denoise pictures go; the
/// thumbnail pruning (thousands of files) is returned as a job for a worker. `free` answers the free bytes of a
/// path (`None` = unknown: nothing happens). `None` when there is nothing to trim.
pub fn trim_caches_if_low(s: &mut Session, probe_path: &Path, free: &dyn Fn(&Path) -> Option<u64>) -> Option<Trim> {
    let free_bytes = free(probe_path).filter(|f| *f < LOW_SPACE_BYTES)?;
    let half = s.cache_bytes() / 2;
    let mut trim = Trim { free_mb: free_bytes >> 20, thumbs_mb: None, denoise_mb: None, job: None };
    if let Some(disk) = s.media.rendered.disk().filter(|d| d.budget() > half || d.size() > half) {
        disk.set_budget(half);
        let rendered = s.media.rendered.clone();
        trim.job = Some(Box::new(move || {
            if let Some(d) = rendered.disk() {
                d.prune();
            }
        }));
        trim.thumbs_mb = Some(half >> 20);
    }
    trim.denoise_mb = s.denoise_trim_half().map(|limit| limit >> 20);
    (trim.thumbs_mb.is_some() || trim.denoise_mb.is_some()).then_some(trim)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-storage-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; len]).unwrap();
    }

    /// A private folder with a file in every part.
    fn fixture(tag: &str) -> (PathBuf, Layout) {
        let files = temp(tag);
        let lib = files.join("library");
        put(&lib.join("catalog.snap"), 100);
        put(&lib.join("Originals/a.jpg"), 200);
        put(&lib.join("thumbs/ab/x.jpg"), 300);
        put(&lib.join("Smart Previews/a.lcsp"), 400);
        put(&lib.join("denoise/a.png"), 500);
        put(&files.join("config/models/face/model.onnx"), 600);
        put(&files.join("config/models/sam3-mirrors.txt"), 10);
        put(&files.join("config/geonames/cities.txt"), 700);
        put(&files.join("logs/lightcraft.log"), 800);
        put(&files.join("inbox/shared.jpg"), 900);
        put(&files.join("config/cache/tmp.bin"), 1000);
        put(&files.join("config/ui.json"), 50);
        let layout = Layout {
            library: Some(lib.clone()),
            smart_previews: Some(lib.join("Smart Previews")),
            config: Some(files.join("config")),
            models: vec![files.join("config/models")],
            logs: Some(files.join("logs")),
            inbox: Some(files.join("inbox")),
            temp: Some(files.join("config/cache")),
            roots: vec![files.clone(), files.join("cache")],
        };
        (files, layout)
    }

    fn bytes(r: &Report, p: Part) -> u64 {
        r.get(p).unwrap().bytes
    }

    #[test]
    fn storage_report_counts_every_part() {
        let (files, layout) = fixture("parts");
        let r = measure(&layout, Limits::default(), &AtomicBool::new(false));
        assert_eq!(bytes(&r, Part::Library), 100);
        assert_eq!(bytes(&r, Part::Originals), 200);
        assert_eq!(bytes(&r, Part::Thumbnails), 300);
        assert_eq!(bytes(&r, Part::SmartPreviews), 400);
        assert_eq!(bytes(&r, Part::DenoiseCache), 500);
        assert_eq!(bytes(&r, Part::Models), 610);
        assert_eq!(bytes(&r, Part::Places), 700);
        assert_eq!(bytes(&r, Part::Logs), 800);
        assert_eq!(bytes(&r, Part::Inbox), 900);
        assert_eq!(bytes(&r, Part::Temp), 1000);
        // the settings file sits in a private folder but in no part
        assert_eq!(bytes(&r, Part::Other), 50);
        // the total is the walk of the roots, so it is what the system adds up
        assert_eq!(r.total, 100 + 200 + 300 + 400 + 500 + 610 + 700 + 800 + 900 + 1000 + 50);
        assert!(r.counts_roots && !r.cancelled);
        assert_eq!(r.get(Part::Thumbnails).unwrap().files, 1);
        // without roots the total is the sum of the parts
        let none = Layout { roots: vec![], ..layout };
        assert_eq!(measure(&none, Limits::default(), &AtomicBool::new(false)).total, 5510);
        let _ = std::fs::remove_dir_all(files);
    }

    #[test]
    fn clear_cache_part_keeps_library_and_originals() {
        let (files, layout) = fixture("clear");
        let mut s = Session::new();
        let limits = Limits::default();
        let c = clear(&mut s, &layout, Part::SmartPreviews, limits).unwrap();
        assert_eq!(c, Cleared { files: 1, bytes: 400 });
        let c = clear(&mut s, &layout, Part::Models, limits).unwrap();
        assert_eq!(c.bytes, 600);
        // the model folder goes; the mirrors list next to it stays
        assert!(!files.join("config/models/face").exists());
        assert!(files.join("config/models/sam3-mirrors.txt").exists());
        clear(&mut s, &layout, Part::Inbox, limits).unwrap();
        put(&files.join("logs/lightcraft.1.log"), 40);
        let c = clear(&mut s, &layout, Part::Logs, limits).unwrap();
        assert_eq!(c.bytes, 840);
        // the log being written stays (emptied); the rotated one goes
        assert_eq!(std::fs::metadata(files.join("logs/lightcraft.log")).unwrap().len(), 0);
        assert!(!files.join("logs/lightcraft.1.log").exists());
        assert!(!files.join("inbox/shared.jpg").exists());
        assert!(files.join("inbox").is_dir(), "the folder stays");
        // what must never go
        for part in [Part::Library, Part::Originals, Part::Places, Part::Temp, Part::Other] {
            assert!(clear(&mut s, &layout, part, limits).is_err(), "{part:?}");
        }
        assert!(files.join("library/catalog.snap").exists());
        assert!(files.join("library/Originals/a.jpg").exists());
        assert!(files.join("config/geonames/cities.txt").exists());
        assert!(Part::Models.consequence().is_some_and(|c| c.contains("stop working")));
        let _ = std::fs::remove_dir_all(files);
    }

    /// A models folder named by an override (outside the settings folder) is never emptied.
    #[test]
    fn clear_models_leaves_override_folders() {
        let (files, mut layout) = fixture("override");
        let shared = temp("shared-models");
        put(&shared.join("other/model.onnx"), 5);
        layout.models.push(shared.clone());
        let mut s = Session::new();
        clear(&mut s, &layout, Part::Models, Limits::default()).unwrap();
        assert!(shared.join("other/model.onnx").exists());
        assert!(!files.join("config/models/face").exists());
        let _ = std::fs::remove_dir_all(files);
        let _ = std::fs::remove_dir_all(shared);
    }

    /// The folders come from the session (what the host registered), never from a command's parameters: a
    /// foreign `inbox` or `logs` is ignored and its files survive.
    #[test]
    fn commands_never_empty_a_folder_the_caller_names() {
        let foreign = temp("foreign");
        put(&foreign.join("precious.jpg"), 5);
        let mut s = Session::new();
        for part in ["inbox", "logs"] {
            let p = serde_json::json!({"part": part, "inbox": foreign.display().to_string(), "logs": foreign.display().to_string(), "roots": [foreign.display().to_string()]});
            // (nothing is registered, so there is nothing to empty)
            let _ = s.execute("library.clearStorage", &p);
        }
        assert!(foreign.join("precious.jpg").exists());
        // the report does not take them either
        let p = serde_json::json!({"inbox": foreign.display().to_string(), "roots": [foreign.display().to_string()]});
        let r = s.execute("library.storage", &p).unwrap();
        assert_eq!(r["total"], 0);
        // a registered folder is the one that is emptied
        s.storage_host.inbox = Some(foreign.clone());
        s.execute("library.clearStorage", &serde_json::json!({"part": "inbox"})).unwrap();
        assert!(!foreign.join("precious.jpg").exists());
        let _ = std::fs::remove_dir_all(foreign);
    }

    #[test]
    fn temp_part_is_not_the_system_temp_folder() {
        let s = Session::new();
        assert_ne!(Layout::of(&s).temp.as_deref(), Some(std::env::temp_dir().as_path()));
    }

    #[test]
    fn free_bytes_of_an_existing_folder() {
        let dir = temp("free");
        #[cfg(unix)]
        assert!(free_bytes(&dir).is_some());
        assert!(free_bytes(&dir.join("missing/deeper")).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn report_walk_is_bounded() {
        let dir = temp("bounded");
        for i in 0..20 {
            put(&dir.join(format!("f{i}")), 1);
        }
        let w = walk(&dir, &[], Limits { max_depth: 24, max_entries: 5 }, &AtomicBool::new(false));
        assert!(w.truncated && w.files <= 5, "{} files", w.files);
        // depth
        put(&dir.join("a/b/c/d/deep.bin"), 9);
        let shallow = walk(&dir.join("a"), &[], Limits { max_depth: 2, max_entries: 100 }, &AtomicBool::new(false));
        assert!(shallow.truncated && shallow.bytes == 0);
        // a set cancel flag stops at once
        let stopped = walk(&dir, &[], Limits::default(), &AtomicBool::new(true));
        assert!(stopped.truncated && stopped.files == 0);
        // a missing folder is zero, not an error
        assert_eq!(walk(&dir.join("nope"), &[], Limits::default(), &AtomicBool::new(false)).bytes, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn low_space_trims_caches() {
        let dir = temp("low");
        let mut s = Session::new().with_fs();
        s.open_library(dir.join("library"), true).unwrap();
        s.set_cache_mb(1024).unwrap();
        let img = lightcraft_raster::Rgba8::from_bytes(1, 1, &[1, 2, 3, 255]).unwrap();
        s.media.rendered.disk().unwrap().put(lightcraft_preview::Hash128(1), &img);
        let probe = dir.clone();
        // plenty of room, or an unknown answer: nothing happens
        assert!(trim_caches_if_low(&mut s, &probe, &|_| Some(50 << 30)).is_none());
        assert!(trim_caches_if_low(&mut s, &probe, &|_| None).is_none());
        assert_eq!(s.media.rendered.disk().unwrap().budget(), 1024 << 20);
        // under 1 GB free: half the limit now, the pruning left to a worker
        let mut trim = trim_caches_if_low(&mut s, &probe, &|_| Some(300 << 20)).unwrap();
        assert_eq!((trim.free_mb, trim.thumbs_mb), (300, Some(512)));
        assert!(trim.take_job().is_some(), "the file work is handed out, not done here");
        assert_eq!(s.media.rendered.disk().unwrap().budget(), 512 << 20);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The denoise cache goes down to half its limit too (sparse files: only their length counts).
    #[test]
    fn low_space_trims_denoise_cache() {
        let dir = temp("low-denoise");
        let mut s = Session::new().with_fs();
        s.open_library(dir.join("library"), true).unwrap();
        let models = dir.join("models");
        crate::denoise::write_settings(&models, &crate::denoise::Settings { cache_gb: Some(1), ..Default::default() }).unwrap();
        s.denoise.models_dir = Some(models);
        let products = dir.join("library").join(crate::denoise::DIR);
        std::fs::create_dir_all(&products).unwrap();
        for n in 0..3 {
            let f = std::fs::File::create(products.join(format!("p{n}.{}", crate::denoise::EXT))).unwrap();
            f.set_len(300 << 20).unwrap();
        }
        let left = || std::fs::read_dir(&products).unwrap().flatten().map(|e| e.metadata().unwrap().len()).sum::<u64>();
        assert_eq!(left(), 900 << 20);
        let trim = trim_caches_if_low(&mut s, &dir, &|_| Some(300 << 20)).unwrap();
        assert_eq!(trim.denoise_mb, Some(512));
        assert!(left() <= 512 << 20 && left() > 0, "{}", left() >> 20);
        // already within half: nothing more to say about it
        let again = trim_caches_if_low(&mut s, &dir, &|_| Some(300 << 20));
        assert!(again.is_none_or(|n| n.denoise_mb.is_none()));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Clearing the models forgets the choices that named them, leaves a running download's staging folder,
    /// and cancels denoise work.
    #[test]
    fn clear_models_forgets_choices_and_keeps_staging() {
        let (files, layout) = fixture("models-state");
        let models = files.join("config/models");
        put(&models.join(".downloads/part.bin"), 5);
        crate::denoise::write_settings(&models, &crate::denoise::Settings { model: Some("dn".into()), ..Default::default() }).unwrap();
        let faces = files.join("config/face-models");
        put(&faces.join("settings.json"), 0);
        std::fs::write(faces.join("settings.json"), br#"{"enabled":true,"embedder":"fm"}"#).unwrap();
        let mut s = Session::new();
        s.denoise.models_dir = Some(models.clone());
        s.face_models_dir = Some(faces.clone());
        s.denoise_refresh_active(true);
        assert_eq!(s.denoise.settings.model.as_deref(), Some("dn"));
        clear(&mut s, &layout, Part::Models, Limits::default()).unwrap();
        assert!(models.join(".downloads/part.bin").exists(), "a download in flight keeps its staging folder");
        assert!(crate::denoise::read_settings(&models).model.is_none());
        assert!(s.denoise.settings.model.is_none() && s.denoise.active.is_none(), "the session no longer reports it");
        let face: serde_json::Value = serde_json::from_slice(&std::fs::read(faces.join("settings.json")).unwrap()).unwrap();
        assert_eq!(face["enabled"], false);
        assert!(face["embedder"].is_null(), "{face}");
        let _ = std::fs::remove_dir_all(files);
    }
}
