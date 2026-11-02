//! The raw types this phone hides from apps (APK-52).
//!
//! Under scoped storage (Android 11 and later) the photo permission lets an app list only the
//! files of shared storage (and of an SD card or USB drive) that Android's media database counts
//! as images or videos, and it counts a file by the MIME type of its extension. A phone whose
//! type table has no `image/*` type for `.cr3` (or `.3fr`, `.iiq`, …) leaves those files out of
//! every directory listing, so no import code can see them; they arrive only through a share or
//! "Open with" (the Files app reads them through its document provider).
//!
//! [`query_hidden_types`] (Android only) asks `MimeTypeMap.getSingleton()
//! .getMimeTypeFromExtension(ext)` for each extension, one local frame each; [`hidden_types`] is
//! the pure rule, tested on every target: an extension whose answer is not an `image/*` type
//! (no answer, `application/octet-stream`, a call that threw) is hidden on this phone.

/// At most this many extensions are asked about (the list is ours, but stays bounded).
pub const MAX_EXTENSIONS: usize = 64;

/// Whether a MIME type is an image type the media database lists to the photo permission.
fn is_image_type(mime: &str) -> bool {
    let mime = mime.trim();
    mime.len() > "image/".len() && mime.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("image/"))
}

/// The extensions among `answers` (`(extension, MIME type or None)`) this phone hides: those
/// whose type is not `image/*`, upper case ("CR3"), in order, each once, at most
/// [`MAX_EXTENSIONS`] looked at. An empty extension is skipped.
pub fn hidden_types<'a>(answers: impl IntoIterator<Item = (&'a str, Option<String>)>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (ext, mime) in answers.into_iter().take(MAX_EXTENSIONS) {
        let ext = ext.trim().trim_start_matches('.').to_ascii_uppercase();
        if ext.is_empty() || mime.as_deref().is_some_and(is_image_type) || out.contains(&ext) {
            continue;
        }
        out.push(ext);
    }
    out
}

/// The extensions among `extensions` (lower case, no dot) this phone hides ([`hidden_types`] of
/// `MimeTypeMap`'s answers), on the calling thread (attached through
/// [`crate::jni_env::with_activity`]: meant for a worker, never inside a `with_activity` closure).
#[cfg(target_os = "android")]
pub fn query_hidden_types(app: &android_activity::AndroidApp, extensions: &[&str]) -> Result<Vec<String>, crate::Error> {
    let answers = crate::jni_env::with_activity(app, |env, _activity| query_answers(env, extensions))?;
    Ok(hidden_types(extensions.iter().copied().zip(answers)))
}

/// `MimeTypeMap.getSingleton()`, then `getMimeTypeFromExtension` per extension in its own local
/// frame; an extension whose call throws or can't be read counts as no answer.
#[cfg(target_os = "android")]
fn query_answers(env: &mut jni::Env<'_>, extensions: &[&str]) -> Result<Vec<Option<String>>, crate::Error> {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{JValue, jni_sig, jni_str};
    env.with_local_frame(16, |env| -> Result<Vec<Option<String>>, crate::Error> {
        let answer =
            env.call_static_method(jni_str!("android/webkit/MimeTypeMap"), jni_str!("getSingleton"), jni_sig!("()Landroid/webkit/MimeTypeMap;"), &[]);
        let map = caught(env, answer)?.l()?;
        if map.is_null() {
            return Err(crate::Error::Jni("no MIME type map".to_owned()));
        }
        let mut answers = Vec::with_capacity(extensions.len().min(MAX_EXTENSIONS));
        for ext in extensions.iter().take(MAX_EXTENSIONS) {
            let mime = env.with_local_frame(8, |env| -> Result<Option<String>, crate::Error> {
                let ext = env.new_string(ext)?;
                let answer = env.call_method(
                    &map,
                    jni_str!("getMimeTypeFromExtension"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[JValue::Object(&ext)],
                );
                let obj = caught(env, answer)?.l()?;
                crate::content::string_of(env, obj)
            });
            answers.push(mime.unwrap_or_else(|_| {
                let _ = env.exception_catch();
                None
            }));
        }
        Ok(answers)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_types_from_mime_answers() {
        let some = |s: &str| Some(s.to_owned());
        let answers = vec![
            // types the phone's table knows as images: listed to the photo permission
            ("dng", some("image/x-adobe-dng")),
            ("cr2", some("image/x-canon-cr2")),
            ("nef", some("IMAGE/X-NIKON-NEF")),
            ("arw", some(" image/x-sony-arw ")),
            // no answer, or a type that isn't an image's: hidden
            ("cr3", None),
            ("3fr", some("application/octet-stream")),
            ("iiq", some("")),
            ("raf", some("image/")),
            ("x3f", some("video/x-sigma")),
            ("mos", some("imagex/foo")),
            // each once, upper case; an empty extension is no type
            ("CR3", None),
            (".erf", None),
            ("", None),
        ];
        assert_eq!(hidden_types(answers), ["CR3", "3FR", "IIQ", "RAF", "X3F", "MOS", "ERF"]);
        // every type known: nothing to say
        assert!(hidden_types(vec![("cr3", some("image/x-canon-cr3")), ("dng", some("image/x-adobe-dng"))]).is_empty());
        // a long list is bounded
        let many: Vec<String> = (0..1000).map(|i| format!("x{i}")).collect();
        assert_eq!(hidden_types(many.iter().map(|e| (e.as_str(), None))).len(), MAX_EXTENSIONS);
    }
}
