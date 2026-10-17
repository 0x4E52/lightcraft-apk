//! LightCraft for Android: the `android_main` symbol android-activity calls. Empty elsewhere.
//!
//! Whenever the host's `run` returns (Activity destroyed, app closed, start error, caught panic)
//! the process exits: winit builds one event loop per process and Android keeps a finished app's
//! process cached, so the next tap on the icon would hit the start guard and seem to do nothing.
//! This is the one exception to android-activity's advice not to exit the process.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Called by android-activity on a fresh thread, once per Activity start.
#[cfg(target_os = "android")]
// SAFETY: android-activity declares `extern "Rust" { fn android_main(app: AndroidApp); }`
// (native_activity/glue.rs:663) and calls it with exactly this signature and ABI; this crate is the
// only definition of the symbol in the process.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lightcraft_android_host::run(app)));
    let result = caught.unwrap_or_else(|_| Err("LightCraft stopped after an internal error".to_owned()));
    match &result {
        Ok(exit) => log::info!("LightCraft ends: {exit:?}"),
        Err(e) => log::error!("LightCraft ends: {e}"),
    }
    log::logger().flush();
    std::process::exit(lightcraft_android_host::exit_code(&result));
}
