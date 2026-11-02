//! A camera's files over MTP (APK-59): `android.mtp.MtpDevice` on a camera the user allowed
//! LightCraft to use ([`crate::usb`]).
//!
//! MTP reads go to the camera itself, not through Android's media database, so every raw type
//! arrives (the photo permission hides the types the database doesn't know, APK-52).
//!
//! [`with_mtp`] (Android only) opens the device (`UsbManager.openDevice`, `MtpDevice.open`) on
//! the calling worker and closes it when the job is done; [`Mtp::objects`] reads
//! `getStorageIds`, `getObjectHandles` and one `getObjectInfo` per object (name, size, dates),
//! each in its own local frame; [`Mtp::import`] copies one object with `importFile(handle, path)`.
//! The pure parts, tested on every target: [`review_items`] turns the object rows into the import
//! review's items (folders, unsafe names and other files left out, each object once, by date), and
//! [`skip_known`] sets aside what the library has already, by name + size + day, before anything
//! is copied.

use std::collections::HashSet;

/// `MtpConstants.FORMAT_ASSOCIATION`: a folder.
pub const FORMAT_ASSOCIATION: i32 = 0x3001;

/// At most this many objects are read from one camera (a broken device can't grow the list forever).
pub const MAX_OBJECTS: usize = 100_000;

/// Objects in a row whose info can't be read before the camera counts as gone (unplugged mid-listing).
#[cfg(target_os = "android")]
const FAILED_IN_A_ROW: usize = 3;
/// A file name longer than this (bytes) is left out.
pub const MAX_NAME_BYTES: usize = 255;

/// One `MtpObjectInfo`, as JNI read it (`None`: the call threw or answered null).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectRow {
    pub handle: i32,
    pub name: Option<String>,
    pub format: i32,
    /// `getCompressedSizeLong()`, bytes.
    pub size: i64,
    /// `getDateCreated()` / `getDateModified()`, milliseconds since the epoch (0: unknown).
    pub created_ms: i64,
    pub modified_ms: i64,
}

/// A photo on the camera, as the import review lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraObject {
    /// The MTP object handle `importFile` copies.
    pub handle: i32,
    /// A plain file name (no separator, no control characters, not hidden).
    pub name: String,
    pub size: u64,
    /// When the camera wrote it, seconds since the epoch.
    pub date: Option<i64>,
}

/// A photo the library has already, as the duplicate check compares it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KnownFile {
    /// The file name, lower case.
    pub name: String,
    pub size: u64,
    /// The day it was taken (days since the epoch), when known.
    pub day: Option<i64>,
}

/// `raw` without control characters and trimmed; `None` when it names a folder (a separator in
/// it), is empty, hidden (`.`, `..` included) or longer than [`MAX_NAME_BYTES`].
pub fn plain_name(raw: &str) -> Option<String> {
    if raw.contains(['/', '\\']) {
        return None;
    }
    let clean: String = raw.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    (!clean.is_empty() && !clean.starts_with('.') && clean.len() <= MAX_NAME_BYTES).then(|| clean.to_owned())
}

/// Seconds since the epoch from an MTP date in milliseconds (0 or negative: unknown).
fn secs(ms: i64) -> Option<i64> {
    (ms > 0).then_some(ms / 1000)
}

/// The import review's items: every object that is a file (not a folder) with a [`plain_name`]
/// that `keep` accepts (the importer's extensions) and a size, each handle once, oldest first
/// (then by name), at most [`MAX_OBJECTS`].
pub fn review_items(rows: impl IntoIterator<Item = ObjectRow>, keep: impl Fn(&str) -> bool) -> Vec<CameraObject> {
    let mut seen = HashSet::new();
    let mut out: Vec<CameraObject> = Vec::new();
    for row in rows.into_iter().take(MAX_OBJECTS) {
        if row.format == FORMAT_ASSOCIATION || !seen.insert(row.handle) {
            continue;
        }
        let Some(name) = row.name.as_deref().and_then(plain_name) else { continue };
        let Ok(size) = u64::try_from(row.size) else { continue };
        if size == 0 || !keep(&name) {
            continue;
        }
        let date = secs(row.created_ms).or_else(|| secs(row.modified_ms));
        out.push(CameraObject { handle: row.handle, name, size, date });
    }
    out.sort_by(|a, b| a.date.unwrap_or(i64::MAX).cmp(&b.date.unwrap_or(i64::MAX)).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Days since the epoch of a time in seconds.
pub fn day_of(secs: i64) -> i64 {
    secs.div_euclid(86_400)
}

/// `objects` split into those the library doesn't have and those it has (`known`): same name (any
/// case) and size, and taken within a day of each other when both dates are known (the camera's
/// clock and the photo's capture time may be in different zones).
pub fn skip_known(objects: Vec<CameraObject>, known: &[KnownFile]) -> (Vec<CameraObject>, Vec<CameraObject>) {
    let mut by_key: std::collections::HashMap<(String, u64), Vec<Option<i64>>> = std::collections::HashMap::new();
    for k in known {
        by_key.entry((k.name.to_lowercase(), k.size)).or_default().push(k.day);
    }
    objects.into_iter().partition(|o| {
        let days = by_key.get(&(o.name.to_lowercase(), o.size));
        let have = days.is_some_and(|days| {
            days.iter().any(|day| match (day, o.date.map(day_of)) {
                (Some(a), Some(b)) => (a - b).abs() <= 1,
                _ => true,
            })
        });
        !have
    })
}

/// An open camera on the calling worker (see [`with_mtp`]).
#[cfg(target_os = "android")]
pub struct Mtp<'a, 'l> {
    env: &'a mut jni::Env<'l>,
    device: &'a jni::objects::JObject<'l>,
}

/// Opens the camera named `device` (`UsbManager.openDevice`, then `MtpDevice.open`) on the calling
/// thread (attached through [`crate::jni_env::with_activity`]: meant for a worker), runs `f` with
/// it and closes it. A camera that is gone, or that LightCraft may not use, is an error.
#[cfg(target_os = "android")]
pub fn with_mtp<T>(app: &android_activity::AndroidApp, device: &str, f: impl FnOnce(&mut Mtp<'_, '_>) -> T) -> Result<T, crate::Error> {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{JValue, jni_sig, jni_str};
    crate::jni_env::with_activity(app, |env, activity| {
        env.with_local_frame(16, |env| -> Result<T, crate::Error> {
            let manager = crate::usb::usb_manager(env, activity)?;
            let Some(usb) = crate::usb::device_named(env, &manager, device)? else {
                return Err(crate::Error::Java { class: "java.io.FileNotFoundException".to_owned(), message: "the camera was unplugged".to_owned() });
            };
            let answer = env.call_method(
                &manager,
                jni_str!("openDevice"),
                jni_sig!("(Landroid/hardware/usb/UsbDevice;)Landroid/hardware/usb/UsbDeviceConnection;"),
                &[JValue::Object(&usb)],
            );
            let connection = caught(env, answer)?.l()?;
            if connection.is_null() {
                return Err(crate::Error::Java {
                    class: "java.lang.SecurityException".to_owned(),
                    message: "the camera could not be opened".to_owned(),
                });
            }
            let mtp = env.new_object(jni_str!("android/mtp/MtpDevice"), jni_sig!("(Landroid/hardware/usb/UsbDevice;)V"), &[JValue::Object(&usb)]);
            let mtp = match caught(env, mtp) {
                Ok(mtp) => mtp,
                Err(e) => {
                    // not left claimed until GC: a retry would find the camera busy
                    let answer = env.call_method(&connection, jni_str!("close"), jni_sig!("()V"), &[]);
                    let _ = caught(env, answer);
                    return Err(e);
                }
            };
            let answer =
                env.call_method(&mtp, jni_str!("open"), jni_sig!("(Landroid/hardware/usb/UsbDeviceConnection;)Z"), &[JValue::Object(&connection)]);
            let opened = crate::activity::answer_bool(env, answer);
            if !matches!(opened, Ok(true)) {
                let answer = env.call_method(&connection, jni_str!("close"), jni_sig!("()V"), &[]);
                let _ = caught(env, answer);
                opened?;
                return Err(crate::Error::Jni(
                    "the camera did not answer over MTP (is another app using it, or is it in mass storage mode?)".to_owned(),
                ));
            }
            let out = f(&mut Mtp { env, device: &mtp });
            // (closes the connection too)
            let answer = env.call_method(&mtp, jni_str!("close"), jni_sig!("()V"), &[]);
            let _ = caught(env, answer);
            Ok(out)
        })
    })
}

#[cfg(target_os = "android")]
fn int_array(env: &mut jni::Env<'_>, obj: jni::objects::JObject<'_>, max: usize) -> Result<Vec<i32>, crate::Error> {
    use jni::refs::Reference;
    if obj.is_null() {
        return Ok(Vec::new());
    }
    let array = env.cast_local::<jni::objects::JIntArray>(obj)?;
    let len = array.len(env)?.min(max);
    let mut out = vec![0i32; len];
    array.get_region(env, 0, &mut out)?;
    Ok(out)
}

#[cfg(target_os = "android")]
impl Mtp<'_, '_> {
    /// Every object's info on every storage of the camera, checking `cancel` between objects.
    pub fn objects(&mut self, cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<ObjectRow>, crate::Error> {
        use crate::dialog::caught;
        use jni::refs::Reference;
        use jni::{JValue, jni_sig, jni_str};
        let device = self.device;
        let env = &mut *self.env;
        let answer = env.call_method(device, jni_str!("getStorageIds"), jni_sig!("()[I"), &[]);
        let storages = caught(env, answer)?.l()?;
        let storages = int_array(env, storages, 16)?;
        let mut rows = Vec::new();
        // objects in a row whose info could not be read, since the last one read
        let mut in_a_row = 0usize;
        for storage in storages {
            // 0: all objects of the storage, not only its root's
            let handles = env.with_local_frame(4, |env| -> Result<Vec<i32>, crate::Error> {
                let answer = env.call_method(
                    device,
                    jni_str!("getObjectHandles"),
                    jni_sig!("(III)[I"),
                    &[JValue::Int(storage), JValue::Int(0), JValue::Int(0)],
                );
                let handles = caught(env, answer)?.l()?;
                int_array(env, handles, MAX_OBJECTS)
            })?;
            for handle in handles {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) || rows.len() >= MAX_OBJECTS {
                    return Ok(rows);
                }
                let row = env.with_local_frame(8, |env| -> Result<Option<ObjectRow>, crate::Error> {
                    let answer =
                        env.call_method(device, jni_str!("getObjectInfo"), jni_sig!("(I)Landroid/mtp/MtpObjectInfo;"), &[JValue::Int(handle)]);
                    let info = caught(env, answer)?.l()?;
                    if info.is_null() {
                        return Ok(None);
                    }
                    let answer = env.call_method(&info, jni_str!("getName"), jni_sig!("()Ljava/lang/String;"), &[]);
                    let name = caught(env, answer)?.l()?;
                    let name = crate::content::string_of(env, name)?;
                    let answer = env.call_method(&info, jni_str!("getFormat"), jni_sig!("()I"), &[]);
                    let format = caught(env, answer)?.i()?;
                    let answer = env.call_method(&info, jni_str!("getCompressedSizeLong"), jni_sig!("()J"), &[]);
                    let size = caught(env, answer)?.j()?;
                    let answer = env.call_method(&info, jni_str!("getDateCreated"), jni_sig!("()J"), &[]);
                    let created_ms = caught(env, answer)?.j()?;
                    let answer = env.call_method(&info, jni_str!("getDateModified"), jni_sig!("()J"), &[]);
                    let modified_ms = caught(env, answer)?.j()?;
                    Ok(Some(ObjectRow { handle, name, format, size, created_ms, modified_ms }))
                });
                match row {
                    Ok(Some(row)) => {
                        in_a_row = 0;
                        rows.push(row);
                    }
                    Ok(None) => in_a_row = 0,
                    // one object that can't be read is left out; an unplugged camera fails them all:
                    // the first, or a run of them, fails the listing rather than open a cut-short review
                    Err(e) => {
                        let _ = env.exception_catch();
                        in_a_row = in_a_row.saturating_add(1);
                        if rows.is_empty() || in_a_row >= FAILED_IN_A_ROW {
                            return Err(e);
                        }
                    }
                }
            }
        }
        Ok(rows)
    }

    /// Copies object `handle` to `to` (`importFile`), which must not exist yet.
    pub fn import(&mut self, handle: i32, to: &std::path::Path) -> Result<(), crate::Error> {
        use jni::{JValue, jni_sig, jni_str};
        let device = self.device;
        self.env.with_local_frame(4, |env| -> Result<(), crate::Error> {
            let path = env.new_string(to.to_string_lossy())?;
            let answer =
                env.call_method(device, jni_str!("importFile"), jni_sig!("(ILjava/lang/String;)Z"), &[JValue::Int(handle), JValue::Object(&path)]);
            if crate::activity::answer_bool(env, answer)? {
                Ok(())
            } else {
                Err(crate::Error::Jni("the camera did not send the file (unplugged, or asleep?)".to_owned()))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(handle: i32, name: &str, size: i64, created_ms: i64) -> ObjectRow {
        ObjectRow { handle, name: Some(name.into()), format: 0x3801, size, created_ms, modified_ms: 0 }
    }

    fn photo(name: &str) -> bool {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        ["cr3", "jpg", "raf", "arw"].contains(&ext.as_str())
    }

    #[test]
    fn object_rows_map_to_review_items() {
        let day = 86_400_000;
        let rows = vec![
            row(3, "IMG_0003.CR3", 30_000_000, 3 * day),
            row(1, "IMG_0001.CR3", 31_000_000, day),
            // a folder (DCIM, 100CANON): not an item
            ObjectRow { format: FORMAT_ASSOCIATION, ..row(10, "100CANON", 0, day) },
            // the camera's own files and unsafe names: left out
            row(11, "CANONMSC.CTG", 512, day),
            row(12, ".hidden.jpg", 100, day),
            row(13, "../../etc/passwd.jpg", 100, day),
            ObjectRow { name: None, ..row(14, "", 100, day) },
            row(15, "   ", 100, day),
            row(16, &format!("{}.jpg", "a".repeat(300)), 100, day),
            // empty or a negative size: never offered
            row(17, "EMPTY.JPG", 0, day),
            row(18, "BROKEN.JPG", -1, day),
            // the same handle twice (listed by two storages): once
            row(1, "IMG_0001.CR3", 31_000_000, day),
            // no creation date: the modification date; neither: last
            ObjectRow { created_ms: 0, modified_ms: 2 * day, ..row(2, "DSCF0002.RAF", 50_000_000, 0) },
            row(4, "DSC00004.ARW", 25_000_000, 0),
            // control characters are dropped from a name
            row(5, "IMG\u{0}_0005.JPG", 5_000_000, 3 * day),
        ];
        let items = review_items(rows, photo);
        let got: Vec<(i32, &str, u64, Option<i64>)> = items.iter().map(|o| (o.handle, o.name.as_str(), o.size, o.date)).collect();
        assert_eq!(
            got,
            [
                (1, "IMG_0001.CR3", 31_000_000, Some(86_400)),
                (2, "DSCF0002.RAF", 50_000_000, Some(2 * 86_400)),
                (3, "IMG_0003.CR3", 30_000_000, Some(3 * 86_400)),
                (5, "IMG_0005.JPG", 5_000_000, Some(3 * 86_400)),
                (4, "DSC00004.ARW", 25_000_000, None),
            ]
        );
        let many = (0..(MAX_OBJECTS as i32 + 10)).map(|i| row(i, &format!("IMG_{i}.JPG"), 1, 0));
        assert_eq!(review_items(many, photo).len(), MAX_OBJECTS);
    }

    #[test]
    fn duplicate_objects_skipped() {
        let obj = |handle: i32, name: &str, size: u64, date: Option<i64>| CameraObject { handle, name: name.into(), size, date };
        let day = |d: i64| d * 86_400 + 3600;
        let known = vec![
            KnownFile { name: "img_0001.cr3".into(), size: 31_000_000, day: Some(100) },
            KnownFile { name: "img_0002.cr3".into(), size: 32_000_000, day: None },
            KnownFile { name: "img_0003.cr3".into(), size: 33_000_000, day: Some(100) },
            KnownFile { name: "img_0004.cr3".into(), size: 34_000_000, day: Some(100) },
        ];
        let (new, have) = skip_known(
            vec![
                // same name (other case), size and day: in the library
                obj(1, "IMG_0001.CR3", 31_000_000, Some(day(100))),
                // no capture day in the library: name and size decide
                obj(2, "IMG_0002.CR3", 32_000_000, Some(day(5))),
                // a day apart (time zones): the same photo
                obj(3, "IMG_0003.CR3", 33_000_000, Some(day(101))),
                // the counter wrapped: same name, another day
                obj(4, "IMG_0004.CR3", 34_000_000, Some(day(300))),
                // same name, another size
                obj(5, "IMG_0001.CR3", 31_000_001, Some(day(100))),
                // the camera gives no date: name and size decide
                obj(6, "img_0001.cr3", 31_000_000, None),
                obj(7, "IMG_0007.CR3", 31_000_000, Some(day(100))),
            ],
            &known,
        );
        let handles = |v: &[CameraObject]| v.iter().map(|o| o.handle).collect::<Vec<_>>();
        assert_eq!(handles(&new), [4, 5, 7]);
        assert_eq!(handles(&have), [1, 2, 3, 6]);
        // an empty library skips nothing
        let (new, have) = skip_known(vec![obj(1, "A.JPG", 1, None)], &[]);
        assert_eq!((new.len(), have.len()), (1, 0));
        assert_eq!(day_of(-1), -1, "before the epoch rounds down");
    }
}
