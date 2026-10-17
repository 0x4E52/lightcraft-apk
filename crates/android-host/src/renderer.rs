//! The window's renderer: which one starts, what a start that failed tells the user, and the wait
//! for the user to read it.
//!
//! A GPU that won't start (an error, a panic or a crash inside the driver) must not leave a black
//! screen or a silent exit, and the next start must not repeat it:
//!
//! - **Order** ([`order`]): Vulkan, then GL (wgpu's GLES backend). Before eframe starts, `run`
//!   writes `<config>/window-init.marker` naming the renderer (`gpu_markers::Files::arm`); eframe's
//!   app creator (it runs once the renderer is up) removes it. A marker left behind names the
//!   renderer the last start died in, which then goes last.
//! - **Why it ended** ([`exit_reason`]): eframe only logs a renderer error and leaves its loop
//!   (its `return_result` is private), so a start that failed is "the app creator never ran"
//!   plus the last error-level record (`host::log_file::last_error`, which also holds eframe's
//!   "Exiting because of error"). A loop the host left on purpose (the Activity went away) shows
//!   nothing.
//! - **The message** ([`failure_text`]) goes into an `AlertDialog` (`android-sys` `dialog.rs`).
//!   Returning from `android_main` finishes the Activity and the dialog with it, so android_main
//!   first waits ([`wait_until`]) until the user closed it, keeping the Activity's lifecycle
//!   commands answered meanwhile (no ANR); then `run` returns `Err` and the process exits, so the
//!   next tap on the icon starts afresh with the other renderer.
//!
//! All of it is pure and tested on every target; `run` (Android only) wires it to eframe, the
//! marker files and the dialog. `compiled` is a parameter: `cargo test -p lightcraft-android-host`
//! alone builds wgpu without `gles` on Linux, so reading it here would drop GL from the tests.

use std::path::Path;
use std::time::{Duration, Instant};

use lightcraft_engine::gpu::backend::{self, Backends, WindowRenderer};

/// How the Android user forces another graphics backend, for the GPU crash notice
/// (`gpu_markers::gpu_crash_notice` completes "to try another graphics backend, …").
pub const HOW_TO_FORCE: &str = "a developer can run adb shell setprop debug.lightcraft.gpu_backend gl (or vulkan) and start LightCraft again";

/// How long android_main waits at most for the failure dialog to be closed.
pub const WAIT_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How often the wait asks whether the dialog is still showing (and how long each poll for the
/// Activity's lifecycle commands blocks).
pub const WAIT_TICK: Duration = Duration::from_millis(200);

/// The window's renderers in the order to try them on Android: `forced` first (the system property
/// `debug.lightcraft.gpu_backend`, APK-20, unless the last start died in it), then Vulkan and GL, the one the last start died in (`failed`) last. Only the backends
/// in `compiled` (`wgpu::Instance::enabled_backend_features()`); eframe's glow renderer is not in
/// the Android build.
pub fn order(forced: Option<WindowRenderer>, failed: Option<WindowRenderer>, compiled: Backends) -> Vec<WindowRenderer> {
    // a forced renderer that was the last start's death goes last like any other: forcing it
    // must not trap the user in the failure (the property stays set until a developer clears it)
    let forced = forced.filter(|f| Some(*f) != failed);
    // (no remembered renderer: that is Windows-only on the desktop too)
    backend::window_renderer_order(forced, None, failed, "android", compiled, false)
}

/// The wgpu backends eframe's instance gets for `renderer` (glow, which Android doesn't build,
/// would leave the platform's own).
pub fn backends_of(renderer: WindowRenderer) -> Backends {
    match renderer {
        WindowRenderer::Wgpu(b) => b,
        WindowRenderer::Glow => backend::default_window_backends("android"),
    }
}

/// What eframe logs when it leaves its loop on an error (eframe `native/run.rs`, `handle_event_result`).
const EFRAME_EXITING: &str = "Exiting because of error";

/// Why the event loop ended, when the user must be told (`None`: nothing to show).
///
/// - `exit_requested`: the host left the loop itself (the Activity went away): never a failure.
/// - `!app_created`: eframe's app creator never ran, so the window's renderer did not come up:
///   the reason is the last error-level record, or a plain sentence when nothing was logged.
/// - the app ran and eframe left the loop on an error (`last_error` is eframe's "Exiting because
///   of error"): that error. Any other `last_error` is an older, unrelated record.
pub fn exit_reason(app_created: bool, exit_requested: bool, last_error: Option<&str>) -> Option<String> {
    if exit_requested {
        return None;
    }
    if !app_created {
        return Some(match last_error.map(str::trim).filter(|e| !e.is_empty()) {
            Some(e) => format!("LightCraft's graphics could not start: {e}"),
            None => "LightCraft's graphics could not start, and no error was recorded.".to_owned(),
        });
    }
    let e = last_error?;
    e.contains(EFRAME_EXITING).then(|| format!("LightCraft stopped: {}", e.trim()))
}

/// Why `run` stopped on a caught panic (`what`: the panic's message): during the renderer's start
/// when the app was never created, else while LightCraft ran.
pub fn panic_reason(app_created: bool, what: &str) -> String {
    if app_created {
        format!("LightCraft stopped after an internal error: {what}")
    } else {
        format!("LightCraft's graphics could not start (an internal error: {what})")
    }
}

/// The failure dialog's text: `reason`; when the window never came up (`retry_with`: the renderer
/// the next start tries first), that the next start tries it; and where the log is.
pub fn failure_text(reason: &str, retry_with: Option<WindowRenderer>, log: Option<&Path>) -> String {
    let mut text = reason.trim().to_owned();
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    if let Some(next) = retry_with {
        text.push_str(&format!("\n\nOpen LightCraft again: it then starts with the {} renderer.", label(next)));
    }
    match log {
        Some(path) => text.push_str(&format!("\n\nDetails are in the log: {}", path.display())),
        None => text.push_str("\n\nDetails are in the system log (adb logcat)."),
    }
    text
}

/// A renderer's name for people.
fn label(r: WindowRenderer) -> &'static str {
    match r {
        WindowRenderer::Wgpu(b) if b == Backends::VULKAN => "Vulkan",
        WindowRenderer::Wgpu(b) if b == Backends::GL => "OpenGL ES",
        other => other.name(),
    }
}

/// How the wait for the failure dialog ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waited {
    /// The dialog was closed (or the Activity destroyed).
    Dismissed,
    /// It was still open after the limit.
    TimedOut,
    /// Asking about it failed (a JNI error): the wait ended at once.
    Failed(String),
}

/// Calls `dismissed` until it answers `Ok(true)`, at most until `limit` has passed and never more
/// often than once per `tick` (it may block for up to a tick itself, polling the Activity's
/// commands); an `Err` ends the wait at once.
pub fn wait_until(mut dismissed: impl FnMut() -> Result<bool, String>, tick: Duration, limit: Duration) -> Waited {
    let start = Instant::now();
    loop {
        let asked = Instant::now();
        match dismissed() {
            Ok(true) => return Waited::Dismissed,
            Err(e) => return Waited::Failed(e),
            Ok(false) => {}
        }
        if start.elapsed() >= limit {
            return Waited::TimedOut;
        }
        // (a poll that returned early, on an event, doesn't make this a busy loop)
        let spent = asked.elapsed();
        if spent < tick {
            std::thread::sleep(tick.saturating_sub(spent));
        }
    }
}

/// eframe's wgpu set-up for `renderer`: its backends only (as the desktop's non-Windows path), with
/// the app's backend options and instance flags.
#[cfg(target_os = "android")]
pub fn wgpu_options(renderer: WindowRenderer) -> eframe::egui_wgpu::WgpuConfiguration {
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(n) = &mut options.wgpu_setup {
        n.instance_descriptor.backends = backends_of(renderer);
        n.instance_descriptor.backend_options = backend::backend_options();
        n.instance_descriptor.flags = backend::instance_flags_from_env(n.instance_descriptor.flags);
    }
    options
}

/// Shows `text` in an `AlertDialog` over the Activity and waits, on android_main, until the user
/// closed it (or the Activity is destroyed, a JNI call failed, or [`WAIT_LIMIT`] passed). Meanwhile
/// the Activity's commands are polled every [`WAIT_TICK`], so Pause, Stop and the surface going
/// away are answered (no ANR), and input that reaches the Activity is let go.
#[cfg(target_os = "android")]
pub fn show_failure_and_wait(app: &android_activity::AndroidApp, text: &str) {
    use android_activity::{InputStatus, MainEvent, PollEvent};
    use lightcraft_android_sys::dialog::{self, Status};
    let alert = dialog::show_alert(app, "LightCraft", text, "OK");
    let waited = wait_until(
        || {
            let (mut destroyed, mut input) = (false, false);
            let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                app.poll_events(Some(WAIT_TICK), |event| match event {
                    PollEvent::Main(MainEvent::Destroy) => destroyed = true,
                    PollEvent::Main(MainEvent::InputAvailable) => input = true,
                    _ => {}
                })
            }));
            if polled.is_err() {
                return Err("polling the Activity's events failed".to_owned());
            }
            if destroyed {
                return Ok(true);
            }
            if input && let Ok(mut events) = app.input_events_iter() {
                while events.next(|_| InputStatus::Unhandled) {}
            }
            match alert.status(app) {
                Status::Pending | Status::Showing => Ok(false),
                Status::Dismissed => Ok(true),
                Status::Failed(e) => Err(e.to_string()),
            }
        },
        WAIT_TICK,
        WAIT_LIMIT,
    );
    match waited {
        Waited::Dismissed => log::info!("the failure dialog was closed"),
        Waited::TimedOut => log::warn!("the failure dialog was still open after {} minutes", WAIT_LIMIT.as_secs() / 60),
        Waited::Failed(e) => log::error!("the failure dialog: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VULKAN: WindowRenderer = WindowRenderer::Wgpu(Backends::VULKAN);
    const GL: WindowRenderer = WindowRenderer::Wgpu(Backends::GL);

    /// Vulkan first on a fresh start; a Vulkan start that died (its marker left behind) puts GL
    /// first next time, Vulkan last. `compiled` is passed: the test build may lack `gles`.
    #[test]
    fn failed_vulkan_start_tries_gl_next() {
        let compiled = Backends::VULKAN | Backends::GL;
        assert_eq!(order(None, None, compiled), [VULKAN, GL]);
        assert_eq!(order(None, Some(VULKAN), compiled), [GL, VULKAN]);
        assert_eq!(order(None, Some(GL), compiled), [VULKAN, GL]);
        // and through the marker file, as `run` reads it
        let dir = std::env::temp_dir().join(format!("lc-android-renderer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = lightcraft_ui_egui::host::gpu_markers::Files::new(Some(dir.clone()));
        files.arm(VULKAN);
        let next = order(None, files.take_failed(), compiled);
        assert_eq!(next.first(), Some(&GL));
        assert_eq!(backends_of(next[0]), Backends::GL);
        assert_eq!(files.take_failed(), None, "the marker is read once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `debug.lightcraft.gpu_backend` goes first, whichever renderer that is; once a start with it
    /// died (its marker is left), the other renderer is tried, though the property is still set.
    #[test]
    fn forced_backend_from_props_goes_first() {
        use lightcraft_android_sys::props::{GPU_BACKEND, settings_from};
        let compiled = Backends::VULKAN | Backends::GL;
        let forced = |value: &'static str| {
            settings_from(|name| (name == GPU_BACKEND).then(|| value.to_owned())).gpu_backend.as_deref().and_then(WindowRenderer::parse)
        };
        assert_eq!(order(forced("gl"), None, compiled), [GL, VULKAN]);
        assert_eq!(order(forced("vulkan"), None, compiled), [VULKAN, GL]);
        // not a renderer name (a list, `auto`, `off`, a typo): the usual order
        for other in ["vulkan,gl", "auto", "off", "vulcan"] {
            assert_eq!(order(forced(other), None, compiled), [VULKAN, GL], "{other}");
        }
        // a forced renderer the last start died in is not tried first again
        assert_eq!(order(forced("vulkan"), Some(VULKAN), compiled), [GL, VULKAN]);
        assert_eq!(order(forced("gl"), Some(VULKAN), compiled), [GL, VULKAN]);
        // one this build lacks is skipped
        assert_eq!(order(forced("gl"), None, Backends::VULKAN), [VULKAN]);
    }

    /// Never glow (not built for Android), never a backend this build lacks, never empty.
    #[test]
    fn order_keeps_to_compiled_backends() {
        assert_eq!(order(None, Some(VULKAN), Backends::VULKAN), [VULKAN]);
        assert_eq!(order(Some(WindowRenderer::Glow), None, Backends::VULKAN | Backends::GL), [VULKAN, GL]);
        assert_eq!(order(None, None, Backends::empty()).len(), 1);
        assert_eq!(backends_of(WindowRenderer::Glow), backend::default_window_backends("android"));
    }

    /// The window never came up: the last logged error is the reason (eframe's "Exiting because
    /// of error" included), or a plain sentence when nothing was logged; the dialog names the
    /// renderer the next start tries and the log.
    #[test]
    fn exit_without_app_shows_last_error() {
        let error = "eframe::native::run: Exiting because of error: Failed to create wgpu surface: Vulkan unsupported";
        let reason = exit_reason(false, false, Some(error)).unwrap();
        assert!(reason.contains("could not start") && reason.contains("Vulkan unsupported"), "{reason}");
        let nothing = exit_reason(false, false, None).unwrap();
        assert!(nothing.contains("no error was recorded"), "{nothing}");
        assert_eq!(exit_reason(false, false, Some("  ")), Some(nothing));
        let text = failure_text(&reason, Some(GL), Some(Path::new("/data/user/0/x/files/logs/lightcraft.log")));
        assert!(text.contains("starts with the OpenGL ES renderer"), "{text}");
        assert!(text.ends_with("Details are in the log: /data/user/0/x/files/logs/lightcraft.log"), "{text}");
        assert!(failure_text("Stopped", None, None).starts_with("Stopped.\n\nDetails are in the system log"));
        // a panic in the renderer's start reads the same way
        assert!(panic_reason(false, "device lost").contains("graphics could not start"));
    }

    /// The host left the loop itself, or LightCraft ran and closed without eframe failing: nothing
    /// to show, whatever older error was logged. An eframe error after the app ran is shown.
    #[test]
    fn normal_exit_shows_nothing() {
        assert_eq!(exit_reason(true, true, None), None);
        assert_eq!(exit_reason(true, true, Some("eframe::native::run: Exiting because of error: lost")), None);
        assert_eq!(exit_reason(false, true, Some("anything")), None, "the Activity went away before the window came up");
        assert_eq!(exit_reason(true, false, None), None);
        assert_eq!(exit_reason(true, false, Some("lightcraft_android_host::state: can't open library /x: busy")), None);
        let stopped = exit_reason(true, false, Some("eframe::native::run: Exiting because of error: Surface lost")).unwrap();
        assert!(stopped.starts_with("LightCraft stopped:") && stopped.contains("Surface lost"), "{stopped}");
        assert!(panic_reason(true, "boom").contains("stopped after an internal error: boom"));
    }

    /// The wait asks until the dialog is closed, then returns at once.
    #[test]
    fn waits_for_dismissal_then_returns() {
        let mut asked = 0;
        let waited = wait_until(
            || {
                asked += 1;
                Ok(asked >= 4)
            },
            Duration::from_millis(1),
            Duration::from_secs(60),
        );
        assert_eq!(waited, Waited::Dismissed);
        assert_eq!(asked, 4);
        // a dialog nobody closes: the limit ends the wait
        let mut open = 0;
        let waited = wait_until(
            || {
                open += 1;
                Ok(false)
            },
            Duration::from_millis(1),
            Duration::from_millis(20),
        );
        assert_eq!(waited, Waited::TimedOut);
        assert!(open >= 2, "{open}");
    }

    /// A JNI error ends the wait at once, with the error for the log.
    #[test]
    fn jni_error_ends_wait() {
        let mut asked = 0;
        let waited = wait_until(
            || {
                asked += 1;
                Err("JNI: java.lang.IllegalStateException".to_owned())
            },
            Duration::from_secs(1),
            Duration::from_secs(600),
        );
        assert_eq!(waited, Waited::Failed("JNI: java.lang.IllegalStateException".into()));
        assert_eq!(asked, 1);
    }
}
