//! A message dialog over the Activity (`android.app.AlertDialog`), for what the user must read
//! even when LightCraft's own window can't draw: a renderer that failed to start (APK-19). A
//! text-input variant follows for APK-35.
//!
//! [`show_alert`] posts the dialog to the Java main thread with
//! `AndroidApp::run_on_java_main_thread` and returns at once: android_main never waits for the Java
//! main thread (module docs of [`crate::jni_env`]). The dialog's one button has a `null` listener,
//! so it just closes the dialog; Back and a tap outside close it too. android_main then asks
//! [`Alert::status`] as often as it likes: `Dialog.isShowing()` only reads a field, so it is called
//! directly on the calling thread. The mapping from what is known to a [`Status`] is pure
//! ([`status_from`]) and tested on every target.

use crate::Error;

/// What a posted dialog is doing, as android_main sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Posted; the Java main thread has not built it yet.
    Pending,
    /// On screen.
    Showing,
    /// Closed: by its button, Back or a tap outside, or by the system.
    Dismissed,
    /// It could not be shown, or asking about it failed.
    Failed(Error),
}

/// The status from what the Java main thread reported (`None`: nothing yet; `Some(Err)`: building
/// or showing the dialog failed) and, once it is shown, `is_showing` (`Dialog.isShowing()`).
pub fn status_from(posted: Option<Result<(), Error>>, is_showing: impl FnOnce() -> Result<bool, Error>) -> Status {
    match posted {
        None => Status::Pending,
        Some(Err(e)) => Status::Failed(e),
        Some(Ok(())) => match is_showing() {
            Ok(true) => Status::Showing,
            Ok(false) => Status::Dismissed,
            Err(e) => Status::Failed(e),
        },
    }
}

/// What the Java main thread hands back: the shown dialog, or why it could not be shown.
#[cfg(target_os = "android")]
type Posted = Option<Result<jni::refs::Global<jni::objects::JObject<'static>>, Error>>;

/// A dialog posted by [`show_alert`].
#[cfg(target_os = "android")]
pub struct Alert {
    posted: std::sync::Arc<std::sync::Mutex<Posted>>,
}

/// Posts an `AlertDialog` with `title`, `message` and one `button` (a `null` listener: it only
/// closes the dialog) to the Java main thread, and returns at once.
#[cfg(target_os = "android")]
pub fn show_alert(app: &android_activity::AndroidApp, title: &str, message: &str, button: &str) -> Alert {
    use std::sync::{Arc, Mutex, PoisonError};
    let posted: Arc<Mutex<Posted>> = Arc::new(Mutex::new(None));
    let (slot, on_main, title, message, button) = (posted.clone(), app.clone(), title.to_owned(), message.to_owned(), button.to_owned());
    app.run_on_java_main_thread(Box::new(move || {
        let shown = crate::jni_env::with_activity(&on_main, |env, activity| build_and_show(env, activity, &title, &message, &button));
        // (an error reaches android_main through `Alert::status`, which logs it)
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(shown);
    }));
    Alert { posted }
}

/// `new AlertDialog.Builder(activity).setTitle(…).setMessage(…).setPositiveButton(…, null)
/// .setCancelable(true).show()`, as a global reference so android_main can ask about it later.
#[cfg(target_os = "android")]
fn build_and_show(
    env: &mut jni::Env<'_>,
    activity: &jni::objects::JObject<'_>,
    title: &str,
    message: &str,
    button: &str,
) -> Result<jni::refs::Global<jni::objects::JObject<'static>>, Error> {
    use jni::objects::JObject;
    use jni::{JValue, jni_sig, jni_str};
    let title = env.new_string(title)?;
    let message = env.new_string(message)?;
    let button = env.new_string(button)?;
    let builder = env.new_object(jni_str!("android/app/AlertDialog$Builder"), jni_sig!("(Landroid/content/Context;)V"), &[JValue::Object(activity)]);
    let builder = caught(env, builder)?;
    let sets_text = jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/AlertDialog$Builder;");
    call(env, &builder, jni_str!("setTitle"), &sets_text, &[JValue::Object(&title)])?;
    call(env, &builder, jni_str!("setMessage"), &sets_text, &[JValue::Object(&message)])?;
    let null = JObject::null();
    let sets_button = jni_sig!("(Ljava/lang/CharSequence;Landroid/content/DialogInterface$OnClickListener;)Landroid/app/AlertDialog$Builder;");
    call(env, &builder, jni_str!("setPositiveButton"), &sets_button, &[JValue::Object(&button), JValue::Object(&null)])?;
    call(env, &builder, jni_str!("setCancelable"), &jni_sig!("(Z)Landroid/app/AlertDialog$Builder;"), &[JValue::Bool(true)])?;
    let dialog = env.call_method(&builder, jni_str!("show"), jni_sig!("()Landroid/app/AlertDialog;"), &[]);
    let dialog = caught(env, dialog)?.l()?;
    Ok(env.new_global_ref(&dialog)?)
}

/// One builder call whose answer (the builder itself) is not needed.
#[cfg(target_os = "android")]
fn call(
    env: &mut jni::Env<'_>,
    obj: &jni::objects::JObject<'_>,
    name: &jni::strings::JNIStr,
    sig: &jni::signature::MethodSignature<'_, '_>,
    args: &[jni::JValue<'_>],
) -> Result<(), Error> {
    let answer = env.call_method(obj, name, sig, args);
    caught(env, answer).map(|_| ())
}

/// A JNI result; for a thrown exception, the exception's class and message (cleared) instead of
/// the bare JNI error.
#[cfg(target_os = "android")]
fn caught<T>(env: &jni::Env<'_>, answer: jni::errors::Result<T>) -> Result<T, Error> {
    answer.map_err(|e| match env.exception_catch() {
        Err(java) => java.into(),
        Ok(()) => e.into(),
    })
}

#[cfg(target_os = "android")]
impl Alert {
    /// What the dialog is doing now. Asks `isShowing()` on the calling thread (attached through
    /// [`crate::jni_env::with_activity`], so never from inside a `with_activity` closure).
    pub fn status(&self, app: &android_activity::AndroidApp) -> Status {
        use std::sync::PoisonError;
        let posted = self.posted.lock().unwrap_or_else(PoisonError::into_inner);
        let known = posted.as_ref().map(|shown| shown.as_ref().map(|_| ()).map_err(Clone::clone));
        status_from(known, || match posted.as_ref() {
            Some(Ok(dialog)) => crate::jni_env::with_activity(app, |env, _activity| {
                let showing = env.call_method(dialog, jni::jni_str!("isShowing"), jni::jni_sig!("()Z"), &[]);
                crate::activity::answer_bool(env, showing)
            }),
            // (status_from asks only once the dialog is shown)
            _ => Ok(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialog is pending until the Java main thread built it, showing while `isShowing()`
    /// answers true, dismissed once it answers false; a failed build or query is an error, and
    /// `isShowing()` is never asked before the dialog exists.
    #[test]
    fn dialog_status_follows_the_posted_dialog() {
        let asked = std::cell::Cell::new(0);
        let ask = |answer: Result<bool, Error>| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                answer
            }
        };
        assert_eq!(status_from(None, ask(Ok(true))), Status::Pending);
        let failed = Error::Java { class: "android.view.WindowManager$BadTokenException".into(), message: "is your activity running?".into() };
        assert_eq!(status_from(Some(Err(failed.clone())), ask(Ok(true))), Status::Failed(failed));
        assert_eq!(asked.get(), 0, "isShowing() is asked only once the dialog exists");
        assert_eq!(status_from(Some(Ok(())), ask(Ok(true))), Status::Showing);
        assert_eq!(status_from(Some(Ok(())), ask(Ok(false))), Status::Dismissed);
        assert_eq!(status_from(Some(Ok(())), ask(Err(Error::Reentrant))), Status::Failed(Error::Reentrant));
        assert_eq!(asked.get(), 3);
    }
}
