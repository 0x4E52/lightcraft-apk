//! Intents: what another app sent LightCraft ("Open with", share one, share several; APK-25).
//!
//! [`IntentSpec`] is the pure part, tested on every target: [`IntentSpec::parse`] takes what the
//! Activity's `getIntent()` carries (action, data URI, type, flags and the `EXTRA_STREAM` extras)
//! and [`IntentSpec::received`] names the URIs to copy, each once. [`read_intent`] (Android only)
//! reads `getIntent()` on the calling thread.
//!
//! NativeActivity doesn't override `onNewIntent`, and `Activity.onNewIntent` doesn't call
//! `setIntent`, so `getIntent()` never changes while the Activity lives: the host reads it once
//! per `android_main` start. A second share while LightCraft runs only brings it to the front.

/// `Intent.ACTION_MAIN`: started from the launcher, nothing to receive.
pub const ACTION_MAIN: &str = "android.intent.action.MAIN";
/// `Intent.ACTION_VIEW`: "Open with"; the URI is the intent's data.
pub const ACTION_VIEW: &str = "android.intent.action.VIEW";
/// `Intent.ACTION_SEND`: share one; the URI is the `EXTRA_STREAM` extra.
pub const ACTION_SEND: &str = "android.intent.action.SEND";
/// `Intent.ACTION_SEND_MULTIPLE`: share several; the URIs are the `EXTRA_STREAM` list.
pub const ACTION_SEND_MULTIPLE: &str = "android.intent.action.SEND_MULTIPLE";
/// `Intent.EXTRA_STREAM`.
pub const EXTRA_STREAM: &str = "android.intent.extra.STREAM";
/// `Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`: the Activity was started again from Recents
/// with the intent that once started it, whose files were received then (and whose read grant
/// is gone).
pub const FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY: i32 = 0x0010_0000;

/// At most this many URIs are taken from one intent (a hostile sender can't grow the list forever).
pub const MAX_URIS: usize = 10_000;

/// The `EXTRA_STREAM` extras, as URI strings: the single one (`ACTION_SEND`) and the list
/// (`ACTION_SEND_MULTIPLE`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    pub stream: Option<String>,
    pub streams: Vec<String>,
}

/// An intent, as plain data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntentSpec {
    /// `getAction()` (`None` for none).
    pub action: Option<String>,
    /// `getDataString()`.
    pub data: Option<String>,
    /// `getType()`: the MIME type the sender gave, if any.
    pub mime: Option<String>,
    /// `getFlags()`.
    pub flags: i32,
    /// The `EXTRA_STREAM` URIs.
    pub extras: Extras,
}

impl IntentSpec {
    /// The intent made of these parts; empty strings count as missing.
    pub fn parse(action: Option<&str>, data: Option<&str>, mime: Option<&str>, flags: i32, extras: Extras) -> IntentSpec {
        let text = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        IntentSpec { action: text(action), data: text(data), mime: text(mime), flags, extras }
    }

    /// The content LightCraft was sent, each URI once in the order given: `ACTION_VIEW`'s data,
    /// `ACTION_SEND`'s stream, `ACTION_SEND_MULTIPLE`'s list. Only `content://` and `file://`
    /// URIs; nothing for any other action, or for a start from Recents (the same intent again).
    pub fn received(&self) -> Vec<String> {
        if self.flags & FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0 {
            return Vec::new();
        }
        let uris: Vec<&str> = match self.action.as_deref() {
            Some(ACTION_VIEW) => self.data.as_deref().into_iter().collect(),
            Some(ACTION_SEND) => self.extras.stream.as_deref().into_iter().collect(),
            Some(ACTION_SEND_MULTIPLE) => self.extras.streams.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        let mut out: Vec<String> = Vec::new();
        for uri in uris {
            let uri = uri.trim();
            if out.len() >= MAX_URIS {
                break;
            }
            if receivable(uri) && !out.iter().any(|seen| seen == uri) {
                out.push(uri.to_owned());
            }
        }
        out
    }
}

impl IntentSpec {
    /// `ACTION_VIEW` for a web link (Help, About; APK-26). Only `https://` with a host and no
    /// whitespace or control characters: the URL may come from a command argument, so anything
    /// else (`http:`, `intent:`, `file:`, `javascript:`) is refused here, before Android sees it.
    pub fn view(url: &str) -> Option<IntentSpec> {
        let url = url.trim();
        let (scheme, rest) = url.split_once("://")?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = authority.split(':').next().unwrap_or("");
        let ok = scheme.eq_ignore_ascii_case("https")
            && !host.is_empty()
            && !authority.starts_with('@')
            && !url.chars().any(|c| c.is_whitespace() || c.is_control());
        // Android matches intent-filter schemes case-sensitively: always hand over lowercase
        ok.then(|| IntentSpec { action: Some(ACTION_VIEW.to_owned()), data: Some(format!("https://{rest}")), ..IntentSpec::default() })
    }
}

/// What Help ▸ a link says when starting the browser failed: no app handles links, or the JNI error.
pub fn open_link_message(e: &crate::Error) -> String {
    match e {
        crate::Error::Java { class, .. } if class.rsplit(['.', '/', '$']).next() == Some("ActivityNotFoundException") => {
            "No app can open links".to_owned()
        }
        other => other.user_message("open the link"),
    }
}

/// `startActivity(new Intent(action, Uri.parse(data)))` on the calling thread (attached through
/// [`crate::jni_env::with_activity`], so never from inside a `with_activity` closure). Android
/// allows it off the Java main thread, so there is no hop and nothing to wait for; an exception
/// (`ActivityNotFoundException`) comes back as the error.
#[cfg(target_os = "android")]
pub fn start_view(app: &android_activity::AndroidApp, spec: &IntentSpec) -> Result<(), crate::Error> {
    let (Some(action), Some(data)) = (spec.action.as_deref(), spec.data.as_deref()) else {
        return Err(crate::Error::Jni("the intent has no action or data".into()));
    };
    crate::jni_env::with_activity(app, |env, activity| {
        use crate::dialog::caught;
        use jni::{JValue, jni_sig, jni_str};
        env.with_local_frame(16, |env| -> Result<(), crate::Error> {
            let action = env.new_string(action)?;
            let text = env.new_string(data)?;
            let answer = env.call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(&text)],
            );
            let uri = caught(env, answer)?.l()?;
            let intent = env.new_object(
                jni_str!("android/content/Intent"),
                jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
                &[JValue::Object(&action), JValue::Object(&uri)],
            );
            let intent = caught(env, intent)?;
            let started = env.call_method(activity, jni_str!("startActivity"), jni_sig!("(Landroid/content/Intent;)V"), &[JValue::Object(&intent)]);
            caught(env, started)?;
            Ok(())
        })
    })
}

/// A URI LightCraft opens through the ContentResolver: `content://…`, or `file://…` from an old
/// sender (the resolver applies the file's own permissions).
fn receivable(uri: &str) -> bool {
    let scheme = uri.split_once("://").map(|(s, rest)| (s.to_ascii_lowercase(), rest));
    matches!(scheme, Some((s, rest)) if (s == "content" || s == "file") && !rest.is_empty())
}

/// What `getIntent()` holds now, read on the calling thread (attached through
/// [`crate::jni_env::with_activity`], so never from inside a `with_activity` closure).
#[cfg(target_os = "android")]
pub fn read_intent(app: &android_activity::AndroidApp) -> Result<IntentSpec, crate::Error> {
    crate::jni_env::with_activity(app, |env, activity| {
        use crate::dialog::caught;
        use jni::{jni_sig, jni_str};
        env.with_local_frame(16, |env| -> Result<IntentSpec, crate::Error> {
            let answer = env.call_method(activity, jni_str!("getIntent"), jni_sig!("()Landroid/content/Intent;"), &[]);
            let intent = caught(env, answer)?.l()?;
            if intent.is_null() {
                return Ok(IntentSpec::default());
            }
            let text = |env: &mut jni::Env<'_>, name: &'static jni::strings::JNIStr| -> Result<Option<String>, crate::Error> {
                let answer = env.call_method(&intent, name, jni_sig!("()Ljava/lang/String;"), &[]);
                let value = caught(env, answer)?.l()?;
                crate::content::string_of(env, value)
            };
            let action = text(env, jni_str!("getAction"))?;
            let data = text(env, jni_str!("getDataString"))?;
            let mime = text(env, jni_str!("getType"))?;
            let answer = env.call_method(&intent, jni_str!("getFlags"), jni_sig!("()I"), &[]);
            let flags = caught(env, answer)?.i()?;
            let mut extras = Extras::default();
            match action.as_deref() {
                Some(ACTION_SEND) => extras.stream = stream_extra(env, &intent)?,
                Some(ACTION_SEND_MULTIPLE) => extras.streams = stream_list_extra(env, &intent)?,
                _ => {}
            }
            Ok(IntentSpec::parse(action.as_deref(), data.as_deref(), mime.as_deref(), flags, extras))
        })
    })
}

/// `getParcelableExtra(EXTRA_STREAM)` as text (deprecated since API 33 but working; the typed
/// overload exists only from 33). A value that is not a `Uri` reads as its `toString()`, which
/// [`IntentSpec::received`] then refuses unless it is a content or file URI.
#[cfg(target_os = "android")]
fn stream_extra(env: &mut jni::Env<'_>, intent: &jni::objects::JObject<'_>) -> Result<Option<String>, crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    env.with_local_frame(8, |env| -> Result<Option<String>, crate::Error> {
        let key = env.new_string(EXTRA_STREAM)?;
        let answer =
            env.call_method(intent, jni_str!("getParcelableExtra"), jni_sig!("(Ljava/lang/String;)Landroid/os/Parcelable;"), &[JValue::Object(&key)]);
        let uri = caught(env, answer)?.l()?;
        to_string(env, &uri)
    })
}

/// `getParcelableArrayListExtra(EXTRA_STREAM)` as text, one local frame per element.
#[cfg(target_os = "android")]
fn stream_list_extra(env: &mut jni::Env<'_>, intent: &jni::objects::JObject<'_>) -> Result<Vec<String>, crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    env.with_local_frame(8, |env| -> Result<Vec<String>, crate::Error> {
        let key = env.new_string(EXTRA_STREAM)?;
        let answer = env.call_method(
            intent,
            jni_str!("getParcelableArrayListExtra"),
            jni_sig!("(Ljava/lang/String;)Ljava/util/ArrayList;"),
            &[JValue::Object(&key)],
        );
        let list = caught(env, answer)?.l()?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        let answer = env.call_method(&list, jni_str!("size"), jni_sig!("()I"), &[]);
        let size = usize::try_from(caught(env, answer)?.i()?).unwrap_or(0).min(MAX_URIS);
        let mut out = Vec::with_capacity(size);
        for i in 0..size {
            let index = i32::try_from(i).map_err(|_| crate::Error::Jni("list index too large".to_owned()))?;
            let item = env.with_local_frame(8, |env| -> Result<Option<String>, crate::Error> {
                let answer = env.call_method(&list, jni_str!("get"), jni_sig!("(I)Ljava/lang/Object;"), &[JValue::Int(index)]);
                let uri = caught(env, answer)?.l()?;
                to_string(env, &uri)
            })?;
            out.extend(item);
        }
        Ok(out)
    })
}

/// `obj.toString()` (`None` for null).
#[cfg(target_os = "android")]
fn to_string(env: &mut jni::Env<'_>, obj: &jni::objects::JObject<'_>) -> Result<Option<String>, crate::Error> {
    use crate::dialog::caught;
    use jni::{jni_sig, jni_str};
    if obj.is_null() {
        return Ok(None);
    }
    let answer = env.call_method(obj, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[]);
    let text = caught(env, answer)?.l()?;
    crate::content::string_of(env, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(action: &str, data: Option<&str>, extras: Extras) -> IntentSpec {
        IntentSpec::parse(Some(action), data, Some("image/jpeg"), 0, extras)
    }

    #[test]
    fn view_spec_rejects_non_https() {
        for url in [
            "http://example.com",
            "intent://x#Intent;end",
            "file:///sdcard/a",
            "javascript:alert(1)",
            "https://",
            "https:///path",
            "https://:443/x",
            "https://exa mple.com",
            "https://example.com/a\nb",
            "example.com",
            "",
        ] {
            assert!(IntentSpec::view(url).is_none(), "{url:?}");
        }
    }

    #[test]
    fn view_spec_fields() {
        let s = IntentSpec::view("  HTTPS://github.com/0x4E52/lightcraft-apk?a=b ").unwrap();
        assert_eq!(s.action.as_deref(), Some(ACTION_VIEW));
        assert_eq!(s.data.as_deref(), Some("https://github.com/0x4E52/lightcraft-apk?a=b"));
        assert_eq!(s.flags, 0);
        // not content to receive
        assert!(s.received().is_empty());
        let e = crate::Error::Java { class: "android.content.ActivityNotFoundException".into(), message: "x".into() };
        assert_eq!(open_link_message(&e), "No app can open links");
        assert!(open_link_message(&crate::Error::Reentrant).starts_with("Could not open the link"));
    }

    #[test]
    fn view_reads_data() {
        let s = spec(ACTION_VIEW, Some("content://com.android.providers.downloads/document/12"), Extras::default());
        assert_eq!(s.received(), ["content://com.android.providers.downloads/document/12"]);
        assert_eq!(s.mime.as_deref(), Some("image/jpeg"));
        // VIEW ignores a stream extra; no data, nothing to receive
        let s = spec(ACTION_VIEW, None, Extras { stream: Some("content://x/1".into()), streams: vec![] });
        assert!(s.received().is_empty());
        // a web link or another scheme is not content LightCraft copies
        for data in ["https://example.com/a.jpg", "content://", "javascript:alert(1)", "/sdcard/a.jpg"] {
            assert!(spec(ACTION_VIEW, Some(data), Extras::default()).received().is_empty(), "{data}");
        }
        assert_eq!(spec(ACTION_VIEW, Some("file:///sdcard/a.dng"), Extras::default()).received(), ["file:///sdcard/a.dng"]);
        // the launcher's MAIN and no action at all receive nothing
        assert!(spec(ACTION_MAIN, Some("content://x/1"), Extras::default()).received().is_empty());
        assert!(IntentSpec::parse(None, Some("content://x/1"), None, 0, Extras::default()).received().is_empty());
    }

    #[test]
    fn send_reads_extra_stream() {
        let s = spec(
            ACTION_SEND,
            Some("content://ignored/9"),
            Extras { stream: Some(" content://media/external/images/media/7 ".into()), streams: vec![] },
        );
        assert_eq!(s.received(), ["content://media/external/images/media/7"]);
        // a text share has no stream
        assert!(spec(ACTION_SEND, None, Extras::default()).received().is_empty());
    }

    #[test]
    fn send_multiple_reads_list() {
        let streams = vec!["content://a/1".to_owned(), "content://a/2".to_owned(), "https://x/3".to_owned(), "content://a/3".to_owned()];
        let s = spec(ACTION_SEND_MULTIPLE, None, Extras { stream: Some("content://single/0".into()), streams });
        assert_eq!(s.received(), ["content://a/1", "content://a/2", "content://a/3"]);
        // capped
        let many: Vec<String> = (0..MAX_URIS + 5).map(|i| format!("content://a/{i}")).collect();
        assert_eq!(spec(ACTION_SEND_MULTIPLE, None, Extras { stream: None, streams: many }).received().len(), MAX_URIS);
    }

    #[test]
    fn same_uri_handled_once() {
        // a sender that lists a URI twice: copied once
        let streams = vec!["content://a/1".to_owned(), "content://a/2".to_owned(), "content://a/1".to_owned()];
        assert_eq!(spec(ACTION_SEND_MULTIPLE, None, Extras { stream: None, streams }).received(), ["content://a/1", "content://a/2"]);
        // the Activity started again from Recents carries the intent that was handled then
        let again = IntentSpec::parse(
            Some(ACTION_SEND),
            None,
            None,
            FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY | 0x1000_0000,
            Extras { stream: Some("content://a/1".into()), streams: vec![] },
        );
        assert!(again.received().is_empty());
        // empty parts are missing parts
        assert_eq!(IntentSpec::parse(Some(" "), Some(""), Some(""), 0, Extras::default()), IntentSpec::default());
    }
}
