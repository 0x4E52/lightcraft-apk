//! Library backups for the browser build: the zip format of [`lightcraft_engine::backup`] (shared
//! with the Android host), written from and restored into browser storage. A backup is built one
//! entry at a time so only one file is in wasm memory at once, and read back the same way.
//!
//! What is the browser's own: the readme, where restored entries go ([`restore_key`]: library
//! files into a new library folder, originals under their content hash), and the pointer to the
//! library in use ([`ACTIVE_LIBRARY`]).
//!
//! Restoring never deletes anything: the library files go to a new library folder in browser
//! storage, originals that aren't stored yet are added (they're named by content, so existing
//! ones are identical; a restored one must match its content hash, see [`verify_original`]),
//! and only then does a small pointer file ([`ACTIVE_LIBRARY`]) switch to the restored library.
//! The library that was there before stays in storage.

pub use lightcraft_engine::backup::{
    MAX_BYTES, ZipEntry, ZipWriter, data_offset, find_central, original_entry, parse_central, tail_len, valid_hash, verify, verify_original,
};

/// Storage key of the file naming the library folder in use (absent: `library`).
pub const ACTIVE_LIBRARY: &str = "active-library";

/// The default library folder.
pub const DEFAULT_LIBRARY_DIR: &str = "library";

/// The readme inside every backup.
pub const README: &str = "LightCraft library backup (browser version)\n\n\
library/      the catalog (catalog.snap + catalog.log), presets, view state and preferences\n\
originals/    every imported photo, as originals/<content hash>/<file name>\n\n\
Restore it in LightCraft for the web with File > Restore Library from Backup...\n\
The photos in originals/ are the untouched files you imported; edits live in the catalog.\n";

/// Is `name` a safe library folder name (`library` or `library-<letters, digits, ->`)?
pub fn valid_library_dir(name: &str) -> bool {
    name == DEFAULT_LIBRARY_DIR
        || name.strip_prefix("library-").is_some_and(|s| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Where a backup entry goes when restoring into library folder `dir`: `Some(storage key)`, or
/// `None` to skip it (README, the manifest, thumbnails, anything unknown or unsafe).
pub fn restore_key(entry: &str, dir: &str) -> Option<String> {
    if entry.starts_with("library/") {
        return lightcraft_engine::backup::library_file(entry).map(|name| format!("{dir}/{name}"));
    }
    lightcraft_engine::backup::entry_hash(entry).map(|hash| format!("originals/{hash}"))
}

/// A name for the restored library's folder.
pub fn restored_dir_name(now_ms: f64) -> String {
    format!("library-restored-{}", now_ms.max(0.0) as u64)
}

#[cfg(test)]
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
    fn originals_must_match_their_content_hash() {
        let red: &[u8] = b"red corner photo";
        let blue: &[u8] = b"blue corner photo";
        let hash = crate::store::content_hash(red);
        let name = original_entry(&hash, "IMG 1.png");
        let key = restore_key(&name, "d").unwrap();
        let key_hash = key.strip_prefix("originals/").unwrap();
        assert_eq!(key_hash, hash);
        // the entry's own bytes pass; other bytes under the same name (valid zip CRC) don't
        let zip = build(&[(name.as_str(), blue)]);
        let tail = &zip[zip.len() - tail_len(zip.len() as u64) as usize..];
        let (off, size) = find_central(tail).unwrap();
        let entries = parse_central(&zip[off as usize..(off + size) as usize]).unwrap();
        let entry = &entries[0];
        verify(entry, blue).unwrap();
        let err = verify_original(entry, key_hash, blue).unwrap_err();
        assert!(err.contains(&name), "{err}");
        verify_original(entry, key_hash, red).unwrap();
    }

    #[test]
    fn restore_keys_are_safe() {
        assert_eq!(restore_key("library/catalog.snap", "library-restored-1").as_deref(), Some("library-restored-1/catalog.snap"));
        assert_eq!(restore_key("library/../../x", "library-r"), None);
        assert_eq!(restore_key("library/evil.bin", "library-r"), None);
        assert_eq!(restore_key("originals/abc123/IMG.jpg", "d").as_deref(), Some("originals/abc123"));
        assert_eq!(restore_key("originals/../library/catalog.log", "d"), None);
        assert_eq!(restore_key("README.txt", "d"), None);
        assert_eq!(restore_key("backup.json", "d"), None);
        assert_eq!(restore_key("thumbs/x.jpg", "d"), None);
        assert!(valid_library_dir("library") && valid_library_dir("library-restored-17"));
        assert!(!valid_library_dir("library-../x") && !valid_library_dir("originals") && !valid_library_dir("library-"));
    }
}
