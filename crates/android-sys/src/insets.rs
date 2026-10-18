//! The window insets the app must keep clear of: system bars, display cutout and soft keyboard
//! (APK-31).
//!
//! With targetSdk 36 the content view is edge-to-edge, so the window reaches under the status bar,
//! the cutout and the gesture bar, and the keyboard covers the bottom. `request_insets` posts
//! `decorView.getRootWindowInsets().getInsets(systemBars() | displayCutout() | ime())` (API 30+)
//! to the Java main thread (views belong to it) with `AndroidApp::run_on_java_main_thread` and
//! returns a channel at once; android_main polls it with `try_recv` once per frame and never
//! waits (module docs of [`crate::jni_env`]). [`Insets`] is pure and tested on every target.

/// Insets in physical pixels, one per window edge. Never negative.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Insets {
    /// The insets as Java reported them (`left`, `top`, `right`, `bottom`); a negative value,
    /// which Android never reports, counts as none.
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Insets {
        Insets { left: left.max(0), top: top.max(0), right: right.max(0), bottom: bottom.max(0) }
    }

    /// No edge is covered.
    pub fn is_zero(&self) -> bool {
        *self == Insets::default()
    }
}

/// What the Java main thread answers: the insets, or why they could not be read.
pub type Answer = Result<Insets, crate::Error>;

/// Asks for the current insets. Posted to the Java main thread; the answer arrives on the
/// returned channel (poll with `try_recv`, never a blocking `recv`). A window that has no insets
/// yet (not attached) answers [`Insets::default`].
#[cfg(target_os = "android")]
pub fn request_insets(app: &android_activity::AndroidApp) -> std::sync::mpsc::Receiver<Answer> {
    let (tx, rx) = std::sync::mpsc::channel();
    let on_main = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let answer = crate::jni_env::with_activity(&on_main, read);
        // (a receiver that is gone just means nobody asks any more)
        let _ = tx.send(answer);
    }));
    rx
}

/// On the Java main thread: the three insets kinds merged in one `getInsets` call.
#[cfg(target_os = "android")]
fn read(env: &mut jni::Env<'_>, activity: &jni::objects::JObject<'_>) -> Result<Insets, crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    let answer = env.call_method(activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[]);
    let window = caught(env, answer)?.l()?;
    let answer = env.call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[]);
    let decor = caught(env, answer)?.l()?;
    let answer = env.call_method(&decor, jni_str!("getRootWindowInsets"), jni_sig!("()Landroid/view/WindowInsets;"), &[]);
    let root = caught(env, answer)?.l()?;
    if root.is_null() {
        return Ok(Insets::default());
    }
    let class = jni_str!("android/view/WindowInsets$Type");
    let mut mask = 0;
    for kind in [jni_str!("systemBars"), jni_str!("displayCutout"), jni_str!("ime")] {
        let answer = env.call_static_method(class, kind, jni_sig!("()I"), &[]);
        mask |= caught(env, answer)?.i()?;
    }
    let answer = env.call_method(&root, jni_str!("getInsets"), jni_sig!("(I)Landroid/graphics/Insets;"), &[JValue::Int(mask)]);
    let insets = caught(env, answer)?.l()?;
    let mut edges = [0; 4];
    for (slot, name) in edges.iter_mut().zip([jni_str!("left"), jni_str!("top"), jni_str!("right"), jni_str!("bottom")]) {
        let answer = env.get_field(&insets, name, jni_sig!("I"));
        *slot = caught(env, answer)?.i()?;
    }
    let [left, top, right, bottom] = edges;
    Ok(Insets::new(left, top, right, bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insets_never_go_negative() {
        assert_eq!(Insets::new(-3, 24, 0, -1), Insets { left: 0, top: 24, right: 0, bottom: 0 });
        assert!(Insets::default().is_zero());
        assert!(!Insets::new(0, 1, 0, 0).is_zero());
    }
}
