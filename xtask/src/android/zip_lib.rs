//! The zip step of the APK: copy every entry of aapt2's output unchanged, then append the native
//! libraries Stored (uncompressed) and 16 KB-aligned so they can be mapped in place
//! (`extractNativeLibs=false`). Nothing is compressed or decompressed here.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Alignment of a native library's data inside the APK (Android's 16 KB page).
pub const SO_ALIGNMENT: u16 = 16384;

/// A native library to add: the file and the name it gets in the APK (`lib/<abi>/<file>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLib {
    pub abi: String,
    pub file: PathBuf,
}

impl NativeLib {
    /// `lib/<abi>/<file name>`.
    pub fn entry_name(&self) -> Result<String, String> {
        let name = self.file.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("{}: not a file name", self.file.display()))?;
        Ok(format!("lib/{}/{name}", self.abi))
    }
}

fn zerr(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}: {e}")
}

/// Writes `output`: every entry of `base` (aapt2's APK) raw-copied, then each of `libs` Stored and
/// aligned to [`SO_ALIGNMENT`].
pub fn assemble(base: &Path, libs: &[NativeLib], output: &Path) -> Result<(), String> {
    let input = File::open(base).map_err(|e| zerr(&base.display().to_string(), e))?;
    let mut source = ZipArchive::new(BufReader::new(input)).map_err(|e| zerr(&base.display().to_string(), e))?;
    let out_file = File::create(output).map_err(|e| zerr(&output.display().to_string(), e))?;
    let mut writer = ZipWriter::new(BufWriter::new(out_file));
    for i in 0..source.len() {
        let entry = source.by_index_raw(i).map_err(|e| zerr(&format!("{} entry {i}", base.display()), e))?;
        let name = entry.name().to_owned();
        writer.raw_copy_file(entry).map_err(|e| zerr(&format!("copy {name}"), e))?;
    }
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).with_alignment(SO_ALIGNMENT);
    for lib in libs {
        let name = lib.entry_name()?;
        let mut data = File::open(&lib.file).map_err(|e| zerr(&lib.file.display().to_string(), e))?;
        writer.start_file(&name, stored).map_err(|e| zerr(&format!("add {name}"), e))?;
        std::io::copy(&mut data, &mut writer).map_err(|e| zerr(&format!("add {name}"), e))?;
    }
    let mut finished = writer.finish().map_err(|e| zerr(&output.display().to_string(), e))?;
    finished.flush().map_err(|e| zerr(&output.display().to_string(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// The text of the Deflated entry: long and repetitive, so the compressed form is smaller.
    fn deflated_body() -> Vec<u8> {
        b"<resources><string name=\"app_name\">LightCraft</string></resources>".repeat(40)
    }

    /// A zip written by hand, entry by entry as `(name, method, uncompressed bytes, stored bytes)`:
    /// the zip crate has no compressor here, and real aapt2 output Deflates its XML.
    fn hand_zip(entries: &[(&str, u16, &[u8], &[u8])]) -> Vec<u8> {
        let (mut body, mut central) = (Vec::new(), Vec::new());
        for (name, method, plain, stored) in entries {
            let mut crc = flate2::Crc::new();
            crc.update(plain);
            let offset = body.len() as u32;
            let fields = |out: &mut Vec<u8>| {
                out.extend_from_slice(&0u16.to_le_bytes()); // flags
                out.extend_from_slice(&method.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes()); // time
                out.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
                out.extend_from_slice(&crc.sum().to_le_bytes());
                out.extend_from_slice(&(stored.len() as u32).to_le_bytes());
                out.extend_from_slice(&(plain.len() as u32).to_le_bytes());
                out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes()); // extra length
            };
            body.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            body.extend_from_slice(&20u16.to_le_bytes());
            fields(&mut body);
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(stored);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // made by
            central.extend_from_slice(&20u16.to_le_bytes()); // needed
            fields(&mut central);
            central.extend_from_slice(&[0; 2 + 2 + 2 + 4]); // comment length, disk, internal and external attributes
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let mut zip = body;
        let central_offset = zip.len() as u32;
        zip.extend_from_slice(&central);
        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[0; 4]); // disk numbers
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(central.len() as u32).to_le_bytes());
        zip.extend_from_slice(&central_offset.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip
    }

    /// A fake aapt2 output in a fresh folder: Stored manifest, table and icon, and a Deflated
    /// string resource like the real thing; returns the folder and the path of the zip.
    fn fake_aapt2_output(tag: &str) -> (PathBuf, PathBuf) {
        use flate2::{Compression, write::DeflateEncoder};
        let dir = std::env::temp_dir().join(format!("xtask-zip-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base.apk");
        let plain = deflated_body();
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&plain).unwrap();
        let deflated = encoder.finish().unwrap();
        assert!(deflated.len() < plain.len());
        let zip = hand_zip(&[
            ("AndroidManifest.xml", 0, b"<manifest/>", b"<manifest/>"),
            ("resources.arsc", 0, b"arsc-table-bytes", b"arsc-table-bytes"),
            ("res/values/strings.xml", 8, &plain, &deflated),
            ("res/mipmap-hdpi/ic_launcher.png", 0, b"\x89PNG-not-really", b"\x89PNG-not-really"),
        ]);
        std::fs::write(&base, zip).unwrap();
        (dir, base)
    }

    fn fake_so(dir: &Path, abi: &str, size: usize) -> NativeLib {
        let sub = dir.join(abi);
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("liblightcraft_android.so");
        std::fs::write(&file, (0..size).map(|i| (i % 251) as u8).collect::<Vec<u8>>()).unwrap();
        NativeLib { abi: abi.to_owned(), file }
    }

    fn open(path: &Path) -> ZipArchive<BufReader<File>> {
        ZipArchive::new(BufReader::new(File::open(path).unwrap())).unwrap()
    }

    #[test]
    fn so_entry_is_stored_uncompressed() {
        let (dir, base) = fake_aapt2_output("stored");
        let libs = [fake_so(&dir, "arm64-v8a", 100_000), fake_so(&dir, "x86_64", 50_000)];
        let out = dir.join("out.apk");
        assemble(&base, &libs, &out).unwrap();
        let mut zip = open(&out);
        for (abi, size) in [("arm64-v8a", 100_000u64), ("x86_64", 50_000)] {
            let mut entry = zip.by_name(&format!("lib/{abi}/liblightcraft_android.so")).unwrap();
            assert_eq!(entry.compression(), CompressionMethod::Stored);
            assert_eq!(entry.size(), size);
            assert_eq!(entry.compressed_size(), size, "stored means no size change");
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            assert_eq!(data, std::fs::read(dir.join(abi).join("liblightcraft_android.so")).unwrap());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn so_entry_data_is_16k_aligned() {
        let (dir, base) = fake_aapt2_output("aligned");
        // odd sizes for the first library so the second would not start aligned by chance
        let libs = [fake_so(&dir, "arm64-v8a", 100_001), fake_so(&dir, "x86_64", 777)];
        let out = dir.join("out.apk");
        assemble(&base, &libs, &out).unwrap();
        let mut zip = open(&out);
        for abi in ["arm64-v8a", "x86_64"] {
            let entry = zip.by_name(&format!("lib/{abi}/liblightcraft_android.so")).unwrap();
            assert_eq!(entry.data_start() % u64::from(SO_ALIGNMENT), 0, "{abi} data starts at {}", entry.data_start());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copied_entries_unchanged() {
        let (dir, base) = fake_aapt2_output("copied");
        let out = dir.join("out.apk");
        assemble(&base, &[fake_so(&dir, "arm64-v8a", 4096)], &out).unwrap();
        let (mut before, mut after) = (open(&base), open(&out));
        assert_eq!(after.len(), before.len() + 1);
        let mut saw_deflated = false;
        // by_index_raw: with the zip crate's features off a Deflated entry reads as Unsupported(8),
        // and only the raw copy (the compressed bytes untouched) is allowed to handle it
        for i in 0..before.len() {
            let mut a = before.by_index_raw(i).unwrap();
            let name = a.name().to_owned();
            let mut abytes = Vec::new();
            a.read_to_end(&mut abytes).unwrap();
            let at = after.index_for_name(&name).unwrap();
            let mut b = after.by_index_raw(at).unwrap();
            assert_eq!(
                (a.compression(), a.crc32(), a.size(), a.compressed_size()),
                (b.compression(), b.crc32(), b.size(), b.compressed_size()),
                "{name}"
            );
            let mut bbytes = Vec::new();
            b.read_to_end(&mut bbytes).unwrap();
            assert_eq!(abytes, bbytes, "{name}");
            if name == "res/values/strings.xml" {
                saw_deflated = true;
                assert_ne!(a.compression(), CompressionMethod::Stored, "the fixture must hold a compressed entry");
                assert!(a.compressed_size() < a.size());
                let mut inflated = Vec::new();
                flate2::read::DeflateDecoder::new(&bbytes[..]).read_to_end(&mut inflated).unwrap();
                assert_eq!(inflated, deflated_body(), "the copied bytes still inflate to the original");
            }
        }
        assert!(saw_deflated);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_inputs_are_errors() {
        let (dir, base) = fake_aapt2_output("missing");
        let gone = NativeLib { abi: "arm64-v8a".to_owned(), file: dir.join("nope.so") };
        assert!(assemble(&base, &[gone], &dir.join("o.apk")).unwrap_err().contains("nope.so"));
        std::fs::write(dir.join("bad.apk"), b"not a zip").unwrap();
        assert!(assemble(&dir.join("bad.apk"), &[], &dir.join("o2.apk")).is_err());
        assert!(assemble(&dir.join("absent.apk"), &[], &dir.join("o3.apk")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
