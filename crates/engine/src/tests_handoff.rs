//! Phone ↔ computer handoff without a cloud (see [`crate::handoff`]).
//!
//! Scenarios, in the words of someone with LightCraft on a phone and a computer:
//!
//! * I edit photos on the phone and hand them off; the computer's library gets them with their
//!   edits. I edit one there and hand it back: the phone lists it as newer and reads it when I
//!   say so; the photos I didn't touch are not listed.
//! * A handoff older than what the library has is listed but never read, even when asked.
//! * A photo the computer already has under another name or folder is found by its content.
//! * A file that changed on the way, or a folder that isn't a handoff, is said so; nothing is
//!   read from it.

use std::path::{Path, PathBuf};

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use crate::Session;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("lc-handoff-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self, rel: &str) -> String {
        self.0.join(rel).components().collect::<PathBuf>().to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_png(path: &str, seed: u8) {
    let (w, h) = (24usize, 16usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 9) as u8, (i / w * 13) as u8, seed, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::create_dir_all(Path::new(path).parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A session whose clock reads `now` until changed with [`at`].
fn device(now: &str) -> Session {
    let mut s = Session::new().with_fs();
    at(&mut s, now);
    s
}

fn at(s: &mut Session, now: &str) {
    let now = now.to_string();
    s.clock = Box::new(move || now.clone());
}

fn import(s: &mut Session, paths: &[String]) {
    s.execute("library.import", &json!({"paths": paths})).unwrap();
}

fn id_of(s: &Session, name: &str) -> PhotoId {
    s.catalog.photos().find(|p| p.file_name == name && p.in_library()).map(|p| p.id).unwrap_or_else(|| panic!("no {name}"))
}

fn exposure(s: &Session, id: PhotoId) -> f64 {
    s.catalog.photo(id).unwrap().develop.light.exposure
}

fn set_exposure(s: &mut Session, id: PhotoId, v: f64) {
    s.selection.ids = vec![id];
    s.selection.active = Some(id);
    s.execute("develop.set", &json!({"ids": [id.0], "control": "light.exposure", "value": v})).unwrap();
}

fn hand_off(s: &mut Session, ids: &[PhotoId], dir: &str) -> Value {
    s.execute("export.handoff", &json!({"dir": dir, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap()
}

fn ids(v: &Value, key: &str) -> Vec<u64> {
    v[key].as_array().unwrap().iter().map(|m| m["id"].as_u64().unwrap()).collect()
}

/// A phone library holding `a.png` (edited) and `b.png`, imported at 09:00 and `a` edited at 10:00.
fn phone(dir: &Scratch) -> (Session, PhotoId, PhotoId) {
    write_png(&dir.path("phone/a.png"), 1);
    write_png(&dir.path("phone/b.png"), 2);
    let mut s = device("2026-10-01T09:00:00");
    import(&mut s, &[dir.path("phone")]);
    let (a, b) = (id_of(&s, "a.png"), id_of(&s, "b.png"));
    at(&mut s, "2026-10-01T10:00:00");
    set_exposure(&mut s, a, 1.0);
    (s, a, b)
}

#[test]
fn handoff_roundtrip_keeps_edits() {
    let dir = Scratch::new("roundtrip");
    let (mut phone, pa, pb) = phone(&dir);
    let out = hand_off(&mut phone, &[pa, pb], &dir.path("to-desk"));
    assert_eq!(out["photos"], 2, "{out}");
    assert!(Path::new(&dir.path("to-desk/lightcraft-handoff.json")).is_file());
    assert!(Path::new(&dir.path("to-desk/a.xmp")).is_file(), "{out}");
    // the manifest lists each original with its hash, id and last change
    let m = crate::handoff::read_manifest(&dir.path("to-desk")).unwrap();
    let a = m.photos.iter().find(|e| e.file == "a.png").unwrap();
    assert_eq!(a.id, pa.0);
    assert_eq!(a.edited.as_deref(), Some("2026-10-01T10:00:00"));
    assert_eq!(Some(a.hash.as_str()), phone.catalog.photo(pa).unwrap().content_hash.as_deref());
    assert_eq!(a.sidecar.as_deref(), Some("a.xmp"));

    // the computer: both are new, and come in with the phone's edits
    let mut desk = device("2026-10-02T08:00:00");
    let plan = desk.execute("library.scanHandoff", &json!({"dir": dir.path("to-desk")})).unwrap();
    assert_eq!(plan["new"].as_array().unwrap().len(), 2, "{plan}");
    let r = desk.execute("library.receiveHandoff", &json!({"dir": dir.path("to-desk")})).unwrap();
    assert_eq!(r["imported"], 2, "{r}");
    let (da, db) = (id_of(&desk, "a.png"), id_of(&desk, "b.png"));
    assert_eq!(exposure(&desk, da), 1.0, "the phone's edit came along");
    assert_eq!(exposure(&desk, db), 0.0);

    // edit one on the computer, hand both back
    at(&mut desk, "2026-10-02T09:00:00");
    set_exposure(&mut desk, da, 2.0);
    hand_off(&mut desk, &[da, db], &dir.path("to-phone"));

    // the phone: `a` is newer, `b` says nothing new
    let plan = phone.execute("library.scanHandoff", &json!({"dir": dir.path("to-phone")})).unwrap();
    assert_eq!(ids(&plan, "newer"), [pa.0], "{plan}");
    assert!(plan["new"].as_array().unwrap().is_empty(), "{plan}");
    assert_eq!(plan["same"], 1, "{plan}");
    // never the default: receiving without choosing reads nothing
    let r = phone.execute("library.receiveHandoff", &json!({"dir": dir.path("to-phone")})).unwrap();
    assert_eq!(r["read"], 0, "{r}");
    assert_eq!(exposure(&phone, pa), 1.0);
    let undo = phone.undo.len();
    let r = phone.execute("library.receiveHandoff", &json!({"dir": dir.path("to-phone"), "read": [pa.0]})).unwrap();
    assert_eq!(r["read"], 1, "{r}");
    assert_eq!(exposure(&phone, pa), 2.0, "the computer's edit is on the phone");
    assert_eq!(phone.undo.len(), undo + 1, "one undo step");
    assert_eq!(phone.undo.last().unwrap().label, "Receive Handoff");
    assert_eq!(phone.catalog.len(), 2, "nothing imported twice");
}

#[test]
fn older_handoff_never_overwrites() {
    let dir = Scratch::new("older");
    let (mut phone, pa, _) = phone(&dir);
    hand_off(&mut phone, &[pa], &dir.path("h1"));
    // the phone edits `a` again after handing it off
    at(&mut phone, "2026-10-01T11:00:00");
    set_exposure(&mut phone, pa, -1.0);
    // ...and the old handoff comes back (a copy left in a synced folder)
    let plan = phone.execute("library.scanHandoff", &json!({"dir": dir.path("h1")})).unwrap();
    assert_eq!(ids(&plan, "older"), [pa.0], "{plan}");
    assert!(plan["newer"].as_array().unwrap().is_empty(), "{plan}");
    let r = phone.execute("library.receiveHandoff", &json!({"dir": dir.path("h1"), "read": [pa.0]})).unwrap();
    assert_eq!(r["read"], 0, "{r}");
    assert_eq!(r["refused"][0][0], pa.0, "{r}");
    assert_eq!(exposure(&phone, pa), -1.0, "the library's newer edit stays");

    // an unknown time on the handoff's side is never newer either
    let dir2 = dir.path("h2");
    hand_off(&mut phone, &[pa], &dir2);
    let manifest = Path::new(&dir2).join(crate::handoff::MANIFEST);
    let mut m: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    m["photos"][0].as_object_mut().unwrap().remove("edited");
    std::fs::write(&manifest, serde_json::to_vec(&m).unwrap()).unwrap();
    std::fs::write(Path::new(&dir2).join("a.xmp"), std::fs::read(dir.path("h1/a.xmp")).unwrap()).unwrap();
    let plan = phone.execute("library.scanHandoff", &json!({"dir": dir2})).unwrap();
    assert_eq!(ids(&plan, "older"), [pa.0], "{plan}");
}

#[test]
fn hash_match_finds_moved_photo() {
    let dir = Scratch::new("moved");
    let (mut phone, pa, _) = phone(&dir);
    // the computer has `a` already, renamed and in another folder, imported before the phone's edit
    std::fs::create_dir_all(dir.path("desk/2026")).unwrap();
    std::fs::copy(dir.path("phone/a.png"), dir.path("desk/2026/holiday-001.png")).unwrap();
    let mut desk = device("2026-10-01T09:30:00");
    import(&mut desk, &[dir.path("desk")]);
    let d = id_of(&desk, "holiday-001.png");
    hand_off(&mut phone, &[pa], &dir.path("h"));
    let plan = desk.execute("library.scanHandoff", &json!({"dir": dir.path("h")})).unwrap();
    assert!(plan["new"].as_array().unwrap().is_empty(), "found by content, not imported again: {plan}");
    assert_eq!(ids(&plan, "newer"), [d.0], "{plan}");
    assert_eq!(plan["newer"][0]["name"], "holiday-001.png");
    desk.execute("library.receiveHandoff", &json!({"dir": dir.path("h"), "read": [d.0]})).unwrap();
    assert_eq!(exposure(&desk, d), 1.0);
    assert_eq!(desk.catalog.len(), 1);
}

#[test]
fn changed_original_is_damaged_and_bad_folders_are_refused() {
    let dir = Scratch::new("damaged");
    let (mut phone, pa, pb) = phone(&dir);
    hand_off(&mut phone, &[pa, pb], &dir.path("h"));
    write_png(&dir.path("h/b.png"), 9);
    let mut desk = device("2026-10-02T08:00:00");
    let plan = desk.execute("library.scanHandoff", &json!({"dir": dir.path("h")})).unwrap();
    assert_eq!(plan["new"].as_array().unwrap().len(), 1, "{plan}");
    assert!(plan["damaged"][0][1].as_str().unwrap().contains("changed"), "{plan}");
    // a handoff folder is never written into twice
    let e = phone.execute("export.handoff", &json!({"dir": dir.path("h"), "ids": [pa.0]})).unwrap_err().to_string();
    assert!(e.contains("already holds a handoff"), "{e}");
    // a folder without a manifest, a broken one, one from a newer version, one naming files
    // outside the folder: errors, never a panic
    let e = desk.execute("library.scanHandoff", &json!({"dir": dir.path("phone")})).unwrap_err().to_string();
    assert!(e.contains("not a handoff folder"), "{e}");
    let manifest = dir.path("h/lightcraft-handoff.json");
    std::fs::write(&manifest, b"{not json").unwrap();
    assert!(desk.execute("library.scanHandoff", &json!({"dir": dir.path("h")})).is_err());
    std::fs::write(&manifest, br#"{"format": 99, "app": "LightCraft", "created": "", "photos": []}"#).unwrap();
    let e = desk.execute("library.scanHandoff", &json!({"dir": dir.path("h")})).unwrap_err().to_string();
    assert!(e.contains("newer LightCraft"), "{e}");
    std::fs::write(
        &manifest,
        br#"{"format": 1, "app": "LightCraft", "created": "", "photos": [{"file": "../phone/a.png", "hash": "x", "size": 1, "id": 1}]}"#,
    )
    .unwrap();
    let plan = desk.execute("library.scanHandoff", &json!({"dir": dir.path("h")})).unwrap();
    assert!(plan["new"].as_array().unwrap().is_empty() && plan["damaged"].as_array().unwrap().len() == 1, "{plan}");
}

#[test]
fn demo_photos_and_virtual_copies_are_skipped() {
    let dir = Scratch::new("skipped");
    let (mut phone, pa, _) = phone(&dir);
    phone.execute("photo.virtualCopy", &json!({"ids": [pa.0]})).unwrap();
    assert!(phone.catalog.photos().any(|p| p.copy_of.is_some()));
    let copies: Vec<u64> = phone.catalog.photos().filter(|p| p.copy_of.is_some()).map(|p| p.id.0).collect();
    let mut all: Vec<u64> = vec![pa.0];
    all.extend(&copies);
    let out = phone.execute("export.handoff", &json!({"dir": dir.path("h"), "ids": all})).unwrap();
    assert_eq!(out["photos"], 1, "{out}");
    assert_eq!(out["skipped"].as_array().unwrap().len(), copies.len(), "{out}");
    let e = phone.execute("export.handoff", &json!({"dir": dir.path("h2"), "ids": []})).unwrap_err().to_string();
    assert!(e.contains("no photos"), "{e}");
}

#[test]
fn handoff_folder_name_is_new() {
    let dir = Scratch::new("folder");
    let base = dir.path("");
    let first = crate::handoff::folder_in(&base, "2026-10-01T10:20:30");
    assert!(first.ends_with("LightCraft Handoff 2026-10-01 10-20-30"), "{first:?}");
    std::fs::create_dir_all(&first).unwrap();
    let second = crate::handoff::folder_in(&base, "2026-10-01T10:20:30");
    assert!(second.ends_with("LightCraft Handoff 2026-10-01 10-20-30 2"), "{second:?}");
}
