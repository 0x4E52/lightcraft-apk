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
/// For now the loop only drains input (unanswered input ends in an ANR) and waits for the
/// Activity's destroy; APK-11 replaces it with eframe's event loop and the UI.
#[cfg(target_os = "android")]
pub fn run(app: android_activity::AndroidApp) -> Result<Exit, String> {
    use android_activity::{InputStatus, MainEvent, PollEvent};

    try_start(&STARTED)?;
    log::info!("LightCraft {} started", env!("CARGO_PKG_VERSION"));
    loop {
        let (mut destroyed, mut input) = (false, false);
        app.poll_events(None, |event| match event {
            PollEvent::Main(MainEvent::Destroy) => destroyed = true,
            PollEvent::Main(MainEvent::InputAvailable) => input = true,
            _ => {}
        });
        if input && let Ok(mut events) = app.input_events_iter() {
            while events.next(|_| InputStatus::Unhandled) {}
        }
        if destroyed {
            return Ok(Exit::Destroyed);
        }
    }
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
