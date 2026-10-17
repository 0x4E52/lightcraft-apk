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
    /// The Activity was destroyed (by the system, or after the user removed it from recents).
    /// Not produced yet: eframe's loop ends the same way for both, so `run` returns
    /// [`Exit::Closed`]; APK-18's lifecycle wrapper, which sees the destroy, maps it here.
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
/// Every normal end is `Ok(Exit::Closed)` until APK-18 tells a destroy apart. A start error
/// (no internal storage folder, a folder that cannot be created) is an `Err` returned before any
/// window exists: the entry crate logs it (logcat) and exits.
#[cfg(target_os = "android")]
pub fn run(app: android_activity::AndroidApp) -> Result<Exit, String> {
    use lightcraft_ui_egui::host::{log_file, prefs, session_open};

    try_start(&STARTED)?;
    // stderr reaches logcat (android-activity forwards it); the file follows once its folder exists
    let logger = log_file::install_with(&log_file::filter_spec(None, None));
    log::info!("LightCraft {} started", env!("CARGO_PKG_VERSION"));
    let paths = crate::paths::Paths::from_internal(&crate::paths::internal_dir(app.internal_data_path())?);
    paths.create()?;
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
    lightcraft_engine::gpu::set_enabled(saved_ui.as_ref().is_none_or(|u| u.settings.gpu));
    let start = crate::state::Start {
        prefs: saved_ui,
        prefs_warning,
        keep_prefs_file,
        prefs_path: prefs::prefs_path(Some(&paths.config)),
        log_file: log_path,
        library: paths.library.clone(),
    };
    let library = paths.library.clone();
    let loader = crate::loader::Loader::start(move || {
        // a new library starts with the procedural demo photos
        let (session, problem) = session_open::open_session(false, Some(library), true);
        (session.with_default_sam3_model(), problem)
    });

    let slot: crate::app::Slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let options = eframe::NativeOptions { android_app: Some(app.clone()), renderer: eframe::Renderer::Wgpu, ..Default::default() };
    let app_slot = slot.clone();
    eframe::run_native("LightCraft", options, Box::new(move |_cc| Ok(Box::new(crate::app::HostApp::new(app_slot, loader, start)))))
        .map(|()| Exit::Closed)
        .map_err(|e| format!("LightCraft could not open its window: {e}"))
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
