//! The device's photo library as folders: a `MediaStore.Images` query turned into paths and
//! grouped by directory (APK-23).
//!
//! On API 30+ an app holding the photo permission (APK-22) opens shared media through ordinary
//! paths (`/storage/emulated/<user>/DCIM/…`, served by FUSE), so the import reads the files by
//! **path** and the catalog and every reader stay as they are. The query only says which folders
//! hold photos: [`query_buckets`] reads `EXTERNAL_CONTENT_URI` for the columns of [`PROJECTION`]
//! (their indexes read once, each row inside `env.with_local_frame(16, …)`), [`row_to_path`]
//! builds a file's path from `_data`, or from the volume root + `RELATIVE_PATH` +
//! `DISPLAY_NAME`, and [`buckets`] groups the paths into folders, which the import dialog scans
//! as it scans a folder the user picked.
//!
//! A row is data from another process: a path that leaves the volume (`..`, a relative or
//! foreign `_data`, a name with a `/`) is dropped, never followed. Everything but
//! [`query_buckets`] is pure and tested on every target. Only the primary shared volume is read
//! (no SD cards yet), and raws MediaStore doesn't classify as images arrive through sharing
//! (APK-25).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// `MediaColumns.DATA`: the file's absolute path (deprecated since API 29, still filled).
pub const DATA: &str = "_data";
/// `MediaColumns.RELATIVE_PATH`: the folder inside the volume, e.g. `DCIM/Camera/`.
pub const RELATIVE_PATH: &str = "relative_path";
/// `MediaColumns.DISPLAY_NAME`: the file name.
pub const DISPLAY_NAME: &str = "_display_name";
/// `MediaColumns.BUCKET_DISPLAY_NAME`: the folder's name as the Gallery shows it.
pub const BUCKET_DISPLAY_NAME: &str = "bucket_display_name";
/// `MediaColumns.MIME_TYPE`.
pub const MIME_TYPE: &str = "mime_type";
/// `MediaColumns.SIZE`, in bytes.
pub const SIZE: &str = "_size";

/// The columns the query asks for, in this order.
pub const PROJECTION: [&str; 6] = [DATA, RELATIVE_PATH, DISPLAY_NAME, BUCKET_DISPLAY_NAME, MIME_TYPE, SIZE];

/// At most this many rows are read (a hostile or broken provider can't grow the list forever).
pub const MAX_ROWS: usize = 2_000_000;

/// One cursor row; a column that is null (or missing on this device) is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub data: Option<String>,
    pub relative_path: Option<String>,
    pub display_name: Option<String>,
    pub bucket: Option<String>,
    pub mime: Option<String>,
    pub size: Option<i64>,
}

/// A folder of the photo library: its name (the Gallery's), its path, and its photos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub name: String,
    pub dir: PathBuf,
    pub count: usize,
    pub bytes: u64,
}

/// `path` is absolute, has only plain components (no `.`, `..`), and lies strictly inside `volume`.
fn inside(path: &Path, volume: &Path) -> bool {
    path.is_absolute()
        && path.strip_prefix(volume).is_ok_and(|rest| rest.components().next().is_some())
        && path.components().skip(1).all(|c| matches!(c, Component::Normal(_)))
        // `components()` drops `.` and repeated `/` silently: compare the text as well
        && !path.to_string_lossy().split('/').skip(1).any(|part| part.is_empty() || part == "." || part == "..")
}

/// A relative path of plain components only (`DCIM/Camera/`; a trailing `/` is fine).
fn plain_relative(rel: &str) -> Option<PathBuf> {
    let rel = rel.strip_suffix('/').unwrap_or(rel);
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    rel.split('/').all(|part| !part.is_empty() && part != "." && part != "..").then(|| PathBuf::from(rel))
}

/// The file a row names, inside `volume` (the primary shared volume, `/storage/emulated/<user>`):
/// `_data` when it is a path inside the volume, else the volume + `RELATIVE_PATH` +
/// `DISPLAY_NAME`. `None` when neither gives a path inside the volume.
pub fn row_to_path(row: &Row, volume: &Path) -> Option<PathBuf> {
    if let Some(data) = row.data.as_deref().filter(|d| !d.is_empty()) {
        let path = PathBuf::from(data);
        if inside(&path, volume) {
            return Some(path);
        }
    }
    let rel = plain_relative(row.relative_path.as_deref()?)?;
    let name = row.display_name.as_deref().filter(|n| !n.is_empty() && *n != "." && *n != ".." && !n.contains('/'))?;
    let path = volume.join(rel).join(name);
    inside(&path, volume).then_some(path)
}

/// The folders that hold the rows' photos, by name (then path). A row that is not an image or
/// names no file inside `volume` is left out; a folder's name is the Gallery's bucket name, or
/// the folder's own name.
pub fn buckets(rows: impl IntoIterator<Item = Row>, volume: &Path) -> Vec<Bucket> {
    let mut by_dir: BTreeMap<PathBuf, Bucket> = BTreeMap::new();
    for row in rows.into_iter().take(MAX_ROWS) {
        if row.mime.as_deref().is_some_and(|m| !m.starts_with("image/")) {
            continue;
        }
        let Some(path) = row_to_path(&row, volume) else { continue };
        let Some(dir) = path.parent() else { continue };
        let bucket = by_dir.entry(dir.to_path_buf()).or_insert_with(|| Bucket {
            name: row
                .bucket
                .clone()
                .filter(|b| !b.trim().is_empty())
                .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default(),
            dir: dir.to_path_buf(),
            count: 0,
            bytes: 0,
        });
        bucket.count = bucket.count.saturating_add(1);
        bucket.bytes = bucket.bytes.saturating_add(row.size.and_then(|s| u64::try_from(s).ok()).unwrap_or(0));
    }
    let mut out: Vec<Bucket> = by_dir.into_values().collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.dir.cmp(&b.dir)));
    out
}

/// The photo library's folders: `ContentResolver.query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
/// PROJECTION, …)` on the calling thread, which attaches to the JVM for the call. Meant for a
/// worker (the query reads the whole library); never call it inside a `with_activity` closure.
/// Without the photo permission the provider answers only the app's own photos (none).
#[cfg(target_os = "android")]
pub fn query_buckets(app: &android_activity::AndroidApp) -> Result<Vec<Bucket>, crate::Error> {
    let (volume, rows) = crate::jni_env::with_activity(app, |env, activity| {
        let volume = primary_volume(env)?;
        let rows = query_rows(env, activity)?;
        Ok((volume, rows))
    })?;
    Ok(buckets(rows, &volume))
}

/// `Environment.getExternalStorageDirectory().getAbsolutePath()`: `/storage/emulated/<user>`.
#[cfg(target_os = "android")]
fn primary_volume(env: &mut jni::Env<'_>) -> Result<PathBuf, crate::Error> {
    use crate::dialog::caught;
    use jni::{jni_sig, jni_str};
    env.with_local_frame(16, |env| -> Result<PathBuf, crate::Error> {
        let answer =
            env.call_static_method(jni_str!("android/os/Environment"), jni_str!("getExternalStorageDirectory"), jni_sig!("()Ljava/io/File;"), &[]);
        let dir = caught(env, answer)?.l()?;
        let answer = env.call_method(&dir, jni_str!("getAbsolutePath"), jni_sig!("()Ljava/lang/String;"), &[]);
        let path = caught(env, answer)?.l()?;
        let path = string_of(env, path)?.ok_or_else(|| crate::Error::Jni("no shared storage volume".to_owned()))?;
        Ok(PathBuf::from(path))
    })
}

#[cfg(target_os = "android")]
use crate::content::string_of;

/// Every row of the images query, the cursor closed afterwards whatever happened.
#[cfg(target_os = "android")]
fn query_rows(env: &mut jni::Env<'_>, activity: &jni::objects::JObject<'_>) -> Result<Vec<Row>, crate::Error> {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{JValue, jni_sig, jni_str};
    let answer = env.call_method(activity, jni_str!("getContentResolver"), jni_sig!("()Landroid/content/ContentResolver;"), &[]);
    let resolver = caught(env, answer)?.l()?;
    let answer =
        env.get_static_field(jni_str!("android/provider/MediaStore$Images$Media"), jni_str!("EXTERNAL_CONTENT_URI"), jni_sig!("Landroid/net/Uri;"));
    let uri = caught(env, answer)?.l()?;
    let len = i32::try_from(PROJECTION.len()).map_err(|_| crate::Error::Jni("projection too long".to_owned()))?;
    let projection = env.new_object_array(len, jni_str!("java/lang/String"), jni::objects::JObject::null())?;
    for (i, column) in PROJECTION.iter().enumerate() {
        let name = env.new_string(column)?;
        projection.set_element(env, i, &name)?;
    }
    let null = jni::objects::JObject::null();
    let answer = env.call_method(
        &resolver,
        jni_str!("query"),
        jni_sig!("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"),
        &[JValue::Object(&uri), JValue::Object(&projection), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)],
    );
    let cursor = caught(env, answer)?.l()?;
    if cursor.is_null() {
        // the provider is gone or refused: no photos rather than an error
        return Ok(Vec::new());
    }
    let rows = read_cursor(env, &cursor);
    let answer = env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[]);
    // (a failed close is cleared and doesn't hide the rows: the cursor is gone either way)
    let _closed = caught(env, answer);
    rows
}

/// The cursor's rows: the column indexes read once, then one local frame per row.
#[cfg(target_os = "android")]
fn read_cursor(env: &mut jni::Env<'_>, cursor: &jni::objects::JObject<'_>) -> Result<Vec<Row>, crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    let mut index = [-1i32; PROJECTION.len()];
    for (slot, column) in index.iter_mut().zip(PROJECTION) {
        *slot = env.with_local_frame(16, |env| -> Result<i32, crate::Error> {
            let name = env.new_string(column)?;
            let answer = env.call_method(cursor, jni_str!("getColumnIndex"), jni_sig!("(Ljava/lang/String;)I"), &[JValue::Object(&name)]);
            Ok(caught(env, answer)?.i()?)
        })?;
    }
    let [data, relative_path, display_name, bucket, mime, size] = index;
    let mut rows = Vec::new();
    while rows.len() < MAX_ROWS {
        let row = env.with_local_frame(16, |env| -> Result<Option<Row>, crate::Error> {
            let answer = env.call_method(cursor, jni_str!("moveToNext"), jni_sig!("()Z"), &[]);
            if !caught(env, answer)?.z()? {
                return Ok(None);
            }
            let mut text = |column: i32| -> Result<Option<String>, crate::Error> {
                if column < 0 {
                    return Ok(None);
                }
                let answer = env.call_method(cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::Int(column)]);
                let value = caught(env, answer)?.l()?;
                string_of(env, value)
            };
            let (data, relative_path, display_name, bucket, mime) =
                (text(data)?, text(relative_path)?, text(display_name)?, text(bucket)?, text(mime)?);
            let size = if size < 0 {
                None
            } else {
                let answer = env.call_method(cursor, jni_str!("getLong"), jni_sig!("(I)J"), &[JValue::Int(size)]);
                Some(caught(env, answer)?.j()?)
            };
            Ok(Some(Row { data, relative_path, display_name, bucket, mime, size }))
        })?;
        match row {
            Some(row) => rows.push(row),
            None => break,
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOLUME: &str = "/storage/emulated/0";

    fn row(data: Option<&str>, rel: Option<&str>, name: Option<&str>) -> Row {
        Row {
            data: data.map(str::to_owned),
            relative_path: rel.map(str::to_owned),
            display_name: name.map(str::to_owned),
            bucket: None,
            mime: Some("image/jpeg".to_owned()),
            size: Some(1000),
        }
    }

    #[test]
    fn row_to_path_prefers_data_column() {
        let r = row(Some("/storage/emulated/0/DCIM/Camera/IMG_1.jpg"), Some("Pictures/Other/"), Some("IMG_9.jpg"));
        assert_eq!(row_to_path(&r, Path::new(VOLUME)), Some(PathBuf::from("/storage/emulated/0/DCIM/Camera/IMG_1.jpg")));
        // an empty `_data` is no path: the relative route answers
        let r = row(Some(""), Some("DCIM/Camera/"), Some("IMG_2.jpg"));
        assert_eq!(row_to_path(&r, Path::new(VOLUME)), Some(PathBuf::from("/storage/emulated/0/DCIM/Camera/IMG_2.jpg")));
    }

    #[test]
    fn row_to_path_from_relative_path() {
        let r = row(None, Some("DCIM/Camera/"), Some("IMG_1.jpg"));
        assert_eq!(row_to_path(&r, Path::new(VOLUME)), Some(PathBuf::from("/storage/emulated/0/DCIM/Camera/IMG_1.jpg")));
        // without the trailing slash, and in a nested folder
        let r = row(None, Some("Pictures/Trips/2026"), Some("a b.HEIC"));
        assert_eq!(row_to_path(&r, Path::new(VOLUME)), Some(PathBuf::from("/storage/emulated/0/Pictures/Trips/2026/a b.HEIC")));
        // a missing column gives no path
        assert_eq!(row_to_path(&row(None, None, Some("IMG_1.jpg")), Path::new(VOLUME)), None);
        assert_eq!(row_to_path(&row(None, Some("DCIM/"), None), Path::new(VOLUME)), None);
    }

    #[test]
    fn rejects_path_outside_volume() {
        let v = Path::new(VOLUME);
        // `_data` elsewhere (another app's private files, another user, a relative path) is not
        // followed; without a usable relative path the row is dropped
        for data in [
            "/data/data/com.other/files/a.jpg",
            "/storage/emulated/10/DCIM/a.jpg",
            "/storage/emulated/0/../1/DCIM/a.jpg",
            "/storage/emulated/0/DCIM/./a.jpg",
            "/storage/emulated/0//a.jpg",
            "DCIM/a.jpg",
            "/storage/emulated/0",
            "/storage/emulated/0/",
        ] {
            assert_eq!(row_to_path(&row(Some(data), None, None), v), None, "{data}");
        }
        // a foreign `_data` falls back to the relative route, which stays inside
        let r = row(Some("/data/x.jpg"), Some("DCIM/"), Some("a.jpg"));
        assert_eq!(row_to_path(&r, v), Some(PathBuf::from("/storage/emulated/0/DCIM/a.jpg")));
        // a relative path or name that climbs out, or a name that is a path, is dropped
        for (rel, name) in [
            ("../../../data/data/x/", "a.jpg"),
            ("DCIM/../../", "a.jpg"),
            ("/DCIM/", "a.jpg"),
            ("DCIM//Camera/", "a.jpg"),
            ("./DCIM/", "a.jpg"),
            ("", "a.jpg"),
            ("DCIM/", "../a.jpg"),
            ("DCIM/", ".."),
            ("DCIM/", "sub/a.jpg"),
            ("DCIM/", ""),
        ] {
            assert_eq!(row_to_path(&row(None, Some(rel), Some(name)), v), None, "{rel} + {name}");
        }
    }

    #[test]
    fn media_store_rows_group_into_folders() {
        let v = Path::new(VOLUME);
        let mut camera = row(Some("/storage/emulated/0/DCIM/Camera/1.jpg"), None, None);
        camera.bucket = Some("Camera".to_owned());
        let mut camera2 = row(None, Some("DCIM/Camera/"), Some("2.jpg"));
        camera2.size = Some(-5);
        let shots = row(None, Some("Pictures/Screenshots/"), Some("s.png"));
        let mut video = row(None, Some("DCIM/Camera/"), Some("v.mp4"));
        video.mime = Some("video/mp4".to_owned());
        let outside = row(Some("/data/x.jpg"), None, None);
        let got = buckets([shots, camera, camera2, video, outside], v);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Bucket { name: "Camera".into(), dir: PathBuf::from("/storage/emulated/0/DCIM/Camera"), count: 2, bytes: 1000 });
        // no bucket name: the folder's own
        assert_eq!(got[1].name, "Screenshots");
        assert_eq!(got[1].count, 1);
        assert!(buckets(Vec::new(), v).is_empty());
    }

    #[test]
    fn media_store_projection_names_the_columns() {
        assert_eq!(PROJECTION, ["_data", "relative_path", "_display_name", "bucket_display_name", "mime_type", "_size"]);
    }
}
