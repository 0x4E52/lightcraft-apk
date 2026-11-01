//! Backing up and restoring a library folder (the Android host's, which lives in the app's private
//! storage and goes with the app) in the [`crate::backup`] format.
//!
//! A backup is made in two steps: [`Plan::of`] on the thread that owns the session (it saves what
//! is queued and reads the library's own files, so the catalog in the backup is the one in
//! memory), then [`write`] on a worker, which streams the Copy-imported originals (the photos
//! below the library's `Originals/` folder) into the zip through `safe_file` (a temp file renamed
//! when complete, so a cancelled or failed backup leaves no half file). Photos added in place, such
//! as the camera's DCIM folder, and any other photo outside `Originals/`, are listed by path in the
//! manifest, not copied (a restore writes originals only below `Originals/`). A library over the
//! format's limits (4 GB, 65 535 entries: no zip64) is refused by [`Plan::check_limits`] before
//! anything is read or written.
//!
//! A restore never touches the library until the whole backup checked out: [`verify_file`] reads
//! every entry and checks its CRC-32 and, for originals, its content hash; [`extract`] then writes
//! the files into a new folder beside the library (removed again on any failure), which the host
//! swaps in and opens. Once it is open, [`rebase`] points the originals at their new place when
//! the library folder's path differs from the backed-up one (another application id or device).
//!
//! Entries are read and written in 1 MiB pieces: nothing sized by the zip is allocated whole.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use lightcraft_catalog::{Catalog, Op, Source};

use crate::Session;
use crate::backup::{
    LIBRARY_FILES, MANIFEST, MANIFEST_FORMAT, MAX_BYTES, MAX_ENTRIES, Manifest, ManifestOriginal, ZipEntry, ZipWriter, data_offset, entry_hash,
    find_central, library_file, original_entry, parse_central, tail_len, verify_original_hash, verify_sums,
};

/// The readme inside a library folder's backup.
pub const README: &str = "LightCraft library backup\n\n\
library/      the catalog (catalog.snap + catalog.log), presets, view state and preferences\n\
originals/    every photo copied into the library, as originals/<content hash>/<file name>\n\
backup.json   where each photo was, and the photos added in place (not copied, listed by path)\n\n\
Restore it in LightCraft with File > Restore Library from Backup...\n\
The photos in originals/ are the untouched files you imported; edits live in the catalog.\n";

/// The activity row of a backup.
pub const BACKUP_LABEL: &str = "Backing up the library";
/// The activity row of a restore.
pub const RESTORE_LABEL: &str = "Restoring the library";

/// Largest `backup.json` a restore reads.
const MAX_MANIFEST: u64 = 64 << 20;
/// Pieces entries are streamed in.
const CHUNK: usize = 1 << 20;

/// A backup to write: the library files as they are now and the originals to copy.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The library folder.
    pub library_dir: PathBuf,
    /// `catalog.snap`, `catalog.log`, … with their bytes, read by [`Plan::of`].
    files: Vec<(&'static str, Vec<u8>)>,
    /// Originals below the library's `Originals/` folder: (catalog path, the photo's file name).
    originals: Vec<(PathBuf, String)>,
    /// Photos outside `Originals/` (added in place), by path.
    in_place: Vec<String>,
    /// The originals' sizes when the plan was made (for the space check).
    originals_bytes: u64,
}

impl Plan {
    /// Save what is queued and read the library's files, on the session's thread. `Err` without a
    /// library folder, or while changes can't be written (the backup would miss them).
    pub fn of(s: &mut Session) -> Result<Plan, String> {
        let dir = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone()).ok_or("No library folder is open to back up")?;
        let _ = s.end_interaction();
        if let Err(e) = s.persist() {
            return Err(format!("The library can't be backed up while changes aren't saved: {e}"));
        }
        s.save_view();
        // (snapshot and log read together on this thread: a compaction resets the log only here)
        let mut files = Vec::new();
        for name in LIBRARY_FILES {
            match std::fs::read(dir.join(name)) {
                Ok(bytes) => files.push((name, bytes)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("Can't read the library's {name}: {e}")),
            }
        }
        if !files.iter().any(|(n, _)| *n == lightcraft_catalog::journal::SNAPSHOT || *n == lightcraft_catalog::journal::LOG) {
            return Err("The library has no catalog file to back up yet".into());
        }
        let mut seen = BTreeSet::new();
        let (mut originals, mut in_place) = (Vec::new(), Vec::new());
        let mut originals_bytes = 0u64;
        for p in s.catalog.photos() {
            let Source::File { path } = &p.source else { continue };
            if !seen.insert(path.clone()) {
                continue; // a virtual copy shares its master's file
            }
            let file = Path::new(path);
            // (a restore writes originals only below Originals/: anything else stays listed by path)
            if file.strip_prefix(&dir).is_ok_and(|rest| rest.components().count() >= 2 && rest.starts_with(ORIGINALS_DIR)) {
                originals_bytes = originals_bytes.saturating_add(std::fs::metadata(file).map(|m| m.len()).unwrap_or(0));
                originals.push((file.to_path_buf(), p.file_name.clone()));
            } else {
                in_place.push(path.clone());
            }
        }
        Ok(Plan { library_dir: dir, files, originals, in_place, originals_bytes })
    }

    /// The space the backup file needs (the files, their headers and directory entries, with
    /// 1 MiB to spare).
    pub fn needed_bytes(&self) -> u64 {
        let files: u64 = self.files.iter().map(|(_, b)| b.len() as u64).sum();
        let entries = (self.files.len() + self.originals.len() + 2) as u64;
        files.saturating_add(self.originals_bytes).saturating_add(entries.saturating_mul(400)).saturating_add(1 << 20)
    }

    /// `Err` when the backup can't be one file without zip64: over [`MAX_BYTES`] or
    /// [`MAX_ENTRIES`] entries. Checked before the space check and before the worker starts, so a
    /// library that can't fit is refused without reading its originals.
    pub fn check_limits(&self) -> Result<(), String> {
        let entries = self.files.len().saturating_add(self.originals.len()).saturating_add(2);
        let needed = self.needed_bytes();
        if needed > MAX_BYTES || entries > MAX_ENTRIES {
            return Err(format!(
                "The library is too large for one backup file: it needs {} for {} originals, and a backup file holds at most {} and 65,535 files (no zip64 yet)",
                size_text(needed),
                self.originals.len(),
                size_text(MAX_BYTES)
            ));
        }
        Ok(())
    }

    /// How many originals the backup copies.
    pub fn originals(&self) -> usize {
        self.originals.len()
    }

    /// How many photos are listed by path only.
    pub fn in_place(&self) -> usize {
        self.in_place.len()
    }
}

/// `bytes` for a message: "3.2 GB", "120 MB", "12 KB".
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

/// `Err` ("needs 3.2 GB, 1.1 GB free") when `free` is known and less than `needed`.
pub fn check_space(what: &str, needed: u64, free: Option<u64>) -> Result<(), String> {
    match free {
        Some(free) if free < needed => {
            Err(format!("Not enough space for {what}: it needs {}, {} free. Free some space and try again.", size_text(needed), size_text(free)))
        }
        _ => Ok(()),
    }
}

/// The backup file's name for a time (Unix seconds): `lightcraft-library-backup-2026-11-01-153000.zip`.
pub fn backup_name(secs: i64) -> String {
    let civil = lightcraft_catalog::dates::civil(secs);
    let stamp: String = civil.replace('T', "-").chars().filter(|c| *c != ':').collect();
    format!("lightcraft-library-backup-{stamp}.zip")
}

/// What [`write`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub path: PathBuf,
    /// The zip's length.
    pub bytes: u64,
    /// Originals copied into it.
    pub originals: usize,
    /// Originals that were gone when the backup reached them (removed meanwhile).
    pub missing: Vec<String>,
    /// Photos listed by path only.
    pub in_place: usize,
}

fn cancelled() -> io::Error {
    io::Error::other("The backup was cancelled")
}

/// Length, CRC-32 and content hash of a file, streamed.
fn measure(file: &Path, cancel: &AtomicBool) -> io::Result<(u64, u32, String)> {
    let mut f = File::open(file)?;
    let (mut crc, mut hash, mut len) = (crc32fast::Hasher::new(), lightcraft_preview::Hasher128::new(), 0u64);
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let n = f.read(&mut buf)?;
        let Some(piece) = buf.get(..n).filter(|p| !p.is_empty()) else { break };
        crc.update(piece);
        hash.update(piece);
        len = len.saturating_add(n as u64);
    }
    Ok((len, crc.finalize(), hash.finish().to_string()))
}

/// Copy `len` bytes of `file` to `out`, checking that they still have CRC-32 `want`.
fn copy_checked(file: &Path, len: u64, want: u32, out: &mut dyn Write, cancel: &AtomicBool) -> io::Result<()> {
    let mut f = File::open(file)?.take(len);
    let mut crc = crc32fast::Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let n = f.read(&mut buf)?;
        let Some(piece) = buf.get(..n).filter(|p| !p.is_empty()) else { break };
        crc.update(piece);
        out.write_all(piece)?;
        copied = copied.saturating_add(n as u64);
    }
    if copied != len || crc.finalize() != want {
        return Err(io::Error::other(format!("{} changed while it was backed up; try again", file.display())));
    }
    Ok(())
}

/// Write the backup of `plan` to `out` (replaced when it exists; a temp file until complete),
/// checking `cancel` between pieces. An original removed since the plan is skipped and named in
/// [`Written::missing`].
pub fn write(plan: &Plan, out: &Path, cancel: &AtomicBool) -> Result<Written, String> {
    let mut originals = 0usize;
    let mut missing = Vec::new();
    let mut fill = |w: &mut dyn Write| -> io::Result<()> {
        let mut zip = ZipWriter::default();
        let put = |zip: &mut ZipWriter, w: &mut dyn Write, name: &str, data: &[u8]| -> io::Result<()> {
            w.write_all(&zip.entry(name, data).map_err(io::Error::other)?)?;
            w.write_all(data)
        };
        put(&mut zip, w, "README.txt", README.as_bytes())?;
        for (name, bytes) in &plan.files {
            put(&mut zip, w, &format!("library/{name}"), bytes)?;
        }
        let mut manifest = Manifest {
            format: MANIFEST_FORMAT,
            library_dir: plan.library_dir.to_string_lossy().into_owned(),
            originals: Vec::new(),
            in_place: plan.in_place.clone(),
        };
        for (path, name) in &plan.originals {
            let (len, crc, hash) = match measure(path, cancel) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    missing.push(path.to_string_lossy().into_owned());
                    continue;
                }
                Err(e) if cancel.load(Ordering::Relaxed) => return Err(e),
                Err(e) => return Err(io::Error::other(format!("Can't read {}: {e}", path.display()))),
            };
            let entry = original_entry(&hash, name);
            w.write_all(&zip.entry_for(&entry, len, crc).map_err(io::Error::other)?)?;
            copy_checked(path, len, crc, w, cancel)?;
            manifest.originals.push(ManifestOriginal { entry, path: path.to_string_lossy().into_owned() });
            originals += 1;
        }
        let json = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
        put(&mut zip, w, MANIFEST, &json)?;
        w.write_all(&zip.finish().map_err(io::Error::other)?)
    };
    let bytes = lightcraft_catalog::safe_file::write_atomic_with(out, &mut fill).map_err(|e| {
        if cancel.load(Ordering::Relaxed) { "The backup was cancelled".to_string() } else { format!("The backup couldn't be written: {e}") }
    })?;
    Ok(Written { path: out.to_path_buf(), bytes, originals, missing, in_place: plan.in_place.len() })
}

/// A backup that checked out ([`verify_file`]).
#[derive(Debug, Clone)]
pub struct Verified {
    entries: Vec<ZipEntry>,
    pub manifest: Manifest,
    /// The bytes a restore writes.
    pub bytes: u64,
    /// Originals it holds.
    pub originals: usize,
}

fn read_at(f: &mut File, at: u64, len: u64) -> Result<Vec<u8>, String> {
    f.seek(SeekFrom::Start(at)).map_err(|e| format!("Can't read the backup: {e}"))?;
    let mut out = Vec::new();
    f.take(len).read_to_end(&mut out).map_err(|e| format!("Can't read the backup: {e}"))?;
    if out.len() as u64 != len {
        return Err("The backup is truncated".into());
    }
    Ok(out)
}

/// Stream an entry's data to `sink` (pieces), checking its CRC-32; returns its content hash.
fn stream_entry(
    f: &mut File,
    file_len: u64,
    e: &ZipEntry,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<String, String> {
    let local = read_at(f, e.header, 30)?;
    let start = data_offset(e, &local)?;
    if start.checked_add(e.size).is_none_or(|end| end > file_len) {
        return Err(format!("{}: damaged entry (past the end of the file)", e.name));
    }
    f.seek(SeekFrom::Start(start)).map_err(|err| format!("Can't read the backup: {err}"))?;
    let mut r = (&mut *f).take(e.size);
    let (mut crc, mut hash, mut len) = (crc32fast::Hasher::new(), lightcraft_preview::Hasher128::new(), 0u64);
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("The restore was cancelled".into());
        }
        let n = r.read(&mut buf).map_err(|err| format!("Can't read the backup: {err}"))?;
        let Some(piece) = buf.get(..n).filter(|p| !p.is_empty()) else { break };
        crc.update(piece);
        hash.update(piece);
        len = len.saturating_add(n as u64);
        sink(piece)?;
    }
    verify_sums(e, len, crc.finalize())?;
    Ok(hash.finish().to_string())
}

/// The folder of a library that Copy-imported originals live in (`Library::originals_dir`): a
/// restore writes originals only below it.
const ORIGINALS_DIR: &str = "Originals";

/// Where an original of `manifest` goes inside the restored folder: its path below the backed-up
/// library folder, which must be a file below `Originals/` (normal components only), so a crafted
/// `backup.json` can't put an original over the catalog or any other library file.
fn original_target(manifest: &Manifest, entry: &str) -> Option<PathBuf> {
    let original = manifest.originals.iter().find(|o| o.entry == entry)?;
    let rest = Path::new(&original.path).strip_prefix(&manifest.library_dir).ok()?;
    let normal = rest.components().all(|c| matches!(c, Component::Normal(_)));
    let below_originals = rest.components().count() >= 2 && rest.starts_with(ORIGINALS_DIR);
    (normal && below_originals).then(|| rest.to_path_buf())
}

/// Read every entry of the backup at `zip` and check it: the CRC-32 of each, the content hash
/// of each original, a catalog and a manifest. Nothing is written.
pub fn verify_file(zip: &Path, cancel: &AtomicBool) -> Result<Verified, String> {
    let mut f = File::open(zip).map_err(|e| format!("Can't open {}: {e}", zip.display()))?;
    let len = f.metadata().map_err(|e| format!("Can't read the backup: {e}"))?.len();
    let tail = read_at(&mut f, len - tail_len(len), tail_len(len))?;
    let (cd_offset, cd_size) = find_central(&tail).map_err(|e| format!("This isn't a LightCraft library backup: {e}"))?;
    if cd_offset.checked_add(cd_size).is_none_or(|end| end > len) {
        return Err("This isn't a LightCraft library backup: its directory is damaged".into());
    }
    let entries = parse_central(&read_at(&mut f, cd_offset, cd_size)?)?;
    if !entries
        .iter()
        .any(|e| matches!(library_file(&e.name), Some(n) if n == lightcraft_catalog::journal::SNAPSHOT || n == lightcraft_catalog::journal::LOG))
    {
        return Err("This isn't a LightCraft library backup (no library/catalog files)".into());
    }
    let Some(m) = entries.iter().find(|e| e.name == MANIFEST) else {
        return Err("This backup was made by LightCraft for the web; restore it there (it has no backup.json)".into());
    };
    if m.size > MAX_MANIFEST {
        return Err("This backup's backup.json is too large".into());
    }
    let mut json = Vec::new();
    stream_entry(&mut f, len, m, cancel, &mut |piece| {
        json.extend_from_slice(piece);
        Ok(())
    })?;
    let manifest: Manifest = serde_json::from_slice(&json).map_err(|e| format!("This backup's backup.json is damaged: {e}"))?;
    if manifest.format > MANIFEST_FORMAT {
        return Err("This backup was made by a newer LightCraft; update LightCraft to restore it".into());
    }
    let (mut bytes, mut originals) = (0u64, 0usize);
    for e in &entries {
        let hash = entry_hash(&e.name);
        if library_file(&e.name).is_none() && hash.is_none() {
            continue; // README, the manifest (read above), anything unknown
        }
        let actual = stream_entry(&mut f, len, e, cancel, &mut |_| Ok(()))?;
        if let Some(hash) = hash {
            verify_original_hash(e, hash, &actual)?;
            if original_target(&manifest, &e.name).is_none() {
                continue; // not one the manifest places: not restored
            }
            originals += 1;
        }
        bytes = bytes.saturating_add(e.size);
    }
    Ok(Verified { entries, manifest, bytes, originals })
}

/// Write the verified backup at `zip` into `staging` (a new folder; one left from an earlier try
/// is removed first). Each entry's CRC-32 is checked again while it is written; on any error
/// `staging` is removed and nothing else was touched.
pub fn extract(zip: &Path, v: &Verified, staging: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let result = extract_into(zip, v, staging, cancel);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

fn extract_into(zip: &Path, v: &Verified, staging: &Path, cancel: &AtomicBool) -> Result<(), String> {
    if staging.exists() {
        std::fs::remove_dir_all(staging).map_err(|e| format!("Can't clear {}: {e}", staging.display()))?;
    }
    std::fs::create_dir_all(staging).map_err(|e| format!("Can't create {}: {e}", staging.display()))?;
    let mut f = File::open(zip).map_err(|e| format!("Can't open {}: {e}", zip.display()))?;
    let len = f.metadata().map_err(|e| format!("Can't read the backup: {e}"))?.len();
    for e in &v.entries {
        let target = match (library_file(&e.name), entry_hash(&e.name)) {
            (Some(name), _) => staging.join(name),
            (None, Some(_)) => match original_target(&v.manifest, &e.name) {
                Some(rest) => staging.join(rest),
                None => continue,
            },
            _ => continue,
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("Can't create {}: {err}", parent.display()))?;
        }
        let mut failure = None;
        let written = lightcraft_catalog::safe_file::write_atomic_with(&target, &mut |w| {
            let r = stream_entry(&mut f, len, e, cancel, &mut |piece| w.write_all(piece).map_err(|err| err.to_string()));
            r.map(|_| ()).map_err(|err| {
                failure = Some(err.clone());
                io::Error::other(err)
            })
        });
        if let Err(err) = written {
            return Err(failure.unwrap_or_else(|| format!("Can't write {}: {err}", target.display())));
        }
    }
    Ok(())
}

/// The relinks that move the catalog's photos inside `from` (the backed-up library folder) to the
/// same place inside `to`. Empty when the folders are the same.
pub fn rebase_ops(catalog: &Catalog, from: &Path, to: &Path) -> Vec<Op> {
    if from == to {
        return Vec::new();
    }
    catalog
        .photos()
        .filter_map(|p| {
            let Source::File { path } = &p.source else { return None };
            let rest = Path::new(path).strip_prefix(from).ok()?;
            let source = Source::File { path: to.join(rest).to_string_lossy().into_owned() };
            Some(Op::Relink { id: p.id, file_name: p.file_name.clone(), source, format: None })
        })
        .collect()
}

/// Point the open library's originals at their place in it after a restore from a backup of the
/// library folder `from` (not undoable: the old paths are gone). Returns how many moved.
pub fn rebase(s: &mut Session, from: &Path) -> Result<usize, String> {
    let to = s.library.as_ref().map(|l| l.dir.clone()).ok_or("No library is open")?;
    let ops = rebase_ops(&s.catalog, from, &to);
    let n = ops.len();
    if n == 0 {
        return Ok(0);
    }
    s.commit("Restore Library", Op::Batch { ops }).map_err(|e| e.to_string())?;
    s.undo.clear();
    s.persist().map_err(|e| e.to_string())?;
    Ok(n)
}

#[cfg(test)]
#[path = "tests_backup.rs"]
mod tests;
