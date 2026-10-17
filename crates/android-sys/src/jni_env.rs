//! The JVM, the Activity object and Java errors.
//!
//! `with_activity` (Android only) runs a closure with a JNI `Env` attached to the
//! current thread and the app's `Activity` as a `JObject`. Its rules, which every caller in the
//! Android crates follows:
//!
//! - **Local references per iteration.** Every loop over Java objects (cursor rows, array
//!   elements, list items) runs each iteration inside `env.with_local_frame(16, |env| …)`. ART's
//!   local-reference table is small; overflowing it is a native abort no panic guard can catch.
//! - **One `Env` in scope.** Always shadow `env`; inside `with_local_frame` use only the closure's
//!   `env` (jni 0.22 panics at run time when a non-top `Env` is used, env.rs:417).
//! - **Not re-entrant.** Never call `with_activity` inside a JNI closure: a nested call returns
//!   [`Error::Reentrant`]. Android-sys functions meant to be called from inside a `with_activity`
//!   closure take `&mut Env` instead.
//! - **Never wait on the Java main thread.** Code on `android_main` never waits for the result of
//!   `AndroidApp::run_on_java_main_thread` (android-activity lib.rs:922-925 warns of the
//!   deadlock): results come back by `try_recv`, polled once per frame.
//! - **Long work on a worker.** Copies, queries and imports run on a worker thread that attaches
//!   with `JavaVM::singleton()?.attach_current_thread(…)` and holds a `TaskGuard`.
//! - **Java errors.** A Java exception becomes [`Error::Java`] with the exception's class and
//!   message (inside a closure `env.exception_catch()?` gives the same); [`user_message`] turns it
//!   into text the user can act on.

use std::cell::Cell;
use std::fmt;

/// What a JNI call can fail with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A Java exception was thrown (and cleared): its class name (`java.lang.SecurityException`)
    /// and message (may be empty).
    Java { class: String, message: String },
    /// `with_activity` was called while a `with_activity` closure was running on this thread.
    Reentrant,
    /// Any other JNI failure (no JVM, a missing method, a wrong type), as text for the log.
    Jni(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Java { class, message } if message.is_empty() => write!(f, "{class}"),
            Error::Java { class, message } => write!(f, "{class}: {message}"),
            Error::Reentrant => f.write_str("with_activity called inside a with_activity closure"),
            Error::Jni(e) => write!(f, "JNI: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The JNI failures that carry a Java exception, independent of the `jni` crate so the mapping is
/// tested on every target.
#[cfg(any(target_os = "android", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure<'a> {
    /// `jni::errors::Error::CaughtJavaException { name, msg, .. }`.
    Caught { name: &'a str, msg: &'a str },
    /// `jni::errors::Error::SecurityViolation`.
    SecurityViolation,
}

#[cfg(any(target_os = "android", test))]
fn from_failure(f: Failure<'_>) -> Error {
    match f {
        Failure::Caught { name, msg } => Error::Java { class: name.to_owned(), message: msg.to_owned() },
        Failure::SecurityViolation => Error::Java { class: "java.lang.SecurityException".to_owned(), message: String::new() },
    }
}

#[cfg(target_os = "android")]
impl From<jni::errors::Error> for Error {
    fn from(e: jni::errors::Error) -> Self {
        match e {
            // `attach_current_thread` also returns this for an exception left pending by a
            // closure (vm/java_vm.rs:479-481); `env.exception_catch()` builds it too.
            jni::errors::Error::CaughtJavaException { name, msg, .. } => from_failure(Failure::Caught { name: &name, msg: &msg }),
            jni::errors::Error::SecurityViolation => from_failure(Failure::SecurityViolation),
            other => Error::Jni(other.to_string()),
        }
    }
}

/// Text the user can act on for a Java error that happened while doing `action` (a short phrase
/// such as "open the photo"). Known exception classes get a plain explanation; any other class is
/// named with its message, so a report still says what happened.
pub fn user_message(class: &str, message: &str, action: &str) -> String {
    // `Class.getName()` is dotted (`android.content.ActivityNotFoundException`), nested classes
    // use `$`; match the simple name so a JNI-style `java/lang/…` name works too.
    let simple = class.rsplit(['.', '/', '$']).next().unwrap_or(class);
    let known = match simple {
        "ActivityNotFoundException" => Some("No app can open this."),
        "SecurityException" => Some("Access was withdrawn. Choose the file again, or allow access in the system settings."),
        "FileNotFoundException" => Some("The file is gone. It may have been moved or deleted."),
        _ => None,
    };
    match (known, message.trim()) {
        (Some(text), _) => format!("Could not {action}: {text}"),
        (None, "") => format!("Could not {action}: {simple}."),
        (None, m) => format!("Could not {action}: {simple} ({m})."),
    }
}

impl Error {
    /// [`user_message`] for this error; non-Java errors keep their log text.
    pub fn user_message(&self, action: &str) -> String {
        match self {
            Error::Java { class, message } => user_message(class, message, action),
            other => format!("Could not {action}: {other}."),
        }
    }
}

/// Marks a thread as inside a `with_activity` closure for as long as it lives. Pure (works on a
/// given flag) so the rule is tested on every target; `with_activity` uses a thread-local flag.
#[derive(Debug)]
pub struct ReentryGuard<'a> {
    flag: &'a Cell<bool>,
}

impl<'a> ReentryGuard<'a> {
    /// Enter: `Err(Error::Reentrant)` when `flag` is already set (a nested call), else sets it
    /// until the guard is dropped.
    pub fn enter(flag: &'a Cell<bool>) -> Result<Self, Error> {
        if flag.replace(true) {
            return Err(Error::Reentrant);
        }
        Ok(ReentryGuard { flag })
    }
}

impl Drop for ReentryGuard<'_> {
    fn drop(&mut self) {
        self.flag.set(false);
    }
}

#[cfg(target_os = "android")]
thread_local! {
    static IN_ACTIVITY: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` with a JNI `Env` attached to this thread and the app's `Activity`. Not re-entrant
/// (a nested call returns [`Error::Reentrant`]); see the module docs for the rules inside `f`.
/// A Java exception thrown in `f` and not caught there comes back as [`Error::Java`].
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
pub fn with_activity<T>(
    app: &android_activity::AndroidApp,
    f: impl FnOnce(&mut jni::Env<'_>, &jni::objects::JObject<'_>) -> Result<T, Error>,
) -> Result<T, Error> {
    IN_ACTIVITY.with(|flag| {
        let _guard = ReentryGuard::enter(flag)?;
        let vm = jni::JavaVM::singleton()?;
        let raw: jni::sys::jobject = app.activity_as_ptr().cast();
        vm.attach_current_thread(|env| {
            // SAFETY: `activity_as_ptr` is android-activity's global reference to the Activity,
            // valid as long as `app` (borrowed for this whole call) lives; we don't own it and
            // `Cast` never deletes it (android-activity lib.rs:814-836). `as_cast_raw` still checks
            // at run time that the object is a `java.lang.Object`.
            let activity = unsafe { env.as_cast_raw::<jni::refs::Global<jni::objects::JObject>>(&raw)? };
            f(env, &activity)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_exception_maps_to_actionable_error() {
        let e = from_failure(Failure::Caught { name: "android.content.ActivityNotFoundException", msg: "No Activity found to handle Intent" });
        assert_eq!(
            e,
            Error::Java { class: "android.content.ActivityNotFoundException".into(), message: "No Activity found to handle Intent".into() }
        );
        assert_eq!(e.user_message("open the photo"), "Could not open the photo: No app can open this.");
        assert!(user_message("java.io.FileNotFoundException", "/x: open failed", "import").contains("The file is gone"));
        assert!(user_message("java/lang/SecurityException", "", "read the photo").contains("Access was withdrawn"));
        // unknown classes keep their simple name and message
        assert_eq!(user_message("java.lang.IllegalStateException", "closed", "share"), "Could not share: IllegalStateException (closed).");
        assert_eq!(user_message("Outer$Inner", " ", "share"), "Could not share: Inner.");
    }

    #[test]
    fn security_violation_maps_to_security_exception() {
        let e = from_failure(Failure::SecurityViolation);
        assert_eq!(e, Error::Java { class: "java.lang.SecurityException".into(), message: String::new() });
        assert_eq!(e.to_string(), "java.lang.SecurityException");
        assert!(e.user_message("read the photo").starts_with("Could not read the photo: Access was withdrawn."));
    }

    #[test]
    fn nested_with_activity_is_an_error() {
        let flag = Cell::new(false);
        {
            let _outer = ReentryGuard::enter(&flag).unwrap();
            assert_eq!(ReentryGuard::enter(&flag).unwrap_err(), Error::Reentrant);
            // the failed nested enter must not clear the outer guard's mark
            assert!(flag.get());
        }
        assert!(!flag.get());
        assert!(ReentryGuard::enter(&flag).is_ok());
    }
}
