//! The system clipboard (APK-28): `ClipboardManager.setPrimaryClip(ClipData.newPlainText(…))`
//! for a copy, `getPrimaryClip()` → `getItemAt(0).coerceToText(context)` for a paste.
//!
//! Both are binder calls that work from any thread, so they run directly on the calling thread
//! (android_main) through [`crate::jni_env::with_activity`]: no hop to the Java main thread, which
//! android_main must never wait on. Android 10 and later answer `getPrimaryClip()` only while the
//! app's window has the focus (and return null otherwise), which is the case whenever the user can
//! paste into a field. Each call runs in one local frame; a Java exception (`SecurityException`,
//! a `TransactionTooLargeException` for a huge copy) comes back as [`crate::Error::Java`].
//!
//! The text pasted is hostile input: [`paste_text`] (pure, tested on every target) caps it and
//! turns CRLF into LF, as egui-winit does on the desktop.

/// The most text a paste brings in (bytes): another app's clipboard can hold megabytes.
pub const MAX_PASTE_BYTES: usize = 1 << 20;

/// The label of the clips LightCraft puts on the clipboard (shown by some keyboards).
pub const CLIP_LABEL: &str = "LightCraft";

/// The text a clip's first item gave, as a paste: `None` for no clip or no text, CRLF as LF, cut
/// to [`MAX_PASTE_BYTES`] on a char boundary.
pub fn paste_text(clip: Option<String>) -> Option<String> {
    let text = clip?;
    let text = if text.contains('\r') { text.replace("\r\n", "\n") } else { text };
    if text.is_empty() {
        return None;
    }
    if text.len() <= MAX_PASTE_BYTES {
        return Some(text);
    }
    let mut end = MAX_PASTE_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).map(str::to_owned)
}

/// The `ClipboardManager` (`Context.getSystemService("clipboard")`).
#[cfg(target_os = "android")]
fn manager<'local>(env: &mut jni::Env<'local>, activity: &jni::objects::JObject<'_>) -> Result<jni::objects::JObject<'local>, crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    let name = env.new_string("clipboard")?;
    let answer =
        env.call_method(activity, jni_str!("getSystemService"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"), &[JValue::Object(&name)]);
    let manager = caught(env, answer)?.l()?;
    if manager.is_null() {
        return Err(crate::Error::Jni("no clipboard service".into()));
    }
    Ok(manager)
}

/// Puts `text` on the system clipboard as plain text (on the calling thread; never inside a
/// `with_activity` closure).
#[cfg(target_os = "android")]
pub fn set_text(app: &android_activity::AndroidApp, text: &str) -> Result<(), crate::Error> {
    crate::jni_env::with_activity(app, |env, activity| {
        use crate::dialog::caught;
        use jni::{JValue, jni_sig, jni_str};
        env.with_local_frame(16, |env| -> Result<(), crate::Error> {
            let manager = manager(env, activity)?;
            let label = env.new_string(CLIP_LABEL)?;
            let value = env.new_string(text)?;
            let answer = env.call_static_method(
                jni_str!("android/content/ClipData"),
                jni_str!("newPlainText"),
                jni_sig!("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"),
                &[JValue::Object(&label), JValue::Object(&value)],
            );
            let clip = caught(env, answer)?.l()?;
            let answer = env.call_method(&manager, jni_str!("setPrimaryClip"), jni_sig!("(Landroid/content/ClipData;)V"), &[JValue::Object(&clip)]);
            caught(env, answer)?;
            Ok(())
        })
    })
}

/// The text on the system clipboard (its first item, coerced to text), as [`paste_text`] gives
/// it: `None` when the clipboard is empty, holds no text, or Android withholds it (the window
/// doesn't have the focus). On the calling thread; never inside a `with_activity` closure.
#[cfg(target_os = "android")]
pub fn get_text(app: &android_activity::AndroidApp) -> Result<Option<String>, crate::Error> {
    crate::jni_env::with_activity(app, |env, activity| {
        use crate::dialog::caught;
        use jni::{JValue, jni_sig, jni_str};
        env.with_local_frame(16, |env| -> Result<Option<String>, crate::Error> {
            let manager = manager(env, activity)?;
            let answer = env.call_method(&manager, jni_str!("getPrimaryClip"), jni_sig!("()Landroid/content/ClipData;"), &[]);
            let clip = caught(env, answer)?.l()?;
            if clip.is_null() {
                return Ok(None);
            }
            let answer = env.call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[]);
            if caught(env, answer)?.i()? < 1 {
                return Ok(None);
            }
            let answer = env.call_method(&clip, jni_str!("getItemAt"), jni_sig!("(I)Landroid/content/ClipData$Item;"), &[JValue::Int(0)]);
            let item = caught(env, answer)?.l()?;
            if item.is_null() {
                return Ok(None);
            }
            let answer = env.call_method(
                &item,
                jni_str!("coerceToText"),
                jni_sig!("(Landroid/content/Context;)Ljava/lang/CharSequence;"),
                &[JValue::Object(activity)],
            );
            let chars = caught(env, answer)?.l()?;
            if chars.is_null() {
                return Ok(None);
            }
            let answer = env.call_method(&chars, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[]);
            let text = caught(env, answer)?.l()?;
            Ok(paste_text(crate::content::string_of(env, text)?))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_text_is_bounded_and_plain() {
        assert_eq!(paste_text(None), None);
        assert_eq!(paste_text(Some(String::new())), None);
        assert_eq!(paste_text(Some("Lisbon".into())).as_deref(), Some("Lisbon"));
        assert_eq!(paste_text(Some("a\r\nb\rc".into())).as_deref(), Some("a\nb\rc"));
        // a huge clip is cut on a char boundary (é is two bytes, so the cut can't fall inside one)
        let huge = format!("x{}", "é".repeat(MAX_PASTE_BYTES));
        let cut = paste_text(Some(huge)).unwrap();
        assert!(cut.len() <= MAX_PASTE_BYTES && cut.len() > MAX_PASTE_BYTES - 4, "{}", cut.len());
        assert!(cut.starts_with("xé"));
    }
}
