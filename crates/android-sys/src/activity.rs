//! The Activity itself: moving the app's task to the background (Back at the root, APK-17) and
//! asking whether it is going away (the lifecycle wrapper, APK-18).
//!
//! `move_task_to_back` posts `Activity.moveTaskToBack(true)` to the Java main thread with
//! `AndroidApp::run_on_java_main_thread` and returns at once: android_main never waits for the
//! Java main thread (module docs of [`crate::jni_env`]). What the call answered reaches `done` on
//! the Java main thread, for the log. The Back cascade that calls it is tested in
//! `lightcraft-ui-egui` with a fake service.
//!
//! `activity_state` reads `isFinishing()` and `isDestroyed()` directly on the calling thread:
//! both only read the Activity's fields, so they need no Java main thread, which may itself be
//! waiting in `onDestroy` for android_main to return. [`ActivityState`] is pure, so the host's
//! lifecycle decisions are tested on every target.

/// What the Activity says about its own end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActivityState {
    /// `Activity.isFinishing()`: `finish()` was called or the user removed the task, so the
    /// Activity will be destroyed.
    pub finishing: bool,
    /// `Activity.isDestroyed()`: `onDestroy` has begun (`performDestroy` sets it first), and the
    /// Java main thread waits there until android_main returns.
    pub destroyed: bool,
}

/// Asks the Activity whether it is finishing or destroyed (two JNI calls on the calling thread,
/// no Java main thread involved).
#[cfg(target_os = "android")]
pub fn activity_state(app: &android_activity::AndroidApp) -> Result<ActivityState, crate::Error> {
    crate::jni_env::with_activity(app, |env, activity| {
        let finishing = env.call_method(activity, jni::jni_str!("isFinishing"), jni::jni_sig!("()Z"), &[]);
        let finishing = answer_bool(env, finishing)?;
        let destroyed = env.call_method(activity, jni::jni_str!("isDestroyed"), jni::jni_sig!("()Z"), &[]);
        let destroyed = answer_bool(env, destroyed)?;
        Ok(ActivityState { finishing, destroyed })
    })
}

/// A `boolean` a Java method answered; a thrown exception is caught and named instead of the bare
/// JNI error.
#[cfg(target_os = "android")]
pub(crate) fn answer_bool(env: &jni::Env<'_>, answer: jni::errors::Result<jni::JValueOwned<'_>>) -> Result<bool, crate::Error> {
    match answer {
        Ok(answer) => Ok(answer.z()?),
        Err(e) => {
            env.exception_catch()?;
            Err(e.into())
        }
    }
}

/// Moves LightCraft's task to the background, as Android's own Back does at an app's root. Posted
/// to the Java main thread and not awaited (fire and forget); `done` gets `Ok(moved)` or the error
/// there, so the caller can log it.
#[cfg(target_os = "android")]
pub fn move_task_to_back(app: &android_activity::AndroidApp, done: impl FnOnce(Result<bool, crate::Error>) + Send + 'static) {
    let posted = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let moved = crate::jni_env::with_activity(&posted, |env, activity| {
            // `boolean moveTaskToBack(boolean nonRoot)`: `true` works from any Activity of the
            // task; the answer says whether the task moved
            match env.call_method(activity, jni::jni_str!("moveTaskToBack"), jni::jni_sig!("(Z)Z"), &[jni::JValue::Bool(true)]) {
                Ok(answer) => Ok(answer.z()?),
                Err(e) => {
                    // a thrown exception names its class and message instead of the bare JNI error
                    env.exception_catch()?;
                    Err(e.into())
                }
            }
        });
        done(moved);
    }));
}
