//! Synchronize Folder commands (see [`crate::sync`]).

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, str_param};
use crate::sync::{SyncChoice, scan, synchronize};
use crate::{Result, Session};

fn path(p: &Value, c: &str) -> Result<String> {
    str_param(p, "path").filter(|d| !d.trim().is_empty()).map(str::to_string).ok_or_else(|| bad(c, "missing `path`"))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "folder.scanChanges",
            "Scan Folder for Changes",
            [],
            None,
            "{path, disk?: bool (a whole disk or share, or a folder holding disks)} — compare a folder of the library (and the folders inside it) with the disk; changes nothing → {path, new: [{path, name, format, …}] files the library doesn't have, duplicates: files whose content it already has, unreadable: [{path, error}], missing: [{id, path}] photos whose file is gone, moved: [{id, from, to}] photos whose file is elsewhere in the folder (same content), metadata: [{id, path}] photos whose XMP sidecar was saved after they came in or were last edited here and says something the library doesn't}",
            always,
            |s, p| {
                const C: &str = "folder.scanChanges";
                let path = path(p, C)?;
                let changes = scan(s, &path, bool_or(p, "disk", false)).map_err(|e| bad(C, e.to_string()))?;
                let v = serde_json::to_value(&changes).unwrap_or_default();
                s.folder_changes = Some(changes);
                Ok(v)
            }
        ),
        cmd!(
            query "library.scanHandoff",
            "Read Handoff Folder",
            [],
            None,
            "{dir} — what a handoff folder (export.handoff on another device) holds for this library; changes nothing → {dir, created, new: [{path, name}] originals the library doesn't have, newer: [{id, name, path, handoffEdited, libraryEdited}] photos whose handed-off settings are newer than the library's, older: [...] photos whose library settings are as new or newer (never read), same: photos it says nothing new about, damaged: [[file, why]] missing or changed originals and unreadable sidecars}. Photos are matched by content hash, whatever their name or folder",
            always,
            |s, p| {
                const C: &str = "library.scanHandoff";
                let dir = str_param(p, "dir").filter(|d| !d.trim().is_empty()).ok_or_else(|| bad(C, "missing `dir`"))?;
                let plan = crate::handoff::scan(s, dir).map_err(|e| bad(C, e.to_string()))?;
                Ok(handoff_plan_json(&plan))
            }
        ),
        cmd!(
            "library.receiveHandoff",
            "Receive Handoff",
            [],
            None,
            "{dir, importNew?: true, read?: [photo ids listed as newer by library.scanHandoff] (default none: reading is never the default)} — read the folder again, read the chosen newer settings (the handoff's sidecar wins; a photo whose library settings are as new or newer, or that changed since, is refused) and import the new originals (copied into the library with their settings); one undo step → {imported, read, refused: [[id, why]], failed: [[file, why]]}",
            always,
            receive_handoff
        ),
        cmd!(
            "folder.synchronize",
            "Synchronize Folder",
            [],
            None,
            "{path, disk?: bool, scanned?: bool, importNew?: true, relinkMoved?: true, removeMissing?: false, readMetadata?: false} — bring a folder of the library up to date with the disk (see folder.scanChanges): import its new files in place, relink photos whose file was renamed or moved within it, move photos whose file is gone to Recently Deleted, read XMP sidecars saved by other apps (the sidecar wins); acts on the last folder.scanChanges of the folder while it is current (the folder's photos unchanged), else scans again — or, with `scanned: true`, refuses; one undo step, no file is touched → {imported, relinked, removed, read, failed: [[path, error]]}",
            always,
            sync
        ),
    ]
}

fn sync(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "folder.synchronize";
    let path = path(p, C)?;
    let d = SyncChoice::default();
    let choice = SyncChoice {
        import_new: bool_or(p, "importNew", d.import_new),
        relink_moved: bool_or(p, "relinkMoved", d.relink_moved),
        remove_missing: bool_or(p, "removeMissing", d.remove_missing),
        read_metadata: bool_or(p, "readMetadata", d.read_metadata),
    };
    let changes = match s.take_folder_changes(&path) {
        Some(c) => c,
        // the person agreed to what a scan showed: never act on something else
        None if bool_or(p, "scanned", false) => return Err(bad(C, format!("{path} changed since it was scanned: scan again"))),
        None => scan(s, &path, bool_or(p, "disk", false)).map_err(|e| bad(C, e.to_string()))?,
    };
    let r = synchronize(s, changes, choice)?;
    Ok(json!({"imported": r.imported, "relinked": r.relinked, "removed": r.removed, "read": r.read, "failed": r.failed}))
}

/// A handoff plan as the commands answer it (camelCase keys).
pub(crate) fn handoff_plan_json(plan: &crate::handoff::HandoffPlan) -> Value {
    let m = |v: &[crate::handoff::HandoffMatch]| -> Vec<Value> {
        v.iter()
            .map(|m| json!({"id": m.id, "name": m.name, "path": m.path, "handoffEdited": m.handoff_edited, "libraryEdited": m.library_edited}))
            .collect()
    };
    json!({
        "dir": plan.dir,
        "created": plan.created,
        "new": plan.new,
        "newer": m(&plan.newer),
        "older": m(&plan.older),
        "same": plan.same,
        "damaged": plan.damaged,
    })
}

fn receive_handoff(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.receiveHandoff";
    let dir = str_param(p, "dir").filter(|d| !d.trim().is_empty()).ok_or_else(|| bad(C, "missing `dir`"))?.to_string();
    let read = match p.get("read") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_u64().map(lightcraft_catalog::PhotoId)).collect::<Option<_>>().ok_or_else(|| bad(C, "`read` must be photo ids"))?
        }
        Some(_) => return Err(bad(C, "`read` must be an array of photo ids")),
    };
    let plan = crate::handoff::scan(s, &dir).map_err(|e| bad(C, e.to_string()))?;
    let choice = crate::handoff::HandoffChoice { import_new: bool_or(p, "importNew", true), read };
    let r = crate::handoff::apply(s, plan, &choice)?;
    Ok(json!({"imported": r.imported, "read": r.read, "refused": r.refused, "failed": r.failed}))
}
