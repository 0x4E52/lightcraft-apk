//! The Activity itself: moving the app's task to the background (Back at the root, APK-17).
//!
//! `move_task_to_back` posts `Activity.moveTaskToBack(true)` to the Java main thread with
//! `AndroidApp::run_on_java_main_thread` and returns at once: android_main never waits for the
//! Java main thread (module docs of [`crate::jni_env`]). What the call answered reaches `done` on
//! the Java main thread, for the log. Nothing here is pure, so on other targets the module is
//! empty; the Back cascade that calls it is tested in `lightcraft-ui-egui` with a fake service.

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
