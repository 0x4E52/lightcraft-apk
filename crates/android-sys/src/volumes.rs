//! SD cards and USB drives: `StorageManager.getStorageVolumes()` turned into the volumes the
//! device scan looks for a `DCIM` folder in (APK-51).
//!
//! [`query_volumes`] (Android only) reads, for each volume, one local frame each: `getState()`,
//! `isPrimary()`, `isRemovable()`, `getUuid()`, `getDirectory()` (API 30) and
//! `getDescription(context)`, the label the system shows ("SanDisk SD card", "USB drive").
//! [`volumes`] is the pure mapper, tested on every target: it keeps the mounted removable
//! volumes that are not the primary shared storage (the phone's own `/storage/emulated/<user>`
//! is in Local already), whose directory is an absolute path of plain components, each volume
//! once (a volume listed twice, by UUID or by path, is one card), and labels a volume without a
//! description by its kind.
//!
//! With the photo permission (APK-22) a card's photos are read by path through FUSE, like the
//! shared storage's; files Android's media database doesn't classify as media are hidden from
//! the listing (APK-52).

use std::path::{Component, PathBuf};

/// `Environment.MEDIA_MOUNTED`.
pub const MOUNTED: &str = "mounted";
/// `Environment.MEDIA_MOUNTED_READ_ONLY` (a card with its lock switch on).
pub const MOUNTED_READ_ONLY: &str = "mounted_ro";

/// At most this many volumes are read (a broken service can't grow the list forever).
pub const MAX_VOLUMES: usize = 64;

/// The label of a removable volume the system gives no description for.
pub const FALLBACK_LABEL: &str = "Removable storage";

/// One `StorageVolume`, as JNI read it; a call that failed or answered null is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VolumeRow {
    pub state: Option<String>,
    pub primary: bool,
    pub removable: bool,
    pub uuid: Option<String>,
    pub directory: Option<String>,
    pub description: Option<String>,
}

/// A card or drive to look for a `DCIM` folder in: its label and its mount point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    pub root: PathBuf,
}

/// An absolute path of plain components only (a row is another process's data).
fn plain_absolute(path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(path);
    let plain = p.is_absolute()
        && p.components().skip(1).all(|c| matches!(c, Component::Normal(_)))
        && p.components().nth(1).is_some()
        && !path.split('/').skip(1).any(|part| part.is_empty() || part == "." || part == "..");
    plain.then_some(p)
}

/// The mounted removable volumes among `rows`, in order, each once (see the module docs).
pub fn volumes(rows: impl IntoIterator<Item = VolumeRow>) -> Vec<Volume> {
    let mut out: Vec<Volume> = Vec::new();
    let mut seen_uuids: Vec<String> = Vec::new();
    for row in rows.into_iter().take(MAX_VOLUMES) {
        let mounted = matches!(row.state.as_deref(), Some(MOUNTED | MOUNTED_READ_ONLY));
        if !mounted || row.primary || !row.removable {
            continue;
        }
        let Some(root) = row.directory.as_deref().and_then(plain_absolute) else { continue };
        let uuid = row.uuid.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(str::to_ascii_uppercase);
        if uuid.as_ref().is_some_and(|u| seen_uuids.contains(u)) || out.iter().any(|v| v.root == root) {
            continue;
        }
        seen_uuids.extend(uuid);
        let name = row.description.as_deref().map(str::trim).filter(|d| !d.is_empty()).unwrap_or(FALLBACK_LABEL).to_owned();
        out.push(Volume { name, root });
    }
    out
}

/// The device's mounted removable volumes ([`volumes`] of `getStorageVolumes()`), on the calling
/// thread (attached through [`crate::jni_env::with_activity`]: meant for the device watch's
/// worker, never inside a `with_activity` closure).
#[cfg(target_os = "android")]
pub fn query_volumes(app: &android_activity::AndroidApp) -> Result<Vec<Volume>, crate::Error> {
    let rows = crate::jni_env::with_activity(app, query_rows)?;
    Ok(volumes(rows))
}

/// `getSystemService("storage").getStorageVolumes()`, each volume read in its own local frame.
#[cfg(target_os = "android")]
fn query_rows(env: &mut jni::Env<'_>, activity: &jni::objects::JObject<'_>) -> Result<Vec<VolumeRow>, crate::Error> {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{JValue, jni_sig, jni_str};
    env.with_local_frame(16, |env| -> Result<Vec<VolumeRow>, crate::Error> {
        let service = env.new_string("storage")?;
        let answer =
            env.call_method(activity, jni_str!("getSystemService"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"), &[JValue::Object(&service)]);
        let manager = caught(env, answer)?.l()?;
        if manager.is_null() {
            return Err(crate::Error::Jni("no storage service".to_owned()));
        }
        let answer = env.call_method(&manager, jni_str!("getStorageVolumes"), jni_sig!("()Ljava/util/List;"), &[]);
        let list = caught(env, answer)?.l()?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        let answer = env.call_method(&list, jni_str!("size"), jni_sig!("()I"), &[]);
        let len = usize::try_from(caught(env, answer)?.i()?).unwrap_or(0).min(MAX_VOLUMES);
        let mut rows = Vec::with_capacity(len);
        for i in 0..len {
            let index = i32::try_from(i).map_err(|_| crate::Error::Jni("too many volumes".to_owned()))?;
            // a volume that can't be read (its frame, `get`) is an empty row, never the whole list
            let row = env.with_local_frame(16, |env| -> Result<VolumeRow, crate::Error> {
                let answer = env.call_method(&list, jni_str!("get"), jni_sig!("(I)Ljava/lang/Object;"), &[JValue::Int(index)]);
                let Ok(volume) = caught(env, answer).and_then(|v| Ok(v.l()?)) else { return Ok(VolumeRow::default()) };
                if volume.is_null() {
                    return Ok(VolumeRow::default());
                }
                Ok(read_volume(env, &volume, activity))
            });
            rows.push(row.unwrap_or_else(|_| {
                let _ = env.exception_catch();
                VolumeRow::default()
            }));
        }
        Ok(rows)
    })
}

/// A `String` answer: `None` when the call threw, answered null or its text can't be read (the
/// exception, if any, cleared).
#[cfg(target_os = "android")]
fn text_of<'l>(env: &mut jni::Env<'l>, answer: jni::errors::Result<jni::objects::JValueOwned<'l>>) -> Option<String> {
    let obj = crate::dialog::caught(env, answer).and_then(|v| Ok(v.l()?)).ok()?;
    match crate::content::string_of(env, obj) {
        Ok(text) => text,
        Err(_) => {
            let _ = env.exception_catch();
            None
        }
    }
}

/// One `StorageVolume`'s answers; a call that throws, or whose text can't be read, counts as no
/// answer for that field (one odd volume doesn't hide the others).
#[cfg(target_os = "android")]
fn read_volume(env: &mut jni::Env<'_>, volume: &jni::objects::JObject<'_>, context: &jni::objects::JObject<'_>) -> VolumeRow {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{JValue, jni_sig, jni_str};
    let answer = env.call_method(volume, jni_str!("isPrimary"), jni_sig!("()Z"), &[]);
    let primary = caught(env, answer).and_then(|v| Ok(v.z()?)).unwrap_or(false);
    let answer = env.call_method(volume, jni_str!("isRemovable"), jni_sig!("()Z"), &[]);
    let removable = caught(env, answer).and_then(|v| Ok(v.z()?)).unwrap_or(false);
    let answer = env.call_method(volume, jni_str!("getState"), jni_sig!("()Ljava/lang/String;"), &[]);
    let state = text_of(env, answer);
    let answer = env.call_method(volume, jni_str!("getUuid"), jni_sig!("()Ljava/lang/String;"), &[]);
    let uuid = text_of(env, answer);
    let answer =
        env.call_method(volume, jni_str!("getDescription"), jni_sig!("(Landroid/content/Context;)Ljava/lang/String;"), &[JValue::Object(context)]);
    let description = text_of(env, answer);
    // API 30: null while the volume is not mounted
    let answer = env.call_method(volume, jni_str!("getDirectory"), jni_sig!("()Ljava/io/File;"), &[]);
    let directory = match caught(env, answer).and_then(|v| Ok(v.l()?)) {
        Ok(dir) if !dir.is_null() => {
            let answer = env.call_method(&dir, jni_str!("getAbsolutePath"), jni_sig!("()Ljava/lang/String;"), &[]);
            text_of(env, answer)
        }
        _ => None,
    };
    VolumeRow { state, primary, removable, uuid, directory, description }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uuid: &str, dir: &str, description: &str) -> VolumeRow {
        VolumeRow {
            state: Some(MOUNTED.into()),
            primary: false,
            removable: true,
            uuid: Some(uuid.into()),
            directory: Some(dir.into()),
            description: Some(description.into()),
        }
    }

    #[test]
    fn volume_rows_map_to_devices() {
        let rows = vec![
            row("1234-ABCD", "/storage/1234-ABCD", "SanDisk SD card"),
            row("5678-EF01", "/storage/5678-EF01", "  "),
            VolumeRow { state: Some(MOUNTED_READ_ONLY.into()), ..row("9ABC-DEF0", "/storage/9ABC-DEF0", "USB drive") },
            // not mounted (ejected, being checked, no directory yet): left out
            VolumeRow { state: Some("unmounted".into()), ..row("1111-2222", "/storage/1111-2222", "SD card") },
            VolumeRow { state: Some("checking".into()), ..row("3333-4444", "/storage/3333-4444", "SD card") },
            VolumeRow { directory: None, ..row("5555-6666", "/storage/5555-6666", "SD card") },
            VolumeRow { state: None, ..row("7777-8888", "/storage/7777-8888", "SD card") },
            // a directory that is not a plain absolute path: never followed
            row("AAAA-0001", "storage/AAAA-0001", "SD card"),
            row("AAAA-0002", "/storage/../data", "SD card"),
            row("AAAA-0003", "/storage//AAAA-0003", "SD card"),
            row("AAAA-0004", "/", "SD card"),
        ];
        let v = volumes(rows);
        assert_eq!(
            v,
            [
                Volume { name: "SanDisk SD card".into(), root: "/storage/1234-ABCD".into() },
                Volume { name: FALLBACK_LABEL.into(), root: "/storage/5678-EF01".into() },
                Volume { name: "USB drive".into(), root: "/storage/9ABC-DEF0".into() },
            ]
        );
    }

    #[test]
    fn same_volume_listed_once() {
        let v = volumes(vec![
            row("1234-ABCD", "/storage/1234-ABCD", "SanDisk SD card"),
            // the same card again, by UUID (any case) or by path
            row("1234-abcd", "/mnt/media_rw/1234-ABCD", "SanDisk SD card"),
            VolumeRow { uuid: None, ..row("", "/storage/1234-ABCD", "SD card") },
            // two volumes without a UUID are told apart by path
            VolumeRow { uuid: None, ..row("", "/storage/usb1", "USB drive") },
            VolumeRow { uuid: Some(" ".into()), ..row("", "/storage/usb2", "USB drive") },
        ]);
        let roots: Vec<_> = v.iter().map(|v| v.root.to_string_lossy().into_owned()).collect();
        assert_eq!(roots, ["/storage/1234-ABCD", "/storage/usb1", "/storage/usb2"]);
        // a broken service can't grow the list forever
        let many = (0..1000).map(|i| row(&format!("U{i}"), &format!("/storage/U{i}"), "USB drive"));
        assert_eq!(volumes(many).len(), MAX_VOLUMES);
    }

    #[test]
    fn non_removable_primary_is_not_a_device() {
        // the phone's own shared storage is in Local already (DCIM, Pictures, Download)
        let primary = VolumeRow { primary: true, removable: false, uuid: None, ..row("", "/storage/emulated/0", "Internal shared storage") };
        // an adopted (internal) card, or a primary that is removable on an old device: not a card to import from
        let adopted = VolumeRow { removable: false, ..row("1234-ABCD", "/storage/1234-ABCD", "SD card") };
        let removable_primary = VolumeRow { primary: true, ..row("", "/storage/sdcard0", "SD card") };
        assert!(volumes(vec![primary, adopted, removable_primary]).is_empty());
    }
}
