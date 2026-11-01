//! Copying what another app shared (`content://` URIs) into LightCraft's own files (APK-25).
//!
//! The read grant that comes with a share ends with the Activity, so the host copies at once, on
//! a worker. Everything here but [`with_content`] is pure and tested on every target:
//!
//! - [`sanitize_display_name`] turns the sender's `DISPLAY_NAME` (data from another process)
//!   into a file name that can't leave the target folder: only the final component, no `/`, `\`,
//!   NUL or other control characters, no leading dots, at most [`MAX_NAME_BYTES`] cut on a char
//!   boundary (the extension kept), an extension from the MIME type when it has none, and `-1`,
//!   `-2`… when the name is taken.
//! - [`route_received`]: a photo or raw goes to the import, anything else (presets, track logs,
//!   keyword lists, models) to the Inbox.
//! - [`copy_to_file`] copies through [`copy_chunks`] into a hidden `.part` file next to the
//!   target, checking the size cap and the cancel flag between chunks, and renames it once
//!   complete; on any failure the partial file is removed.
//!
//! [`ContentReader`] is what the host's worker uses, so the job is tested with a fake;
//! [`with_content`] (Android only) gives the real one: `ContentResolver.query` for
//! `OpenableColumns.DISPLAY_NAME` / `SIZE`, `getType`, and `openInputStream` read through a
//! 1 MiB `byte[]`. Content URIs are never reopened through `/proc/self/fd`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Files over this size are refused (by the provider's `SIZE`, and again while copying).
pub const MAX_BYTES: u64 = 2 << 30;
/// One read from the Java stream.
pub const CHUNK: usize = 1 << 20;
/// The longest file name kept, in bytes.
pub const MAX_NAME_BYTES: usize = 120;
/// The name a nameless file gets (before its extension).
pub const FALLBACK_NAME: &str = "shared";
/// The longest extension recognised as one, in bytes.
const MAX_EXT_BYTES: usize = 16;

/// Where a received file goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// A photo or raw: copied, then imported.
    Import,
    /// Anything else: copied into the Inbox, where the in-app file chooser finds it.
    Inbox,
}

/// What the provider says about a URI; every field may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// `OpenableColumns.DISPLAY_NAME`.
    pub name: Option<String>,
    /// `OpenableColumns.SIZE`, in bytes.
    pub size: Option<i64>,
    /// `ContentResolver.getType(uri)`.
    pub mime: Option<String>,
}

/// Why a copy failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyError {
    /// The provider refused or failed (revoked grant, file gone): a Java error.
    Java(crate::Error),
    /// Reading the stream failed part way.
    Read(String),
    /// Writing the copy failed (a full disk).
    Write(String),
    /// Larger than the cap.
    TooBig,
    /// The user cancelled.
    Cancelled,
}

impl CopyError {
    /// Text the user can act on, for the file `name`.
    pub fn user_message(&self, name: &str) -> String {
        match self {
            CopyError::Java(e) => e.user_message(&format!("receive {name}")),
            CopyError::Read(e) => format!("Could not receive {name}: reading it stopped part way ({e})."),
            CopyError::Write(e) => format!("Could not receive {name}: saving it failed ({e}). Is the device full?"),
            CopyError::TooBig => format!("Could not receive {name}: it is larger than {} GB.", MAX_BYTES >> 30),
            CopyError::Cancelled => format!("Receiving {name} was cancelled."),
        }
    }
}

#[cfg(target_os = "android")]
impl From<jni::errors::Error> for CopyError {
    fn from(e: jni::errors::Error) -> Self {
        CopyError::Java(e.into())
    }
}

/// The extension a MIME type implies, for a name that has none.
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    let mime = mime.split(';').next().unwrap_or(mime).trim().to_ascii_lowercase();
    Some(match mime.as_str() {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => "jpg",
        "image/png" => "png",
        "image/tiff" => "tif",
        "image/webp" => "webp",
        "image/heic" => "heic",
        "image/heif" => "heif",
        "image/avif" => "avif",
        "image/jxl" => "jxl",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/x-adobe-dng" | "image/dng" => "dng",
        "image/x-canon-cr2" => "cr2",
        "image/x-canon-cr3" => "cr3",
        "image/x-nikon-nef" => "nef",
        "image/x-sony-arw" => "arw",
        "image/x-fuji-raf" => "raf",
        "image/x-olympus-orf" => "orf",
        "image/x-panasonic-rw2" => "rw2",
        "image/x-pentax-pef" => "pef",
        "image/x-samsung-srw" => "srw",
        "application/rdf+xml" => "xmp",
        "application/xml" | "text/xml" => "xml",
        "application/gpx+xml" => "gpx",
        "application/zip" => "zip",
        "text/plain" => "txt",
        _ => return None,
    })
}

/// The extension of `name` (lower case), when it has a short one after a dot that isn't first.
fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty() && ext.len() <= MAX_EXT_BYTES && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_lowercase())
}

/// The longest prefix of `s` of at most `max` bytes that ends on a char boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// `stem` + `suffix` + `.ext`, the stem cut so the whole fits [`MAX_NAME_BYTES`].
fn fit(stem: &str, suffix: &str, ext: Option<&str>) -> String {
    let tail = ext.map_or(0, |e| e.len() + 1) + suffix.len();
    let stem = cut(stem, MAX_NAME_BYTES.saturating_sub(tail));
    let stem = if stem.is_empty() { FALLBACK_NAME } else { stem };
    match ext {
        Some(ext) => format!("{stem}{suffix}.{ext}"),
        None => format!("{stem}{suffix}"),
    }
}

/// A safe file name for what the sender called `raw` (see the module docs); `taken` says whether
/// a candidate is in use (a file in the target folder, or a name given earlier in the same job).
pub fn sanitize_display_name(raw: Option<&str>, mime: Option<&str>, taken: impl Fn(&str) -> bool) -> String {
    let raw = raw.unwrap_or("");
    // only the final component, of either separator
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = last.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim().trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']);
    let (stem, ext) = match extension(clean) {
        Some(ext) => {
            // (the sender's spelling of the extension is kept)
            let at = clean.len().saturating_sub(ext.len());
            let stem = clean.get(..at.saturating_sub(1)).unwrap_or(clean);
            (stem.to_owned(), clean.get(at..).map(str::to_owned))
        }
        None => (clean.to_owned(), mime.and_then(extension_for_mime).map(str::to_owned)),
    };
    let first = fit(&stem, "", ext.as_deref());
    if !taken(&first) {
        return first;
    }
    for n in 1..=10_000u32 {
        let candidate = fit(&stem, &format!("-{n}"), ext.as_deref());
        if !taken(&candidate) {
            return candidate;
        }
    }
    // (10 000 files of one name in one folder: a name nobody gave)
    fit(&stem, &format!("-{}", std::process::id()), ext.as_deref())
}

/// Where a file named `name` (sanitized, so its extension is its type) goes: [`Route::Import`]
/// when its extension, or else its MIME type's, is one of `photos` (the importer's extensions).
pub fn route_received(name: &str, mime: Option<&str>, photos: &[&str]) -> Route {
    let ext = extension(name).or_else(|| mime.and_then(extension_for_mime).map(str::to_owned));
    match ext {
        Some(ext) if photos.iter().any(|p| p.eq_ignore_ascii_case(&ext)) => Route::Import,
        _ => Route::Inbox,
    }
}

/// Copies `reader` into `writer` until the end, checking `cancel` before each chunk and refusing
/// more than `cap` bytes; answers the bytes copied.
pub fn copy_chunks(reader: &mut dyn Read, writer: &mut dyn Write, cap: u64, cancel: &AtomicBool) -> Result<u64, CopyError> {
    let mut buf = vec![0u8; CHUNK];
    let mut total: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CopyError::Cancelled);
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(CopyError::Read(e.to_string())),
        };
        total = total.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        if total > cap {
            return Err(CopyError::TooBig);
        }
        let chunk = buf.get(..n).ok_or_else(|| CopyError::Read("the stream answered more than was asked".to_owned()))?;
        writer.write_all(chunk).map_err(|e| CopyError::Write(e.to_string()))?;
    }
}

/// The hidden file a copy to `target` is written to until it is complete.
pub fn part_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    target.with_file_name(format!(".{name}.part"))
}

/// Copies `reader` to `target` through [`part_path`] and renames it once complete; on any
/// failure the partial file is removed and `target` is not created.
pub fn copy_to_file(reader: &mut dyn Read, target: &Path, cap: u64, cancel: &AtomicBool) -> Result<u64, CopyError> {
    let part = part_path(target);
    let copied = (|| {
        let mut file = std::fs::File::create(&part).map_err(|e| CopyError::Write(e.to_string()))?;
        let n = copy_chunks(reader, &mut file, cap, cancel)?;
        file.sync_all().map_err(|e| CopyError::Write(e.to_string()))?;
        drop(file);
        std::fs::rename(&part, target).map_err(|e| CopyError::Write(e.to_string()))?;
        Ok(n)
    })();
    if copied.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    copied
}

/// The provider side of receiving files: what a URI is, and copying it.
pub trait ContentReader {
    /// The URI's name, size and type.
    fn meta(&mut self, uri: &str) -> Result<Meta, crate::Error>;
    /// Copies the URI's content to `target` (as [`copy_to_file`]).
    fn copy_to(&mut self, uri: &str, target: &Path, cap: u64, cancel: &AtomicBool) -> Result<u64, CopyError>;
}

/// A Java string as Rust text (`None` for null).
#[cfg(target_os = "android")]
pub(crate) fn string_of(env: &mut jni::Env<'_>, obj: jni::objects::JObject<'_>) -> Result<Option<String>, crate::Error> {
    use jni::refs::Reference;
    if obj.is_null() {
        return Ok(None);
    }
    let text = env.cast_local::<jni::objects::JString>(obj)?;
    Ok(Some(text.try_to_string(env)?))
}

/// Runs `f` with the device's [`ContentReader`] on the calling thread (attached to the JVM for
/// the call, so meant for a worker, and never inside a `with_activity` closure).
#[cfg(target_os = "android")]
pub fn with_content<T>(app: &android_activity::AndroidApp, f: impl FnOnce(&mut dyn ContentReader) -> T) -> Result<T, crate::Error> {
    use crate::dialog::caught;
    use jni::{jni_sig, jni_str};
    crate::jni_env::with_activity(app, |env, activity| {
        let answer = env.call_method(activity, jni_str!("getContentResolver"), jni_sig!("()Landroid/content/ContentResolver;"), &[]);
        let resolver = caught(env, answer)?.l()?;
        let mut reader = JniContent { env, resolver: &resolver };
        Ok(f(&mut reader))
    })
}

#[cfg(target_os = "android")]
struct JniContent<'a, 'l> {
    env: &'a mut jni::Env<'l>,
    resolver: &'a jni::objects::JObject<'l>,
}

#[cfg(target_os = "android")]
fn parse_uri<'l>(env: &mut jni::Env<'l>, uri: &str) -> Result<jni::objects::JObject<'l>, crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    let text = env.new_string(uri)?;
    let answer = env.call_static_method(
        jni_str!("android/net/Uri"),
        jni_str!("parse"),
        jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
        &[JValue::Object(&text)],
    );
    Ok(caught(env, answer)?.l()?)
}

#[cfg(target_os = "android")]
impl ContentReader for JniContent<'_, '_> {
    fn meta(&mut self, uri: &str) -> Result<Meta, crate::Error> {
        use crate::dialog::caught;
        use jni::refs::Reference;
        use jni::{JValue, jni_sig, jni_str};
        let resolver = self.resolver;
        self.env.with_local_frame(16, |env| -> Result<Meta, crate::Error> {
            let uri = parse_uri(env, uri)?;
            let answer = env.call_method(resolver, jni_str!("getType"), jni_sig!("(Landroid/net/Uri;)Ljava/lang/String;"), &[JValue::Object(&uri)]);
            let mime = caught(env, answer)?.l()?;
            let mime = string_of(env, mime)?;
            let null = jni::objects::JObject::null();
            let answer = env.call_method(
                resolver,
                jni_str!("query"),
                jni_sig!("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"),
                &[JValue::Object(&uri), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)],
            );
            // (a provider that can't be queried still can be read: name and size unknown)
            let cursor = match caught(env, answer) {
                Ok(c) => c.l()?,
                Err(_) => return Ok(Meta { mime, ..Meta::default() }),
            };
            if cursor.is_null() {
                return Ok(Meta { mime, ..Meta::default() });
            }
            let row = read_name_and_size(env, &cursor);
            let answer = env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[]);
            let _closed = caught(env, answer);
            let (name, size) = row?;
            Ok(Meta { name, size, mime })
        })
    }

    fn copy_to(&mut self, uri: &str, target: &Path, cap: u64, cancel: &AtomicBool) -> Result<u64, CopyError> {
        use crate::dialog::caught;
        use jni::{JValue, jni_sig, jni_str};
        let resolver = self.resolver;
        self.env.with_local_frame(16, |env| -> Result<u64, CopyError> {
            let uri = parse_uri(env, uri).map_err(CopyError::Java)?;
            let answer =
                env.call_method(resolver, jni_str!("openInputStream"), jni_sig!("(Landroid/net/Uri;)Ljava/io/InputStream;"), &[JValue::Object(&uri)]);
            let stream = caught(env, answer).and_then(|v| Ok(v.l()?)).map_err(CopyError::Java)?;
            if jni::refs::Reference::is_null(&stream) {
                return Err(CopyError::Java(crate::Error::Java { class: "java.io.FileNotFoundException".to_owned(), message: String::new() }));
            }
            let array = jni::objects::JByteArray::new(env, CHUNK).map_err(|e| CopyError::Java(e.into()))?;
            let copied = {
                let mut reader = JavaStream { env: &mut *env, stream: &stream, array: &array, buf: vec![0i8; CHUNK] };
                copy_to_file(&mut reader, target, cap, cancel)
            };
            let answer = env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[]);
            let _closed = caught(env, answer);
            copied
        })
    }
}

/// The first row's `DISPLAY_NAME` and `SIZE` (`OpenableColumns`).
#[cfg(target_os = "android")]
fn read_name_and_size(env: &mut jni::Env<'_>, cursor: &jni::objects::JObject<'_>) -> Result<(Option<String>, Option<i64>), crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    let answer = env.call_method(cursor, jni_str!("moveToFirst"), jni_sig!("()Z"), &[]);
    if !caught(env, answer)?.z()? {
        return Ok((None, None));
    }
    let index = |env: &mut jni::Env<'_>, column: &str| -> Result<i32, crate::Error> {
        let name = env.new_string(column)?;
        let answer = env.call_method(cursor, jni_str!("getColumnIndex"), jni_sig!("(Ljava/lang/String;)I"), &[JValue::Object(&name)]);
        Ok(caught(env, answer)?.i()?)
    };
    let (name_col, size_col) = (index(env, "_display_name")?, index(env, "_size")?);
    let name = if name_col < 0 {
        None
    } else {
        let answer = env.call_method(cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::Int(name_col)]);
        let value = caught(env, answer)?.l()?;
        string_of(env, value)?
    };
    let size = if size_col < 0 {
        None
    } else {
        let answer = env.call_method(cursor, jni_str!("isNull"), jni_sig!("(I)Z"), &[JValue::Int(size_col)]);
        if caught(env, answer)?.z()? {
            None
        } else {
            let answer = env.call_method(cursor, jni_str!("getLong"), jni_sig!("(I)J"), &[JValue::Int(size_col)]);
            Some(caught(env, answer)?.j()?)
        }
    };
    Ok((name, size))
}

/// A Java `InputStream` as a [`Read`]: `read(byte[], 0, n)` then the bytes copied out of the array.
#[cfg(target_os = "android")]
struct JavaStream<'a, 'l> {
    env: &'a mut jni::Env<'l>,
    stream: &'a jni::objects::JObject<'l>,
    array: &'a jni::objects::JByteArray<'l>,
    buf: Vec<i8>,
}

#[cfg(target_os = "android")]
impl Read for JavaStream<'_, '_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        use crate::dialog::caught;
        use jni::{JValue, jni_sig, jni_str};
        let want = out.len().min(self.buf.len());
        let len = i32::try_from(want).unwrap_or(i32::MAX);
        let (stream, array, buf) = (self.stream, self.array, &mut self.buf);
        let n = self
            .env
            .with_local_frame(4, |env| -> Result<usize, crate::Error> {
                let answer =
                    env.call_method(stream, jni_str!("read"), jni_sig!("([BII)I"), &[JValue::Object(array), JValue::Int(0), JValue::Int(len)]);
                let n = caught(env, answer)?.i()?;
                // -1 is the end of the stream
                let n = usize::try_from(n).unwrap_or(0).min(want);
                let region = buf.get_mut(..n).ok_or_else(|| crate::Error::Jni("short buffer".to_owned()))?;
                array.get_region(env, 0, region)?;
                Ok(n)
            })
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        for (o, b) in out.iter_mut().zip(buf.iter().take(n)) {
            *o = u8::from_ne_bytes(b.to_ne_bytes());
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const PHOTOS: &[&str] = &["jpg", "jpeg", "png", "dng", "cr3", "nef", "heic"];

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-android-content-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// Reads `ok` bytes, then fails.
    struct Failing {
        ok: usize,
    }

    impl Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.ok == 0 {
                return Err(std::io::Error::other("provider died"));
            }
            let n = buf.len().min(self.ok).min(1000);
            buf[..n].fill(7);
            self.ok -= n;
            Ok(n)
        }
    }

    /// Endless bytes, and sets `cancel` once `after` bytes were read.
    struct Endless<'a> {
        read: usize,
        after: usize,
        cancel: &'a AtomicBool,
    }

    impl Read for Endless<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(4096);
            self.read += n;
            if self.read >= self.after {
                self.cancel.store(true, Ordering::Relaxed);
            }
            Ok(n)
        }
    }

    #[test]
    fn display_name_cannot_escape_dir() {
        let none = |_: &str| false;
        for (raw, want) in [
            ("../../../data/data/x/databases/catalog.db", "catalog.db"),
            ("..\\..\\evil.xmp", "evil.xmp"),
            ("/abs/path/IMG_1.JPG", "IMG_1.JPG"),
            ("..", "shared"),
            (".hidden.xmp", "hidden.xmp"),
            ("...", "shared"),
            ("a\0b\n.jpg", "ab.jpg"),
            ("dir/", "shared"),
            ("   ", "shared"),
            ("name.", "name"),
        ] {
            let got = sanitize_display_name(Some(raw), None, none);
            assert_eq!(got, want, "{raw:?}");
            assert!(!got.contains(['/', '\\', '\0']) && !got.starts_with('.') && got != "..");
            assert_eq!(Path::new("/inbox").join(&got).parent(), Some(Path::new("/inbox")), "{raw:?}");
        }
        // no name: one from the MIME type; a name with an extension keeps its own
        assert_eq!(sanitize_display_name(None, Some("image/x-adobe-dng"), none), "shared.dng");
        assert_eq!(sanitize_display_name(Some("IMG_2041"), Some("image/jpeg"), none), "IMG_2041.jpg");
        assert_eq!(sanitize_display_name(Some("look.xmp"), Some("application/octet-stream"), none), "look.xmp");
        assert_eq!(sanitize_display_name(Some("track"), Some("application/gpx+xml; charset=utf-8"), none), "track.gpx");
        assert_eq!(sanitize_display_name(Some("blob"), Some("application/octet-stream"), none), "blob");
        // taken names get a number; a set of names given in the same job counts too
        let mut given = HashSet::new();
        for want in ["IMG_1.jpg", "IMG_1-1.jpg", "IMG_1-2.jpg"] {
            let name = sanitize_display_name(Some("IMG_1.jpg"), None, |n| given.contains(n));
            assert_eq!(name, want);
            given.insert(name);
        }
    }

    #[test]
    fn oversized_name_is_truncated_at_char_boundary() {
        // 3-byte characters, so 120 bytes falls inside one
        let long = format!("{}.jpeg", "日".repeat(100));
        let got = sanitize_display_name(Some(&long), None, |_| false);
        assert!(got.len() <= MAX_NAME_BYTES, "{} bytes", got.len());
        assert!(got.ends_with(".jpeg"), "the extension is kept: {got}");
        assert_eq!(got, format!("{}.jpeg", "日".repeat(38)));
        // with the number of a taken name, still within the cap
        let taken = got.clone();
        let next = sanitize_display_name(Some(&long), None, |n| n == taken);
        assert!(next.len() <= MAX_NAME_BYTES && next.ends_with("-1.jpeg"), "{next}");
        // a very long "extension" is part of the name, cut as well
        let ext = format!("a.{}", "x".repeat(300));
        assert!(sanitize_display_name(Some(&ext), None, |_| false).len() <= MAX_NAME_BYTES);
        assert_eq!(cut("aé", 2), "a");
    }

    #[test]
    fn non_photo_goes_to_inbox() {
        assert_eq!(route_received("IMG_1.JPG", Some("image/jpeg"), PHOTOS), Route::Import);
        assert_eq!(route_received("DSC_1.nef", Some("application/octet-stream"), PHOTOS), Route::Import);
        assert_eq!(route_received("x.cr3", None, PHOTOS), Route::Import);
        // no extension: the MIME type's
        assert_eq!(route_received("shot", Some("image/x-adobe-dng"), PHOTOS), Route::Import);
        for (name, mime) in [
            ("Look.xmp", Some("application/rdf+xml")),
            ("walk.gpx", Some("application/gpx+xml")),
            ("keywords.txt", Some("text/plain")),
            ("model.onnx", Some("application/octet-stream")),
            ("presets.zip", Some("application/zip")),
            ("blob", None),
            // an image type LightCraft doesn't import
            ("icon.svg", Some("image/svg+xml")),
        ] {
            assert_eq!(route_received(name, mime, PHOTOS), Route::Inbox, "{name}");
        }
    }

    #[test]
    fn copy_removes_partial_file_on_read_error() {
        let dir = temp("read-error");
        let target = dir.join("IMG_1.jpg");
        let cancel = AtomicBool::new(false);
        let err = copy_to_file(&mut Failing { ok: 5000 }, &target, MAX_BYTES, &cancel).unwrap_err();
        assert!(matches!(err, CopyError::Read(ref e) if e.contains("provider died")), "{err:?}");
        assert!(files(&dir).is_empty(), "no partial file: {:?}", files(&dir));
        assert!(err.user_message("IMG_1.jpg").contains("part way"));
        // a complete copy lands under its name, and only it
        let data: Vec<u8> = (0..3 * CHUNK + 17).map(|i| (i % 251) as u8).collect();
        let n = copy_to_file(&mut &data[..], &target, MAX_BYTES, &cancel).unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), data);
        assert_eq!(files(&dir), ["IMG_1.jpg"]);
        // a folder that can't be written is a write error, and leaves nothing
        let err = copy_to_file(&mut &data[..], &dir.join("missing").join("a.jpg"), MAX_BYTES, &cancel).unwrap_err();
        assert!(matches!(err, CopyError::Write(_)), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_stops_at_size_cap() {
        let dir = temp("cap");
        let target = dir.join("big.dng");
        let cancel = AtomicBool::new(false);
        let data = vec![1u8; 10_000];
        assert_eq!(copy_to_file(&mut &data[..], &target, 9_999, &cancel).unwrap_err(), CopyError::TooBig);
        assert!(files(&dir).is_empty());
        // exactly the cap is fine
        assert_eq!(copy_to_file(&mut &data[..], &target, 10_000, &cancel).unwrap(), 10_000);
        assert!(CopyError::TooBig.user_message("big.dng").contains("larger than 2 GB"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_removes_partial_file() {
        let dir = temp("cancel");
        let target = dir.join("huge.dng");
        let cancel = AtomicBool::new(false);
        let mut reader = Endless { read: 0, after: 3 * CHUNK, cancel: &cancel };
        assert_eq!(copy_to_file(&mut reader, &target, MAX_BYTES, &cancel).unwrap_err(), CopyError::Cancelled);
        assert!(reader.read < 5 * CHUNK, "stopped soon after the flag: {}", reader.read);
        assert!(files(&dir).is_empty());
        // cancelled before it starts: nothing read
        let mut reader = Endless { read: 0, after: 0, cancel: &cancel };
        assert_eq!(copy_to_file(&mut reader, &target, MAX_BYTES, &cancel).unwrap_err(), CopyError::Cancelled);
        assert_eq!(reader.read, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
