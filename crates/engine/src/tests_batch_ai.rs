//! Auto Settings and Update AI Masks on a selection (APK-60): each photo its own analysis, one undo step, photos that
//! can't be read skipped with the reason, SAM 3 selections computed again per photo (a stand-in for the model here),
//! heuristic masks left to follow each photo, and the activity row with ✕.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lightcraft_catalog::{Op, Photo, PhotoId, Source};
use lightcraft_develop::{MaskShape, SegMask};
use lightcraft_raster::Rgb32f;
use serde_json::{Value, json};

use crate::Session;

/// The grey level a test photo's file decodes to, from its name; `None`: the file can't be read.
fn level(path: &str) -> Option<f32> {
    ["dark", "mid", "bright"].iter().zip([0.02f32, 0.12, 0.6]).find(|(n, _)| path.contains(*n)).map(|(_, v)| v)
}

fn photo(id: u64, name: &str) -> Photo {
    Photo::new(PhotoId(id), Source::File { path: format!("/nowhere/apk60/{name}") }, name, "JPEG", 600, 400, "2026-10-31T00:00:00")
}

/// A session with photos 1 (dark), 2 (mid), 3 (bright) and 4 (a file that can't be read), decoded by a stand-in
/// loader that waits while `gate` is closed.
fn session(gate: Arc<AtomicBool>) -> Session {
    let mut s = Session::new();
    s.media.file_loader = Some(Arc::new(move |p: &str, edge: usize| {
        let t = Instant::now();
        while !gate.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(2));
        }
        let v = level(p).ok_or_else(|| format!("{p}: no such file"))?;
        let (w, h) = (edge.clamp(8, 96), (edge.clamp(8, 96) * 2 / 3).max(1));
        Ok((Rgb32f::filled(w, h, [v, v, v]), Default::default()))
    }));
    s.execute_fn("test.add", |s| {
        for (id, name) in [(1, "dark.jpg"), (2, "mid.jpg"), (3, "bright.jpg"), (4, "gone.jpg")] {
            s.apply_system(Op::AddPhoto { photo: Box::new(photo(id, name)) })?;
        }
        Ok(Value::Null)
    })
    .unwrap();
    s
}

fn open() -> Session {
    session(Arc::new(AtomicBool::new(true)))
}

fn exposure(s: &Session, id: u64) -> f64 {
    s.develop_of(PhotoId(id)).unwrap().light.exposure
}

#[test]
fn auto_on_selection_analyses_each_photo() {
    // what Auto Settings gives each photo on its own, one at a time
    let mut one = open();
    let mut alone = Vec::new();
    for id in [1u64, 2, 3] {
        one.execute("library.select", &json!({"ids": [id]})).unwrap();
        let a = one.execute("develop.auto", &json!({})).unwrap();
        alone.push(a["exposure"].as_f64().unwrap());
    }
    assert!(alone[0] > alone[1] && alone[1] > alone[2], "darker photos get more exposure: {alone:?}");
    // the same values, each from its own photo, on the selection at once
    let mut s = open();
    s.execute("library.select", &json!({"ids": [1, 2, 3]})).unwrap();
    let r = s.execute("develop.auto", &json!({"selection": true})).unwrap();
    assert_eq!((r["kind"].as_str(), r["photos"].as_u64(), r["changed"].as_u64()), (Some("auto"), Some(3), Some(3)), "{r}");
    assert_eq!(r["skipped"], json!([]));
    for (i, id) in [1u64, 2, 3].into_iter().enumerate() {
        assert_eq!(exposure(&s, id), alone[i], "photo {id}");
        let d = s.develop_of(PhotoId(id)).unwrap();
        let o = one.develop_of(PhotoId(id)).unwrap();
        assert_eq!((d.light.highlights, d.light.shadows, d.color.vibrance), (o.light.highlights, o.light.shadows, o.color.vibrance));
    }
    // `ids` names them without selecting; the selection is left as it was
    let mut s = open();
    s.execute("library.select", &json!({"ids": [3]})).unwrap();
    let r = s.execute("develop.auto", &json!({"ids": [1, 2]})).unwrap();
    assert_eq!(r["changed"], 2, "{r}");
    assert_eq!((exposure(&s, 1), exposure(&s, 2), exposure(&s, 3)), (alone[0], alone[1], 0.0));
    assert_eq!(s.selected(), vec![PhotoId(3)]);
    // no photos named and none selected: refused, nothing started
    assert!(s.execute("develop.auto", &json!({"ids": []})).is_err());
    assert!(s.execute("develop.auto", &json!({"ids": [99]})).unwrap_err().to_string().contains("no photo 99"));
    assert!(!s.batch_ai.running());
}

#[test]
fn auto_on_selection_is_one_undo_step() {
    let mut s = open();
    s.execute("library.select", &json!({"ids": [1, 2, 3]})).unwrap();
    let undo = s.undo.len();
    s.execute("develop.auto", &json!({"selection": true})).unwrap();
    assert_eq!(s.undo.len(), undo + 1, "one undo step for the whole selection");
    assert!([1, 2, 3].iter().all(|id| exposure(&s, *id) != 0.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!([1, 2, 3].iter().all(|id| exposure(&s, *id) == 0.0), "one undo takes back every photo");
    s.execute("edit.redo", &json!({})).unwrap();
    assert!([1, 2, 3].iter().all(|id| exposure(&s, *id) != 0.0));
    // each photo's history has the step
    let h = &s.catalog.photo(PhotoId(2)).unwrap().history;
    assert_eq!(h.last().map(|x| x.label.as_str()), Some("Auto Settings"));
}

#[test]
fn unreadable_photo_is_skipped_with_reason() {
    let mut s = open();
    let r = s.execute("develop.auto", &json!({"ids": [1, 4, 3]})).unwrap();
    assert_eq!(r["changed"], 2, "{r}");
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{r}");
    assert_eq!(skipped[0]["id"], 4);
    assert_eq!(skipped[0]["name"], "gone.jpg");
    let reason = skipped[0]["reason"].as_str().unwrap();
    assert!(reason.contains("could not be read") && reason.contains("no such file"), "{reason}");
    assert_eq!(exposure(&s, 4), 0.0, "an unreadable photo is left as it was");
    assert!(exposure(&s, 1) != 0.0 && exposure(&s, 3) != 0.0);
    // only unreadable photos: nothing changes, no undo step
    let undo = s.undo.len();
    let r = s.execute("develop.auto", &json!({"ids": [4]})).unwrap();
    assert_eq!((r["changed"].as_u64(), s.undo.len()), (Some(0), undo), "{r}");
}

/// In the background: a row in the activity stack while it runs, `batch.pump` finishes it, ✕ changes nothing.
#[test]
fn auto_in_the_background_has_an_activity_row_and_cancels() {
    let gate = Arc::new(AtomicBool::new(false));
    let mut s = session(gate.clone());
    let r = s.execute("develop.auto", &json!({"ids": [1, 2], "background": true})).unwrap();
    assert_eq!(r["started"], true, "{r}");
    let rows = s.activity.list();
    let row = rows.iter().find(|t| t.kind == "auto").expect("an activity row");
    assert_eq!((row.label.as_str(), row.total, row.cancellable), ("Auto Settings", 2, true));
    // a second batch waits for the first
    assert!(s.execute("mask.updateAi", &json!({"ids": [1]})).unwrap_err().to_string().contains("still running"));
    assert_eq!(s.execute("batch.pump", &json!({})).unwrap()["active"], true);
    s.activity.cancel(row.id).unwrap();
    gate.store(true, Ordering::SeqCst);
    let done = pump_until_done(&mut s);
    assert_eq!((done["cancelled"].as_bool(), done["changed"].as_u64()), (Some(true), Some(0)), "{done}");
    assert_eq!((exposure(&s, 1), exposure(&s, 2)), (0.0, 0.0), "cancelled: nothing changes");
    assert!(s.activity.list().iter().all(|t| t.kind != "auto"), "the row goes when the job ends");
    // and run to the end it applies
    s.execute("develop.auto", &json!({"ids": [1, 2], "background": true})).unwrap();
    let done = pump_until_done(&mut s);
    assert_eq!(done["changed"], 2, "{done}");
    assert!(exposure(&s, 1) != 0.0);
}

fn pump_until_done(s: &mut Session) -> Value {
    let t = Instant::now();
    loop {
        let v = s.execute("batch.pump", &json!({})).unwrap();
        if let Some(d) = v.get("done") {
            return d.clone();
        }
        assert!(t.elapsed() < Duration::from_secs(20), "the batch never ended: {v}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn seg(v: f32) -> SegMask {
    SegMask::from_logits(4, &[v; 16])
}

fn prompt_seg(s: &Session, id: u64) -> Option<SegMask> {
    let d = s.develop_of(PhotoId(id)).unwrap();
    d.masks.iter().flat_map(|m| &m.components).find_map(|c| match &c.shape {
        MaskShape::Prompt { seg, .. } => seg.clone(),
        _ => None,
    })
}

/// Photo 1 with a Describe "subject" selection (its own segmentation) and a heuristic Subject mask, pasted with its
/// masks onto photos 2 and 3: they carry photo 1's segmentation until Update AI Masks.
fn pasted_masks(s: &mut Session) {
    s.execute("library.select", &json!({"ids": [1]})).unwrap();
    s.execute("mask.add", &json!({"kind": "prompt", "text": "subject", "seg": seg(1.0)})).unwrap();
    s.execute("mask.add", &json!({"kind": "subject"})).unwrap();
    s.execute("develop.copy", &json!({"groups": ["masks"]})).unwrap();
    s.execute("develop.paste", &json!({"ids": [2, 3]})).unwrap();
    assert_eq!(prompt_seg(s, 2), Some(seg(1.0)), "a paste carries the source photo's selection");
}

#[test]
fn update_ai_masks_recomputes_subject() {
    let mut s = open();
    pasted_masks(&mut s);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    s.batch_ai.fake_sam = Some(Arc::new(move |id: PhotoId, shape: &MaskShape| {
        let MaskShape::Prompt { text, .. } = shape else { return Err("not a Describe selection".into()) };
        log.lock().unwrap().push((id.0, text.clone()));
        Ok(Some(seg(id.0 as f32 * 2.0)))
    }));
    let undo = s.undo.len();
    let r = s.execute("mask.updateAi", &json!({"ids": [2, 3]})).unwrap();
    assert_eq!((r["kind"].as_str(), r["changed"].as_u64(), r["live"].as_u64()), (Some("aiMasks"), Some(2), Some(2)), "{r}");
    assert_eq!((r["kept"].clone(), r["skipped"].clone()), (json!([]), json!([])));
    assert_eq!(*asked.lock().unwrap(), vec![(2, "subject".to_string()), (3, "subject".to_string())], "each photo is segmented itself");
    assert_eq!((prompt_seg(&s, 2), prompt_seg(&s, 3)), (Some(seg(4.0)), Some(seg(6.0))));
    assert_eq!(prompt_seg(&s, 1), Some(seg(1.0)), "photos not named keep theirs");
    // the heuristic Subject mask is still there, as it was (it follows each photo when it renders)
    let d = s.develop_of(PhotoId(2)).unwrap();
    assert!(d.masks.iter().any(|m| m.components.iter().any(|c| c.shape == MaskShape::Subject)));
    assert_eq!(s.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((prompt_seg(&s, 2), prompt_seg(&s, 3)), (Some(seg(1.0)), Some(seg(1.0))));
}

#[test]
fn update_ai_masks_keeps_sam_masks_where_the_model_cannot_run() {
    // the app says SAM 3 can't run on this device: the selections are kept and each photo is reported
    let mut s = open();
    pasted_masks(&mut s);
    s.batch_ai.fake_sam = Some(Arc::new(|_: PhotoId, _: &MaskShape| panic!("the model is not asked")));
    let r = s.execute("mask.updateAi", &json!({"ids": [2, 3], "samUnavailable": "Needs a device with 8 GB of memory"})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["live"].as_u64()), (Some(0), Some(2)), "{r}");
    let kept = r["kept"].as_array().unwrap();
    assert_eq!(kept.iter().map(|k| k["id"].as_u64().unwrap()).collect::<Vec<_>>(), vec![2, 3]);
    assert!(kept[0]["reason"].as_str().unwrap().contains("8 GB"), "{r}");
    assert_eq!(prompt_seg(&s, 2), Some(seg(1.0)));
    // without the model at all (no stand-in, nothing installed): kept, with the reason
    s.batch_ai.fake_sam = None;
    let r = s.execute("mask.updateAi", &json!({"ids": [2]})).unwrap();
    assert_eq!(r["kept"].as_array().map(Vec::len), Some(1), "{r}");
    // a model that finds nothing on a photo keeps that selection; one that fails skips the photo, with the reason
    s.batch_ai.fake_sam =
        Some(Arc::new(|id: PhotoId, _: &MaskShape| if id.0 == 2 { Ok(None) } else { Err("the photo could not be rendered".into()) }));
    let r = s.execute("mask.updateAi", &json!({"ids": [2, 3]})).unwrap();
    assert_eq!(r["changed"], 0, "{r}");
    assert!(r["kept"][0]["reason"].as_str().unwrap().contains("nothing matching “subject”"), "{r}");
    assert!(r["skipped"][0]["reason"].as_str().unwrap().contains("could not be rendered"), "{r}");
}

/// Update AI Masks in the background: its own activity row; ✕ changes nothing, even when the model answers later.
#[test]
fn update_ai_masks_in_the_background_has_an_activity_row_and_cancels() {
    let mut s = open();
    pasted_masks(&mut s);
    s.batch_ai.stalled = Some(Vec::new());
    let r = s.execute("mask.updateAi", &json!({"ids": [2, 3], "background": true})).unwrap();
    assert_eq!((r["started"].as_bool(), r["kind"].as_str()), (Some(true), Some("aiMasks")), "{r}");
    let rows = s.activity.list();
    let row = rows.iter().find(|t| t.kind == "aiMasks").expect("an activity row");
    assert_eq!((row.label.as_str(), row.total, row.cancellable), ("Update AI Masks", 2, true));
    assert_eq!(s.execute("batch.pump", &json!({})).unwrap()["active"], true);
    let undo = s.undo.len();
    s.activity.cancel(row.id).unwrap();
    // the model answers after the ✕: thrown away
    for tx in s.batch_ai.stalled.take().unwrap() {
        let _ = tx.send(Ok(Some(seg(9.0))));
    }
    let done = pump_until_done(&mut s);
    assert_eq!((done["cancelled"].as_bool(), done["changed"].as_u64()), (Some(true), Some(0)), "{done}");
    assert_eq!((prompt_seg(&s, 2), prompt_seg(&s, 3)), (Some(seg(1.0)), Some(seg(1.0))), "cancelled: nothing changes");
    assert_eq!(s.undo.len(), undo);
    assert!(s.activity.list().iter().all(|t| t.kind != "aiMasks"), "the row goes when the job ends");
}

/// A waiting command (CLI, MCP) doesn't hang on a model that never answers: what's unanswered is skipped.
#[test]
fn update_ai_masks_waiting_gives_up_on_a_stalled_model() {
    let mut s = open();
    pasted_masks(&mut s);
    s.batch_ai.stalled = Some(Vec::new());
    s.batch_ai.wait = Some(Duration::from_millis(50));
    let t = Instant::now();
    let r = s.execute("mask.updateAi", &json!({"ids": [2, 3]})).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10));
    assert_eq!((r["changed"].as_u64(), r["cancelled"].as_bool()), (Some(0), Some(false)), "{r}");
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.iter().map(|k| k["id"].as_u64().unwrap()).collect::<Vec<_>>(), vec![2, 3], "{r}");
    assert!(skipped[0]["reason"].as_str().unwrap().contains("took too long"), "{r}");
    assert_eq!(prompt_seg(&s, 2), Some(seg(1.0)));
    assert!(!s.batch_ai.running());
}
