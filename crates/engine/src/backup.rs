//! The library backup format, shared by the browser build (`apps/lightcraft-web`, whose library
//! lives in browser storage) and the Android host (whose library lives in the app's private
//! storage, deleted with the app): a plain zip, entries stored, not compressed (photos don't
//! shrink), written one entry at a time and read back the same way. Pure and wasm-safe: the hosts
//! do the reading and writing ([`crate::backup_dir`] for a library folder).
//!
//! Layout of a backup:
//! - `library/catalog.snap`, `library/catalog.log`, `library/presets.json`, … (see
//!   [`LIBRARY_FILES`]);
//! - `originals/<content hash>/<file name>`: each stored original's bytes, under its own name so
//!   an unzipped backup is browsable;
//! - `backup.json` (backups of a library folder only, [`Manifest`]): where the library and each
//!   original were, and the photos that stayed where they were (added in place) by path;
//! - `README.txt`.
//!
//! No zip64: a backup holds at most [`MAX_BYTES`] and 65 535 entries.

use serde::{Deserialize, Serialize};

/// Largest backup (offsets are 32-bit without zip64).
pub const MAX_BYTES: u64 = 0xFFFF_FFFF - (64 << 20);

/// Most entries in one backup (the end record's count is 16-bit without zip64).
pub const MAX_ENTRIES: usize = u16::MAX as usize;

/// The library's own files a backup carries (thumbnails and previews are made again).
pub const LIBRARY_FILES: [&str; 6] = ["catalog.snap", "catalog.log", "presets.json", "view.json", "prefs.json", "ui.json"];

/// The entry holding a library folder's [`Manifest`].
pub const MANIFEST: &str = "backup.json";

/// The version [`Manifest::format`] carries.
pub const MANIFEST_FORMAT: u32 = 1;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
/// General-purpose flag bit 11: names are UTF-8.
const UTF8: u16 = 0x0800;
/// 1980-01-01 00:00 in MS-DOS format (date, time).
const DOS_DATE: u16 = (1 << 5) | 1;

/// What a backup of a library folder records beside the files (the `backup.json` entry).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Manifest {
    /// [`MANIFEST_FORMAT`].
    pub format: u32,
    /// The library folder the backup was made of: the catalog's paths of the originals inside it
    /// start with it, and a restore into another folder moves them along.
    pub library_dir: String,
    /// Each original in the backup: its entry and the path the catalog knows it by.
    pub originals: Vec<ManifestOriginal>,
    /// Photos added in place (outside the library folder, such as the camera's DCIM folder): not
    /// copied into the backup, only listed by path.
    pub in_place: Vec<String>,
}

/// One original of a [`Manifest`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ManifestOriginal {
    /// The zip entry (`originals/<hash>/<name>`).
    pub entry: String,
    /// The catalog's path of the file.
    pub path: String,
}

/// Is `h` a plausible content hash (used as a storage key segment)?
pub fn valid_hash(h: &str) -> bool {
    !h.is_empty() && h.len() <= 128 && h.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Zip entry name for an original.
pub fn original_entry(hash: &str, file_name: &str) -> String {
    let name: String = file_name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let name = if name.is_empty() || name == "." || name == ".." { "original".to_string() } else { name };
    format!("originals/{hash}/{name}")
}

/// The content hash an `originals/<hash>/…` entry names (`None` for any other entry, or a hash
/// that isn't one).
pub fn entry_hash(entry: &str) -> Option<&str> {
    let rest = entry.strip_prefix("originals/")?;
    let (hash, _) = rest.split_once('/').unwrap_or((rest, ""));
    valid_hash(hash).then_some(hash)
}

/// The library file a `library/<name>` entry holds, when it is one of [`LIBRARY_FILES`].
pub fn library_file(entry: &str) -> Option<&'static str> {
    let name = entry.strip_prefix("library/")?;
    LIBRARY_FILES.iter().copied().find(|f| *f == name)
}

/// Hash of a file's bytes (the same as the catalog's `content_hash`).
pub fn content_hash(bytes: &[u8]) -> String {
    lightcraft_preview::hash_bytes(bytes).to_string()
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

struct Central {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

/// Writes a stored (uncompressed) zip piece by piece: [`ZipWriter::entry`] returns the local
/// header to emit before the entry's bytes, [`ZipWriter::finish`] the central directory.
#[derive(Default)]
pub struct ZipWriter {
    entries: Vec<Central>,
    offset: u64,
}

impl ZipWriter {
    /// A writer whose next entry starts at `offset` (tests of the size limit).
    pub fn at_offset(offset: u64) -> ZipWriter {
        ZipWriter { entries: Vec::new(), offset }
    }

    /// The local header for `name` holding `data` (emit it, then `data`).
    pub fn entry(&mut self, name: &str, data: &[u8]) -> Result<Vec<u8>, String> {
        self.entry_for(name, data.len() as u64, crc32fast::hash(data))
    }

    /// The local header for `name` holding `size` bytes whose CRC-32 is `crc` (emit it, then the
    /// bytes): for entries streamed from a file, measured first.
    pub fn entry_for(&mut self, name: &str, size: u64, crc: u32) -> Result<Vec<u8>, String> {
        let size = u32::try_from(size).map_err(|_| format!("{name}: larger than 4 GB"))?;
        let name_len = u16::try_from(name.len()).map_err(|_| format!("{name}: name too long"))?;
        if self.entries.len() >= MAX_ENTRIES {
            return Err("too many files for one backup".into());
        }
        let header_len = 30 + u64::from(name_len);
        if self.offset.saturating_add(header_len).saturating_add(u64::from(size)) > MAX_BYTES {
            return Err("the library is too large for one backup file (4 GB)".into());
        }
        let offset = u32::try_from(self.offset).map_err(|_| "backup too large".to_string())?;
        let mut h = Vec::with_capacity(30 + usize::from(name_len));
        u32le(&mut h, LOCAL_SIG);
        u16le(&mut h, 20); // version needed
        u16le(&mut h, UTF8);
        u16le(&mut h, 0); // stored
        u16le(&mut h, 0); // time
        u16le(&mut h, DOS_DATE);
        u32le(&mut h, crc);
        u32le(&mut h, size);
        u32le(&mut h, size);
        u16le(&mut h, name_len);
        u16le(&mut h, 0); // extra
        h.extend_from_slice(name.as_bytes());
        self.offset += header_len + u64::from(size);
        self.entries.push(Central { name: name.to_string(), crc, size, offset });
        Ok(h)
    }

    /// The bytes written so far (headers and data).
    pub fn written(&self) -> u64 {
        self.offset
    }

    /// The central directory and end record (emit last).
    pub fn finish(self) -> Result<Vec<u8>, String> {
        let mut cd = Vec::new();
        for e in &self.entries {
            u32le(&mut cd, CENTRAL_SIG);
            u16le(&mut cd, 20); // made by
            u16le(&mut cd, 20); // needed
            u16le(&mut cd, UTF8);
            u16le(&mut cd, 0);
            u16le(&mut cd, 0);
            u16le(&mut cd, DOS_DATE);
            u32le(&mut cd, e.crc);
            u32le(&mut cd, e.size);
            u32le(&mut cd, e.size);
            u16le(&mut cd, e.name.len() as u16);
            u16le(&mut cd, 0); // extra
            u16le(&mut cd, 0); // comment
            u16le(&mut cd, 0); // disk
            u16le(&mut cd, 0); // internal attrs
            u32le(&mut cd, 0); // external attrs
            u32le(&mut cd, e.offset);
            cd.extend_from_slice(e.name.as_bytes());
        }
        let cd_offset = u32::try_from(self.offset).map_err(|_| "backup too large".to_string())?;
        let cd_size = u32::try_from(cd.len()).map_err(|_| "backup too large".to_string())?;
        let n = self.entries.len() as u16;
        let mut out = cd;
        u32le(&mut out, END_SIG);
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u16le(&mut out, n);
        u16le(&mut out, n);
        u32le(&mut out, cd_size);
        u32le(&mut out, cd_offset);
        u16le(&mut out, 0); // comment
        Ok(out)
    }
}

/// One entry of a zip's central directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub crc: u32,
    pub size: u64,
    /// Offset of the entry's local header.
    pub header: u64,
    /// 0 = stored (the only method a restore reads).
    pub method: u16,
}

fn rd16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}
fn rd32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// How many bytes from the end of a zip to read to find its end record.
pub fn tail_len(file_len: u64) -> u64 {
    file_len.min(22 + 65_535)
}

/// The central directory's (offset, size) from the last [`tail_len`] bytes of a zip.
pub fn find_central(tail: &[u8]) -> Result<(u64, u64), String> {
    let at = (0..tail.len().saturating_sub(21)).rev().find(|&i| rd32(tail, i) == Some(END_SIG)).ok_or("not a zip file (no end record)")?;
    let size = rd32(tail, at + 12).ok_or("truncated zip")?;
    let offset = rd32(tail, at + 16).ok_or("truncated zip")?;
    if size == u32::MAX || offset == u32::MAX {
        return Err("zip64 backups are not supported".into());
    }
    Ok((u64::from(offset), u64::from(size)))
}

/// Parse a central directory.
pub fn parse_central(cd: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i.saturating_add(46) <= cd.len() {
        if rd32(cd, i) != Some(CENTRAL_SIG) {
            return Err("damaged zip directory".into());
        }
        let bad = || "damaged zip directory".to_string();
        let method = rd16(cd, i + 10).ok_or_else(bad)?;
        let crc = rd32(cd, i + 16).ok_or_else(bad)?;
        let size = rd32(cd, i + 24).ok_or_else(bad)?;
        let n = usize::from(rd16(cd, i + 28).ok_or_else(bad)?);
        let extra = usize::from(rd16(cd, i + 30).ok_or_else(bad)?);
        let comment = usize::from(rd16(cd, i + 32).ok_or_else(bad)?);
        let header = rd32(cd, i + 42).ok_or_else(bad)?;
        let name = cd.get(i + 46..i + 46 + n).ok_or_else(bad)?;
        out.push(ZipEntry { name: String::from_utf8_lossy(name).into_owned(), crc, size: u64::from(size), header: u64::from(header), method });
        i += 46 + n + extra + comment;
    }
    Ok(out)
}

/// Offset of an entry's data from its local header's first 30 bytes.
pub fn data_offset(entry: &ZipEntry, local: &[u8]) -> Result<u64, String> {
    if rd32(local, 0) != Some(LOCAL_SIG) {
        return Err(format!("{}: damaged entry", entry.name));
    }
    let n = rd16(local, 26).ok_or("truncated entry")?;
    let extra = rd16(local, 28).ok_or("truncated entry")?;
    Ok(entry.header + 30 + u64::from(n) + u64::from(extra))
}

/// Check an entry's bytes.
pub fn verify(entry: &ZipEntry, data: &[u8]) -> Result<(), String> {
    verify_sums(entry, data.len() as u64, crc32fast::hash(data))
}

/// Check an entry read in pieces: its length and CRC-32.
pub fn verify_sums(entry: &ZipEntry, len: u64, crc: u32) -> Result<(), String> {
    if entry.method != 0 {
        return Err(format!("{}: compressed entries are not supported (not a LightCraft backup?)", entry.name));
    }
    if len != entry.size || crc != entry.crc {
        return Err(format!("{}: damaged (checksum mismatch)", entry.name));
    }
    Ok(())
}

/// Check that an original's bytes are the photo its storage key names: originals are stored
/// under their content hash, and a restore that kept bytes under another photo's hash would
/// give that photo's catalog record a different image (the zip checksum can't tell).
pub fn verify_original(entry: &ZipEntry, hash: &str, data: &[u8]) -> Result<(), String> {
    verify_original_hash(entry, hash, &content_hash(data))
}

/// [`verify_original`] with the bytes' hash computed by the caller (streamed).
pub fn verify_original_hash(entry: &ZipEntry, hash: &str, actual: &str) -> Result<(), String> {
    if actual != hash {
        return Err(format!("{}: the file doesn't match its content hash (damaged or altered backup)", entry.name));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = ZipWriter::default();
        let mut out = Vec::new();
        for (n, d) in files {
            out.extend(w.entry(n, d).unwrap());
            out.extend_from_slice(d);
        }
        out.extend(w.finish().unwrap());
        out
    }

    #[test]
    fn written_backups_read_back() {
        let files: [(&str, &[u8]); 3] =
            [("README.txt", b"readme"), ("library/catalog.log", b"{\"seq\":1}\n"), ("originals/abc123/IMG 1.jpg", &[0, 1, 2, 3, 255])];
        let zip = build(&files);
        let tail = &zip[zip.len() - tail_len(zip.len() as u64) as usize..];
        let (off, size) = find_central(tail).unwrap();
        let entries = parse_central(&zip[off as usize..(off + size) as usize]).unwrap();
        assert_eq!(entries.len(), 3);
        for (e, (name, data)) in entries.iter().zip(files) {
            assert_eq!(e.name, name);
            let start = data_offset(e, &zip[e.header as usize..e.header as usize + 30]).unwrap() as usize;
            let got = &zip[start..start + e.size as usize];
            assert_eq!(got, data);
            verify(e, got).unwrap();
        }
        // damage is caught
        let e = &entries[2];
        assert!(verify(e, &[0, 1, 2, 3, 254]).is_err());
        assert!(find_central(b"not a zip at all, just some text").is_err());
    }

    #[test]
    fn originals_must_match_their_content_hash() {
        let red: &[u8] = b"red corner photo";
        let blue: &[u8] = b"blue corner photo";
        let hash = content_hash(red);
        let name = original_entry(&hash, "IMG 1.png");
        assert_eq!(entry_hash(&name), Some(hash.as_str()));
        // the entry's own bytes pass; other bytes under the same name (valid zip CRC) don't
        let zip = build(&[(name.as_str(), blue)]);
        let tail = &zip[zip.len() - tail_len(zip.len() as u64) as usize..];
        let (off, size) = find_central(tail).unwrap();
        let entries = parse_central(&zip[off as usize..(off + size) as usize]).unwrap();
        let entry = &entries[0];
        verify(entry, blue).unwrap();
        let err = verify_original(entry, &hash, blue).unwrap_err();
        assert!(err.contains(&name), "{err}");
        verify_original(entry, &hash, red).unwrap();
    }

    #[test]
    fn entry_names_are_safe() {
        assert_eq!(original_entry("abc", "a/b\\c.jpg"), "originals/abc/a_b_c.jpg");
        assert_eq!(original_entry("abc", ".."), "originals/abc/original");
        assert_eq!(entry_hash("originals/../library/catalog.log"), None);
        assert_eq!(entry_hash("library/catalog.log"), None);
        assert_eq!(library_file("library/catalog.snap"), Some("catalog.snap"));
        assert_eq!(library_file("library/../../x"), None);
        assert_eq!(library_file("library/evil.bin"), None);
    }

    #[test]
    fn size_limits_are_errors_not_panics() {
        let mut w = ZipWriter::at_offset(MAX_BYTES - 10);
        assert!(w.entry("x", &[0; 64]).is_err());
        assert!(ZipWriter::default().entry_for("y", u64::MAX, 0).is_err());
        // a directory that claims more than it holds
        assert!(parse_central(&[0x50, 0x4b, 0x01, 0x02]).unwrap().is_empty());
        let mut cd = vec![0u8; 46];
        cd[..4].copy_from_slice(&CENTRAL_SIG.to_le_bytes());
        cd[28] = 200; // a name past the end
        assert!(parse_central(&cd).is_err());
    }

    /// The streamed hash (pieces) equals the whole-file hash, so a backup can hash an original
    /// without holding it in memory.
    #[test]
    fn streamed_content_hash_matches() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut h = lightcraft_preview::Hasher128::new();
        for chunk in data.chunks(4093) {
            h.update(chunk);
        }
        assert_eq!(h.finish().to_string(), content_hash(&data));
    }
}
