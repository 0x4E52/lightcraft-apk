//! Library folder backups (APK-50): a backup restores the catalog (edits, albums) and the
//! Copy-imported originals into another folder, photos added in place are listed by path, a
//! backup that can't fit is refused before anything is written, and a tampered entry stops the
//! restore before anything is written.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use serde_json::json;

use super::*;
use crate::Session;
use crate::backup::{MANIFEST, find_central, parse_central, tail_len};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-backup-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A small procedural PNG (distinct per seed).
fn write_png(path: &Path, seed: u8) {
    let (w, h) = (40usize, 30usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 5) as u8, (i / w * 7) as u8, seed, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn path_of(s: &Session, id: lightcraft_catalog::PhotoId) -> String {
    match &s.catalog.photo(id).unwrap().source {
        lightcraft_catalog::Source::File { path } => path.clone(),
        _ => panic!("not a file"),
    }
}

/// A library with one Copy-imported photo (rated 4, in an album) and one added in place.
fn library(root: &Path) -> (Session, PathBuf, lightcraft_catalog::PhotoId, PathBuf) {
    let lib = root.join("library");
    let src = root.join("camera");
    write_png(&src.join("copied.png"), 1);
    let dcim = root.join("DCIM");
    write_png(&dcim.join("in-place.png"), 2);
    let mut s = Session::new().with_fs();
    s.open_library(&lib, false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [src.join("copied.png").to_string_lossy()], "mode": "copy"})).unwrap();
    let copied = lightcraft_catalog::PhotoId(r["imported"][0].as_u64().unwrap());
    s.execute("library.import", &json!({"paths": [dcim.join("in-place.png").to_string_lossy()]})).unwrap();
    assert!(Path::new(&path_of(&s, copied)).starts_with(lib.join("Originals")));
    s.execute("library.select", &json!({"ids": [copied.0]})).unwrap();
    s.execute("photo.rate", &json!({"rating": 4})).unwrap();
    s.execute("album.create", &json!({"name": "Trip", "addSelected": true})).unwrap();
    (s, lib, copied, dcim.join("in-place.png"))
}

#[test]
fn backup_roundtrip_restores_catalog_and_originals() {
    let root = temp_dir("roundtrip");
    let (mut s, lib, copied, in_place) = library(&root);
    let original = std::fs::read(path_of(&s, copied)).unwrap();
    let plan = Plan::of(&mut s).unwrap();
    assert_eq!((plan.originals(), plan.in_place()), (1, 1));
    let out = root.join("Documents").join(backup_name(1_793_000_000));
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let no = AtomicBool::new(false);
    let written = write(&plan, &out, &no).unwrap();
    assert_eq!((written.originals, written.in_place, written.missing.len()), (1, 1, 0));
    assert!(written.bytes <= plan.needed_bytes(), "{} > {}", written.bytes, plan.needed_bytes());
    assert_eq!(std::fs::metadata(&out).unwrap().len(), written.bytes);
    drop(s);

    // the manifest lists the photo added in place by path; it isn't copied
    let v = verify_file(&out, &no).unwrap();
    assert_eq!(v.originals, 1);
    assert_eq!(v.manifest.in_place, [in_place.to_string_lossy().to_string()]);
    assert_eq!(v.manifest.library_dir, lib.to_string_lossy());

    // restored into another folder (a reinstall under another id): the originals move along
    let restored = root.join("restored");
    extract(&out, &v, &restored, &no).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(&restored, false).unwrap();
    assert_eq!(rebase(&mut s, Path::new(&v.manifest.library_dir)).unwrap(), 1);
    assert!(s.undo.is_empty(), "the relink isn't an undo step");
    let p = s.catalog.photo(copied).unwrap();
    assert_eq!(p.rating, 4, "edits are back");
    let path = path_of(&s, copied);
    assert!(Path::new(&path).starts_with(restored.join("Originals")), "{path}");
    assert_eq!(std::fs::read(&path).unwrap(), original, "the original's bytes are back");
    assert!(s.catalog.albums().any(|a| a.name == "Trip" && a.photos.contains(&copied)), "albums are back");
    // and it stays that way after a restart
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(&restored, false).unwrap();
    assert_eq!(path_of(&s, copied), path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn backup_refuses_when_space_is_short() {
    let e = check_space("the backup", 3_200_000_000, Some(1_100_000_000)).unwrap_err();
    assert!(e.contains("needs 3.2 GB, 1.1 GB free"), "{e}");
    check_space("the backup", 10, Some(10)).unwrap();
    check_space("the backup", 10, None).unwrap();
    assert_eq!(size_text(120_000_000), "120 MB");
    assert_eq!(size_text(12), "1 KB");
    // a plan's need covers what it writes (the space check runs before anything is written)
    let root = temp_dir("space");
    let (mut s, ..) = library(&root);
    let plan = Plan::of(&mut s).unwrap();
    assert!(check_space("the backup", plan.needed_bytes(), Some(plan.needed_bytes() - 1)).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn restore_rejects_tampered_entry() {
    let root = temp_dir("tamper");
    let (mut s, _lib, copied, _) = library(&root);
    let plan = Plan::of(&mut s).unwrap();
    let out = root.join("backup.zip");
    let no = AtomicBool::new(false);
    write(&plan, &out, &no).unwrap();
    let mut zip = std::fs::read(&out).unwrap();
    let tail_at = zip.len() - tail_len(zip.len() as u64) as usize;
    let (off, size) = find_central(&zip[tail_at..]).unwrap();
    let entries = parse_central(&zip[off as usize..(off + size) as usize]).unwrap();
    let original = entries.iter().find(|e| e.name.starts_with("originals/")).unwrap();
    // flip a byte inside the original's data
    let name_len = u16::from_le_bytes([zip[original.header as usize + 26], zip[original.header as usize + 27]]) as usize;
    let at = original.header as usize + 30 + name_len + 10;
    zip[at] ^= 0xff;
    let tampered = root.join("tampered.zip");
    std::fs::write(&tampered, &zip).unwrap();
    let e = verify_file(&tampered, &no).unwrap_err();
    assert!(e.contains("damaged (checksum mismatch)") && e.contains("originals/"), "{e}");

    // bytes swapped for another photo's, with a matching CRC: the content hash catches it
    let fake = entries.iter().find(|e| e.name.starts_with("originals/")).unwrap().name.clone();
    let mut w = crate::backup::ZipWriter::default();
    let mut forged = Vec::new();
    for (n, d) in [("library/catalog.snap", &b"{}"[..]), (fake.as_str(), b"other bytes"), (MANIFEST, b"{}")] {
        forged.extend(w.entry(n, d).unwrap());
        forged.extend_from_slice(d);
    }
    forged.extend(w.finish().unwrap());
    let forged_path = root.join("forged.zip");
    std::fs::write(&forged_path, &forged).unwrap();
    let e = verify_file(&forged_path, &no).unwrap_err();
    assert!(e.contains("doesn't match its content hash"), "{e}");

    // extracting stops on damage too, and leaves no folder behind
    let v = verify_file(&out, &no).unwrap();
    let staging = root.join("staging");
    extract(&tampered, &v, &staging, &no).unwrap_err();
    assert!(!staging.exists(), "a failed restore leaves nothing");
    // the library itself was never touched
    assert_eq!(s.catalog.photo(copied).unwrap().rating, 4);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn restore_refuses_what_is_not_a_device_backup() {
    let root = temp_dir("notbackup");
    let no = AtomicBool::new(false);
    let junk = root.join("junk.zip");
    std::fs::write(&junk, b"not a zip at all").unwrap();
    assert!(verify_file(&junk, &no).unwrap_err().contains("isn't a LightCraft library backup"));
    // a web backup (no manifest) says where to restore it
    let mut w = crate::backup::ZipWriter::default();
    let mut web = Vec::new();
    web.extend(w.entry("library/catalog.log", b"").unwrap());
    web.extend(w.finish().unwrap());
    let web_path = root.join("web.zip");
    std::fs::write(&web_path, &web).unwrap();
    assert!(verify_file(&web_path, &no).unwrap_err().contains("for the web"));
    assert!(verify_file(&root.join("missing.zip"), &no).is_err());
    // a manifest that would place an original outside the folder is not followed
    let m = crate::backup::Manifest {
        format: 1,
        library_dir: "/lib".into(),
        originals: vec![crate::backup::ManifestOriginal { entry: "originals/abc/x.png".into(), path: "/lib/../etc/x.png".into() }],
        in_place: Vec::new(),
    };
    assert_eq!(original_target(&m, "originals/abc/x.png"), None);
    // nor one that would land on a library file or anywhere outside Originals/
    for path in ["/lib/catalog.snap", "/lib/catalog.log", "/lib/prefs.json", "/lib/Previews/x.png", "/lib/Originals", "/lib"] {
        let m = crate::backup::Manifest {
            originals: vec![crate::backup::ManifestOriginal { entry: "originals/abc/x.png".into(), path: path.into() }],
            ..m.clone()
        };
        assert_eq!(original_target(&m, "originals/abc/x.png"), None, "{path}");
    }
    let ok = crate::backup::Manifest {
        originals: vec![crate::backup::ManifestOriginal { entry: "originals/abc/x.png".into(), path: "/lib/Originals/2026/x.png".into() }],
        ..m
    };
    assert_eq!(original_target(&ok, "originals/abc/x.png"), Some(PathBuf::from("Originals/2026/x.png")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cancelled_backup_leaves_no_file() {
    let root = temp_dir("cancel");
    let (mut s, ..) = library(&root);
    let plan = Plan::of(&mut s).unwrap();
    let out = root.join("backup.zip");
    let e = write(&plan, &out, &AtomicBool::new(true)).unwrap_err();
    assert!(e.contains("cancelled"), "{e}");
    assert!(!out.exists());
    let leftovers: Vec<_> =
        std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains("tmp")).collect();
    assert!(leftovers.is_empty(), "no temp file left");
    let _ = std::fs::remove_dir_all(&root);
}

/// A library over the zip's limits (no zip64) is refused before anything is read or written.
#[test]
fn backup_over_the_zip_limits_is_refused_first() {
    let root = temp_dir("limits");
    let (mut s, ..) = library(&root);
    let plan = Plan::of(&mut s).unwrap();
    plan.check_limits().unwrap();
    let big = Plan { originals_bytes: crate::backup::MAX_BYTES, ..plan.clone() };
    let e = big.check_limits().unwrap_err();
    assert!(e.contains("too large for one backup file") && e.contains("4.2 GB"), "{e}");
    let original = plan.originals.first().cloned().unwrap();
    let many = Plan { originals: vec![original; crate::backup::MAX_ENTRIES], ..plan.clone() };
    assert!(many.check_limits().unwrap_err().contains("65,535 files"));
    let _ = std::fs::remove_dir_all(&root);
}

/// A photo inside the library folder but outside `Originals/` isn't restored by a restore, so a
/// backup lists it by path instead of copying it.
#[test]
fn photo_outside_originals_is_listed_by_path() {
    let root = temp_dir("stray");
    let (mut s, lib, ..) = library(&root);
    let stray = lib.join("Stray").join("stray.png");
    write_png(&stray, 5);
    s.execute("library.import", &json!({"paths": [stray.to_string_lossy()]})).unwrap();
    assert!(s.catalog.photos().any(|p| matches!(&p.source, lightcraft_catalog::Source::File { path } if Path::new(path) == stray)));
    let plan = Plan::of(&mut s).unwrap();
    assert_eq!((plan.originals(), plan.in_place()), (1, 2));
    assert!(plan.in_place.contains(&stray.to_string_lossy().into_owned()));
    let _ = std::fs::remove_dir_all(&root);
}
