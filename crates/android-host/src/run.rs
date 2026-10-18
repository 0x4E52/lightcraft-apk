//! Start guard, start-up and the event loop.
//!
//! winit builds one `EventLoop` per process (winit 0.30 event_loop.rs:118-133), and Android often
//! keeps a process cached after its Activity finished, so `android_main` can run a second time in
//! a process whose event loop is spent. The start guard refuses that second start with an error;
//! the entry crate exits the process whenever `run` returns, so the next launch starts fresh and
//! the guard is only a backstop.

use std::sync::atomic::{AtomicBool, Ordering};

/// Why `run` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The Activity was destroyed or is finishing (by the system, or after the user removed it
    /// from recents): the lifecycle wrapper noticed and left the event loop.
    Destroyed,
    /// The user closed LightCraft (File ▸ Quit or the window closing).
    Closed,
}

/// The process exit status for what `run` returned: 0 when the app ended normally, 1 when it
/// failed (a start error, a second start in the same process or a caught panic).
pub fn exit_code(result: &Result<Exit, String>) -> i32 {
    match result {
        Ok(Exit::Destroyed | Exit::Closed) => 0,
        Err(_) => 1,
    }
}

/// The error [`try_start`] returns for a second start in the same process.
pub const REOPENED: &str = "LightCraft was reopened in the same process; it closes now so that the next start begins afresh. Open it again.";

/// Marks the process as started: `Ok` the first time, `Err(REOPENED)` on every later call. Pure
/// over the given flag so tests never touch the process-wide one.
pub fn try_start(started: &AtomicBool) -> Result<(), String> {
    if started.swap(true, Ordering::AcqRel) { Err(REOPENED.to_owned()) } else { Ok(()) }
}

/// Set by the first `run` of this process.
#[cfg(target_os = "android")]
static STARTED: AtomicBool = AtomicBool::new(false);

/// Runs LightCraft in this process's Activity until it ends. A second call in the same process
/// returns `Err(REOPENED)` at once.
///
/// Everything before eframe's event loop polls for the first time blocks the Java main thread
/// (onStart/onResume, the input queue and surfaceCreated wait for android_main), so only quick
/// work happens here: the logger, the folders, the memory budget, the panic hook and the settings
/// file. The library opens on a worker ([`crate::loader`]) behind a loading frame.
///
/// eframe runs inside the host's lifecycle wrapper (`lifecycle.rs`) on an event loop built here,
/// so focus loss saves, the background trims, and a destroyed Activity ends the loop:
/// `Ok(Exit::Destroyed)` then, `Ok(Exit::Closed)` for any other end. A start error (no internal
/// storage folder, a folder that cannot be created) is an `Err` returned before any window
/// exists: the entry crate logs it (logcat) and exits.
///
/// The window's renderer ([`crate::renderer`]): Vulkan, then GL, the one the last start died in
/// last; its marker is armed before eframe starts and cleared by the app creator. A renderer that
/// fails (an error eframe only logs, or a panic, caught here) or an event loop that fails gets an
/// `AlertDialog` saying why and where the log is; `run` waits until it is closed, then returns
/// `Err`, so the process exits and the next start tries the other renderer. The GPU crash
/// sentinels of the desktop (`host::gpu_markers`) turn GPU compute off after a start that died
/// creating the device or a GPU lost while running, with a notice.
#[cfg(target_os = "android")]
pub fn run(app: android_activity::AndroidApp) -> Result<Exit, String> {
    use lightcraft_ui_egui::host::{gpu_markers, log_file, prefs};

    try_start(&STARTED)?;
    // the diagnostics switches (`adb shell setprop debug.lightcraft.*`) stand in for the
    // environment variables an app does not get: they are applied before any record is logged
    // or any switch is read
    let diag = lightcraft_android_sys::props::read();
    lightcraft_profile::set_enabled(diag.profile);
    let gpu_choice = diag.gpu_backend.as_deref().and_then(lightcraft_engine::gpu::backend::parse_backends);
    let gpu_choice_in_force = gpu_choice.map(lightcraft_engine::gpu::backend::set_choice);
    // stderr reaches logcat (android-activity forwards it); the file follows once its folder exists
    let logger = log_file::install_with(&log_file::filter_spec(diag.log.as_deref(), None));
    log::info!("LightCraft {} started", env!("CARGO_PKG_VERSION"));
    if diag != lightcraft_android_sys::props::DiagSettings::default() {
        log::info!("diagnostics from system properties: {diag:?}");
    }
    if let Some(Err(in_use)) = gpu_choice_in_force {
        log::warn!("graphics backend choice already fixed ({in_use:?}); debug.lightcraft.gpu_backend ignored");
    }
    let files_dir = crate::paths::internal_dir(app.internal_data_path())?;
    let paths = crate::paths::Paths::from_internal(&files_dir);
    paths.create()?;
    // the debug control server: only with `debug.lightcraft.control`, and only with a token
    let control = crate::control::prepare(diag.control, &crate::control::token_path(&files_dir), crate::control::random_bytes);
    if let Err(in_use) = lightcraft_engine::config::use_dir(paths.config.clone()) {
        log::warn!("settings folder already set to {}", in_use.display());
    }
    let log_path = logger.and_then(|logger| match logger.attach_dir(&paths.logs) {
        Ok(path) => Some(path),
        Err(e) => {
            log::warn!("no log file: {e}");
            None
        }
    });
    lightcraft_engine::guard::install_hook(paths.panic_log());
    if let Some(logger) = logger {
        log_file::record_panics(logger);
    }

    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    // the phone budget is "automatic" too (Settings ▸ Performance ▸ Memory, the bug report)
    if let Err(in_use) = lightcraft_engine::memory::set_default_budget(crate::memory::budget_for(&meminfo)) {
        log::warn!("automatic memory budget already set to {} MiB", in_use >> 20);
    }
    // messages that name the machine say "this device"
    lightcraft_engine::memory::set_handheld();
    let budget = lightcraft_engine::memory::set_budget(lightcraft_engine::memory::default_budget());
    let total = crate::memory::mem_total(&meminfo).and_then(|b| usize::try_from(b).ok());
    if let Some(total) = total
        && let Err(in_use) = lightcraft_engine::memory::set_total_ram(total)
    {
        log::warn!("device memory already set to {in_use} bytes");
    }
    log::info!("memory budget {} MiB of {} MiB", budget >> 20, total.map_or(0, |t| t >> 20));

    let (saved_ui, prefs_warning, keep_prefs_file) = prefs::load_prefs(Some(&paths.config), false);
    if let Some(ui) = &saved_ui {
        lightcraft_ui_egui::i18n::set_language(ui.language);
    }
    // GPU compute: off if the preference says so, or if the last start died creating the device
    // or stopped after the GPU was lost — before anything can create it
    let gpu_notice = gpu_markers::gpu_crash_check_at(gpu_markers::gpu_marker_path(Some(&paths.config)), false, crate::renderer::HOW_TO_FORCE)
        .or_else(|| gpu_markers::gpu_lost_check_at(gpu_markers::gpu_lost_marker_path(Some(&paths.config)).as_deref(), false));
    lightcraft_engine::gpu::set_enabled(saved_ui.as_ref().is_none_or(|u| u.settings.gpu) && gpu_notice.is_none());
    // the window's renderer: Vulkan, then GL; the one the last start died in goes last
    let files = std::rc::Rc::new(gpu_markers::Files::new(Some(paths.config.clone())));
    let failed = files.take_failed();
    let compiled = eframe::wgpu::Instance::enabled_backend_features();
    let forced = diag.gpu_backend.as_deref().and_then(lightcraft_engine::gpu::backend::WindowRenderer::parse);
    let order = crate::renderer::order(forced, failed, compiled);
    let Some(&renderer) = order.first() else { return Err("LightCraft has no graphics renderer to start".to_owned()) };
    log::info!("window: renderer {} (order {:?})", renderer.name(), order.iter().map(|r| r.name()).collect::<Vec<_>>());
    let renderer_notice =
        failed.map(|f| format!("The window could not start with its {} renderer last time, so it now uses {}.", f.name(), renderer.name()));
    let reopen = crate::state::Reopen::default();
    let start = crate::state::Start {
        prefs: saved_ui,
        prefs_warning,
        gpu_notice,
        renderer_notice,
        keep_prefs_file,
        prefs_path: prefs::prefs_path(Some(&paths.config)),
        log_file: log_path.clone(),
        library: paths.library.clone(),
        reopen: Some(reopen.clone()),
        move_to_back: Some(move_to_back(&app)),
    };
    let loader = crate::loader::open_library(paths.library.clone());

    // the renderer's start: a panic is caught here, so the dialog below can still say why
    let app_created = std::rc::Rc::new(std::cell::Cell::new(false));
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_eframe(&app, Eframe { renderer, files: files.clone(), app_created: app_created.clone(), loader, start, reopen, control })
    }));
    let app_created = app_created.get();
    let (result, reason) = match ran {
        Ok(Ok(left)) => {
            let reason = crate::renderer::exit_reason(app_created, left, log_file::last_error().as_deref());
            (Ok(if left { Exit::Destroyed } else { Exit::Closed }), reason)
        }
        Ok(Err(e)) => (Err(e.clone()), Some(e)),
        Err(panic) => {
            let reason = crate::renderer::panic_reason(app_created, &lightcraft_engine::guard::panic_message(&*panic));
            (Err(reason.clone()), Some(reason))
        }
    };
    if !app_created && reason.is_none() {
        // the Activity went away before the window came up: not the renderer's failure
        let _ = files.take_failed();
    }
    let Some(reason) = reason else { return result };
    log::error!("{reason}");
    if app_created && let Some(lost) = gpu_markers::gpu_lost_reason() {
        // the GPU was lost while LightCraft ran: the next start renders on the CPU and says why
        gpu_markers::record_gpu_lost(gpu_markers::gpu_lost_marker_path(Some(&paths.config)).as_deref(), &lost);
    }
    // the window never came up and its marker stays: the next start tries the next renderer
    let retry_with = (!app_created && files.armed())
        .then(|| crate::renderer::order(forced, Some(renderer), compiled).first().copied())
        .flatten()
        .filter(|next| *next != renderer);
    // returning finishes the Activity and the dialog with it: wait until it was read
    crate::renderer::show_failure_and_wait(&app, &crate::renderer::failure_text(&reason, retry_with, log_path.as_deref()));
    Err(reason)
}

/// What [`run_eframe`] starts eframe with.
#[cfg(target_os = "android")]
struct Eframe {
    renderer: lightcraft_engine::gpu::backend::WindowRenderer,
    files: std::rc::Rc<lightcraft_ui_egui::host::gpu_markers::Files>,
    /// Set by eframe's app creator, which runs once the window's renderer is up.
    app_created: std::rc::Rc<std::cell::Cell<bool>>,
    loader: crate::loader::Loader,
    start: crate::state::Start,
    reopen: crate::state::Reopen,
    /// The debug control server to start with the window (`None`: off).
    control: Option<crate::control::Server>,
}

/// Builds the event loop, starts eframe on `renderer` (its marker armed first) and runs the loop
/// inside the lifecycle wrapper until it ends: `Ok(true)` when the host left it because the
/// Activity went away, `Ok(false)` for any other end, `Err` when the event loop failed.
#[cfg(target_os = "android")]
fn run_eframe(app: &android_activity::AndroidApp, e: Eframe) -> Result<bool, String> {
    let Eframe { renderer, files, app_created, loader, start, reopen, control } = e;
    // our own event loop, so the lifecycle wrapper sees every callback (eframe's `run_native`
    // would build and run it out of reach)
    use winit::platform::android::EventLoopBuilderExtAndroid as _;
    let event_loop = winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event()
        .with_android_app(app.clone())
        .build()
        .map_err(|e| format!("LightCraft could not start its event loop: {e}"))?;
    let insets_dirty = crate::insets::Dirty::default();
    let slot: crate::app::Slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let options = eframe::NativeOptions {
        android_app: Some(app.clone()),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: crate::renderer::wgpu_options(renderer),
        ..Default::default()
    };
    let app_slot = slot.clone();
    let insets = crate::insets::Tracker::new(
        {
            let app = app.clone();
            Box::new(move || lightcraft_android_sys::insets::request_insets(&app))
        },
        insets_dirty.clone(),
    );
    files.arm(renderer);
    let eframe_app = eframe::create_native(
        "LightCraft",
        options,
        Box::new(move |cc| {
            // the window's renderer is up (eframe calls this after creating it)
            files.started(renderer);
            app_created.set(true);
            if let Some(render) = cc.wgpu_render_state.as_ref() {
                // log wgpu errors outside an error scope instead of wgpu's default panic
                lightcraft_engine::gpu::window::install_error_handler(&render.device);
                let info = render.adapter.get_info();
                log::info!("window: adapter {} ({:?}, {:?}), driver {} {}", info.name, info.backend, info.device_type, info.driver, info.driver_info);
            }
            // egui's own Ctrl+Q would close the window and end the one event loop, the loading
            // frame included (the app turns it off again once it runs: `Services::quit_disabled`)
            cc.egui_ctx.options_mut(|o| o.quit_shortcuts.clear());
            let control_rx =
                control.map(|server| lightcraft_ui_egui::host::control_server::start(server.port, cc.egui_ctx.clone(), Some(server.token)));
            Ok(Box::new(crate::app::HostApp::new(app_slot, loader, start, reopen, control_rx, insets)))
        }),
        &event_loop,
    );
    let core = crate::lifecycle_core::CoreState::new(activity_probe(app));
    let mut handler = crate::lifecycle::Lifecycle::new(eframe_app, slot, core, insets_dirty);
    event_loop.run_app(&mut handler).map_err(|e| format!("LightCraft's event loop failed: {e}"))?;
    Ok(handler.left())
}

/// Asks the Activity whether it is finishing or destroyed, on the calling thread (android_main).
#[cfg(target_os = "android")]
fn activity_probe(app: &android_activity::AndroidApp) -> crate::lifecycle_core::Probe {
    let app = app.clone();
    Box::new(move || lightcraft_android_sys::activity::activity_state(&app).map_err(|e| e.to_string()))
}

/// Back at the root: `Activity.moveTaskToBack(true)`, posted to the Java main thread and not
/// awaited; what it answered is logged there.
#[cfg(target_os = "android")]
fn move_to_back(app: &android_activity::AndroidApp) -> crate::services::MoveToBack {
    let app = app.clone();
    std::rc::Rc::new(move || {
        lightcraft_android_sys::activity::move_task_to_back(&app, |moved| match moved {
            Ok(true) => {}
            Ok(false) => log::warn!("Back: Android did not move LightCraft to the background"),
            Err(e) => log::warn!("Back: {e}"),
        });
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_start_is_refused() {
        let started = AtomicBool::new(false);
        assert_eq!(try_start(&started), Ok(()));
        let again = try_start(&started).unwrap_err();
        assert_eq!(again, REOPENED);
        assert!(again.contains("reopened in the same process"));
        // still refused: the guard never resets within a process
        assert!(try_start(&started).is_err());
    }

    #[test]
    fn exit_code_for_each_outcome() {
        assert_eq!(exit_code(&Ok(Exit::Destroyed)), 0);
        assert_eq!(exit_code(&Ok(Exit::Closed)), 0);
        assert_eq!(exit_code(&Err(REOPENED.to_owned())), 1);
        assert_eq!(exit_code(&Err("LightCraft stopped after an internal error".into())), 1);
    }
}
