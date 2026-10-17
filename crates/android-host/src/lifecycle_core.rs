//! What the Android host does when the app is left, comes back, runs short of memory or is
//! destroyed by the system. Pure: compiled and tested on every target; `lifecycle.rs` (Android
//! only) feeds it winit's callbacks.
//!
//! No eframe frame runs while the app is in the background (winit sends no redraws while
//! suspended), so the host saves and trims from the lifecycle callbacks themselves:
//! - **Focus lost** → [`Host::flush_leaving`]: everything to disk, an open slider drag committed.
//!   Focus changes don't make the Java main thread wait (android-activity only writes a
//!   command), and focus goes before the surface when the app is sent to the background.
//! - **Suspended** (the surface is gone) → [`Host::trim`] (memory only), and
//!   [`Host::flush_dirty`] only when no focus-loss flush ran since focus came back. `suspended`
//!   runs inside the surfaceDestroyed handshake: the Java main thread waits until winit's whole
//!   callback returns, so a full save there could hold it long enough for an ANR.
//! - **Memory warning** (`onLowMemory`; NativeActivity doesn't forward `onTrimMemory`) →
//!   [`Host::trim`].
//! - **About to wait**, while the app is away (suspended or without focus, until it is resumed or
//!   focused again) → ask the Activity whether it is destroyed or finishing ([`should_leave`]).
//!   winit swallows `onDestroy`, which keeps the Java main thread waiting until android_main
//!   returns: leaving the event loop is the only way the destroy can complete without an ANR.
//!
//! The Activity is asked through [`CoreState`]'s probe rather than the [`Host`], so a destroy
//! while the library is still opening (no host state yet, and none can arrive: the loading frame
//! needs frames, which a suspended app doesn't get) still ends the loop. Every other event before
//! the host state exists does nothing. A host state that is in use (a re-entrant callback) is
//! logged and the call skipped, never a panic.

use std::cell::RefCell;

pub use lightcraft_android_sys::activity::ActivityState;

/// A lifecycle callback, as the host sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEvent {
    /// The surface is back (winit `resumed`).
    Resumed,
    /// The surface is gone: the app went to the background (winit `suspended`).
    Suspended,
    /// The window lost focus (another app, the recents screen, a system dialog, Home).
    FocusLost,
    /// The window has focus again.
    FocusGained,
    /// The system is low on memory (winit `memory_warning`).
    MemoryWarning,
    /// The event loop is about to wait for the next event (the last callback of each iteration).
    AboutToWait,
}

/// What the event loop does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    /// Leave the event loop: the Activity is being destroyed.
    Exit,
}

/// What the lifecycle callbacks do to the app.
pub trait Host {
    /// Everything to disk now, an open interaction committed first (the app is being left).
    fn flush_leaving(&mut self) -> Result<(), String>;
    /// What changed to disk, an open interaction left open; quick.
    fn flush_dirty(&mut self) -> Result<(), String>;
    /// Let go of the memory caches that are cheap to fill again. Memory only, no I/O.
    fn trim(&mut self);
}

/// Asks the Activity whether it is finishing or destroyed (JNI on Android, a fake in tests).
pub type Probe = Box<dyn Fn() -> Result<ActivityState, String>>;

/// What the lifecycle remembers between callbacks.
pub struct CoreState {
    /// The surface is gone (since `Suspended`, until `Resumed`).
    suspended: bool,
    /// Suspended or without focus, since the last `Resumed` / `FocusGained`: each `AboutToWait`
    /// asks the Activity.
    away: bool,
    /// A focus-loss flush ran since focus last came back, so `Suspended` needn't save again.
    flushed_on_focus_loss: bool,
    /// `Exit` was answered once; every later event answers it again without asking.
    leaving: bool,
    probe: Probe,
}

impl CoreState {
    pub fn new(probe: Probe) -> CoreState {
        CoreState { suspended: false, away: false, flushed_on_focus_loss: false, leaving: false, probe }
    }

    /// Whether `Exit` was answered.
    pub fn leaving(&self) -> bool {
        self.leaving
    }
}

/// Whether the event loop must end for what the Activity says: it is destroyed (`onDestroy`
/// waits for android_main), or it is finishing and its surface is already gone (nothing is left
/// to show; `onDestroy` follows).
pub fn should_leave(state: ActivityState, suspended: bool) -> bool {
    state.destroyed || (state.finishing && suspended)
}

/// Run `f` on the host state when there is one and it is free; `true` when it ran.
fn with_host<H: Host>(slot: &RefCell<Option<H>>, what: &str, f: impl FnOnce(&mut H)) -> bool {
    match slot.try_borrow_mut() {
        Ok(mut slot) => match slot.as_mut() {
            Some(host) => {
                f(host);
                true
            }
            None => false,
        },
        Err(_) => {
            log::warn!("{what} skipped: the app state is in use");
            false
        }
    }
}

fn log_failure(result: Result<(), String>) {
    if let Err(e) = result {
        log::error!("{e}");
    }
}

/// Handle one lifecycle event; see the module docs for what each does. On `Exit` everything has
/// been saved (when the host state is there); the caller ends the event loop.
pub fn on_event<H: Host>(ev: HostEvent, slot: &RefCell<Option<H>>, core: &mut CoreState) -> Action {
    if core.leaving {
        return Action::Exit;
    }
    match ev {
        HostEvent::Resumed => {
            core.suspended = false;
            core.away = false;
        }
        HostEvent::FocusGained => {
            core.flushed_on_focus_loss = false;
            core.away = false;
        }
        HostEvent::FocusLost => {
            core.away = true;
            if with_host(slot, "saving on focus loss", |h| log_failure(h.flush_leaving())) {
                core.flushed_on_focus_loss = true;
            }
        }
        HostEvent::Suspended => {
            core.suspended = true;
            core.away = true;
            let flushed = core.flushed_on_focus_loss;
            with_host(slot, "trimming in the background", |h| {
                if !flushed {
                    log_failure(h.flush_dirty());
                }
                h.trim();
            });
        }
        HostEvent::MemoryWarning => {
            with_host(slot, "trimming on low memory", |h| h.trim());
        }
        HostEvent::AboutToWait => {
            if !core.away {
                return Action::Continue;
            }
            match (core.probe)() {
                Ok(state) if should_leave(state, core.suspended) => {
                    log::info!("the Activity is going away ({state:?}): leaving the event loop");
                    core.leaving = true;
                    with_host(slot, "saving before the end", |h| log_failure(h.flush_leaving()));
                    return Action::Exit;
                }
                Ok(_) => {}
                // (can't tell: stay; the next iteration asks again)
                Err(e) => log::warn!("can't ask the Activity whether it is ending: {e}"),
            }
        }
    }
    Action::Continue
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Records what the lifecycle asked of it.
    #[derive(Default)]
    struct Fake {
        calls: Vec<&'static str>,
    }

    impl Host for Fake {
        fn flush_leaving(&mut self) -> Result<(), String> {
            self.calls.push("flush_leaving");
            Ok(())
        }
        fn flush_dirty(&mut self) -> Result<(), String> {
            self.calls.push("flush_dirty");
            Ok(())
        }
        fn trim(&mut self) {
            self.calls.push("trim");
        }
    }

    /// A probe answering whatever `state` holds, counting the questions.
    fn probe(state: &Rc<Cell<Result<ActivityState, &'static str>>>, asked: &Rc<Cell<u32>>) -> Probe {
        let (state, asked) = (state.clone(), asked.clone());
        Box::new(move || {
            asked.set(asked.get() + 1);
            state.get().map_err(str::to_owned)
        })
    }

    struct Setup {
        slot: RefCell<Option<Fake>>,
        core: CoreState,
        state: Rc<Cell<Result<ActivityState, &'static str>>>,
        asked: Rc<Cell<u32>>,
    }

    fn setup(with_app: bool) -> Setup {
        let state = Rc::new(Cell::new(Ok(ActivityState::default())));
        let asked = Rc::new(Cell::new(0));
        let core = CoreState::new(probe(&state, &asked));
        Setup { slot: RefCell::new(with_app.then(Fake::default)), core, state, asked }
    }

    impl Setup {
        fn send(&mut self, ev: HostEvent) -> Action {
            on_event(ev, &self.slot, &mut self.core)
        }
        fn calls(&self) -> Vec<&'static str> {
            self.slot.borrow().as_ref().map(|f| f.calls.clone()).unwrap_or_default()
        }
    }

    #[test]
    fn core_flushes_on_focus_loss() {
        let mut s = setup(true);
        assert_eq!(s.send(HostEvent::FocusLost), Action::Continue);
        assert_eq!(s.calls(), ["flush_leaving"]);
        // focus back, lost again: saved again
        s.send(HostEvent::FocusGained);
        s.send(HostEvent::FocusLost);
        assert_eq!(s.calls(), ["flush_leaving", "flush_leaving"]);
    }

    #[test]
    fn suspend_after_focus_loss_does_not_reflush() {
        let mut s = setup(true);
        s.send(HostEvent::FocusLost);
        s.send(HostEvent::Suspended);
        assert_eq!(s.calls(), ["flush_leaving", "trim"], "the surface handshake only trims");
        // back to the foreground, then away again the same way
        s.send(HostEvent::Resumed);
        s.send(HostEvent::FocusGained);
        s.send(HostEvent::FocusLost);
        s.send(HostEvent::Suspended);
        assert_eq!(s.calls(), ["flush_leaving", "trim", "flush_leaving", "trim"]);
    }

    #[test]
    fn core_trims_when_backgrounded() {
        // the surface goes without a focus loss first: what changed is saved, quickly
        let mut s = setup(true);
        assert_eq!(s.send(HostEvent::Suspended), Action::Continue);
        assert_eq!(s.calls(), ["flush_dirty", "trim"]);
        // focus came back since the last focus-loss flush: the next suspend saves what changed
        let mut s = setup(true);
        s.send(HostEvent::FocusLost);
        s.send(HostEvent::FocusGained);
        s.send(HostEvent::Suspended);
        assert_eq!(s.calls(), ["flush_leaving", "flush_dirty", "trim"]);
    }

    #[test]
    fn core_maps_memory_warning_to_trim() {
        let mut s = setup(true);
        assert_eq!(s.send(HostEvent::MemoryWarning), Action::Continue);
        assert_eq!(s.calls(), ["trim"]);
        // in the foreground nothing asks the Activity
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Continue);
        assert_eq!(s.asked.get(), 0);
    }

    #[test]
    fn core_leaves_when_destroyed_while_suspended() {
        let mut s = setup(true);
        s.send(HostEvent::FocusLost);
        s.send(HostEvent::Suspended);
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Continue, "stopped, not destroyed");
        assert_eq!(s.asked.get(), 1);
        s.state.set(Ok(ActivityState { finishing: true, destroyed: true }));
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Exit);
        assert!(s.core.leaving());
        assert_eq!(s.calls(), ["flush_leaving", "trim", "flush_leaving"], "saved before the loop ends");
        // the loop may still deliver events before it stops: all answer Exit, nothing runs twice
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Exit);
        assert_eq!(s.send(HostEvent::Resumed), Action::Exit);
        assert_eq!(s.asked.get(), 2);
        assert_eq!(s.calls().len(), 3);

        // finishing (removed from recents, `finish()`) with the surface gone leaves too
        let mut s = setup(true);
        s.send(HostEvent::Suspended);
        s.state.set(Ok(ActivityState { finishing: true, destroyed: false }));
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Exit);
    }

    #[test]
    fn focus_loss_alone_keeps_loop() {
        let mut s = setup(true);
        s.send(HostEvent::FocusLost);
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Continue);
        assert_eq!(s.asked.get(), 1, "away: the Activity is asked");
        // finishing while the surface is still there: not yet
        s.state.set(Ok(ActivityState { finishing: true, destroyed: false }));
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Continue);
        // a JNI error counts as "stay"
        s.state.set(Err("JNI: no JVM"));
        s.send(HostEvent::Suspended);
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Continue);
        assert!(!s.core.leaving());
        // focus back: nothing is asked any more
        s.state.set(Ok(ActivityState::default()));
        s.send(HostEvent::Resumed);
        s.send(HostEvent::FocusGained);
        let asked = s.asked.get();
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Continue);
        assert_eq!(s.asked.get(), asked);
    }

    #[test]
    fn events_before_app_created_are_noops() {
        let mut s = setup(false);
        for ev in [HostEvent::Resumed, HostEvent::FocusGained, HostEvent::FocusLost, HostEvent::Suspended, HostEvent::MemoryWarning] {
            assert_eq!(s.send(ev), Action::Continue, "{ev:?}");
        }
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Continue);
        assert!(s.slot.borrow().is_none());
        // the state arrives (the library opened): the first focus loss saves it
        *s.slot.borrow_mut() = Some(Fake::default());
        s.send(HostEvent::Resumed);
        s.send(HostEvent::FocusGained);
        s.send(HostEvent::FocusLost);
        assert_eq!(s.calls(), ["flush_leaving"]);
    }

    #[test]
    fn destroyed_while_the_library_opens_still_leaves() {
        // no host state yet, and none can arrive while suspended: the destroy must still end the loop
        let mut s = setup(false);
        s.send(HostEvent::FocusLost);
        s.send(HostEvent::Suspended);
        s.state.set(Ok(ActivityState { finishing: false, destroyed: true }));
        assert_eq!(s.send(HostEvent::AboutToWait), Action::Exit);
    }

    #[test]
    fn a_busy_state_is_skipped_not_a_panic() {
        let mut s = setup(true);
        let held = s.slot.borrow_mut();
        assert_eq!(on_event(HostEvent::FocusLost, &s.slot, &mut s.core), Action::Continue);
        drop(held);
        assert!(s.calls().is_empty());
        // the flush didn't run, so the suspend saves what changed
        s.send(HostEvent::Suspended);
        assert_eq!(s.calls(), ["flush_dirty", "trim"]);
    }

    #[test]
    fn leaving_needs_destroyed_or_finishing_without_a_surface() {
        let state = |finishing, destroyed| ActivityState { finishing, destroyed };
        assert!(!should_leave(state(false, false), true));
        assert!(!should_leave(state(true, false), false));
        assert!(should_leave(state(true, false), true));
        assert!(should_leave(state(false, true), false));
        assert!(should_leave(state(true, true), true));
    }
}
