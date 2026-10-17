//! The one switch for LightCraft's diagnostics timings (`LIGHTCRAFT_PROFILE`).
//!
//! Every crate that prints stage timings to stderr (pipeline, GPU render, catalog journal,
//! denoise, faces, engine) asks [`enabled`]. A desktop or CLI process turns it on with the
//! environment variable `LIGHTCRAFT_PROFILE` (any value); a host without environment variables
//! (Android) calls [`set_enabled`] before the first read. The variable is read at most once per
//! process, on the first [`enabled`] call that finds the switch unset; [`set_enabled`] always
//! wins over it, also after that read.
//!
//! A crate of its own because the readers sit on four layers and share no lower crate.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::atomic::{AtomicU8, Ordering};

/// The environment variable that turns the timings on when nothing set the switch.
pub const VAR: &str = "LIGHTCRAFT_PROFILE";

const UNSET: u8 = 0;
const OFF: u8 = 1;
const ON: u8 = 2;

static STATE: AtomicU8 = AtomicU8::new(UNSET);

/// Whether diagnostics timings are printed. Unset: reads `LIGHTCRAFT_PROFILE` once.
pub fn enabled() -> bool {
    enabled_in(&STATE, || std::env::var_os(VAR).is_some())
}

/// Turn the timings on or off for the whole process, overriding `LIGHTCRAFT_PROFILE`.
pub fn set_enabled(on: bool) {
    set_in(&STATE, on);
}

/// [`enabled`] over `state`, with `env` answering "is the variable set" (called only while
/// `state` is unset).
fn enabled_in(state: &AtomicU8, env: impl FnOnce() -> bool) -> bool {
    match state.load(Ordering::Relaxed) {
        ON => true,
        OFF => false,
        _ => {
            let read = if env() { ON } else { OFF };
            // a `set_enabled` that ran meanwhile wins over the variable
            match state.compare_exchange(UNSET, read, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => read == ON,
                Err(current) => current == ON,
            }
        }
    }
}

/// [`set_enabled`] over `state`.
fn set_in(state: &AtomicU8, on: bool) {
    state.store(if on { ON } else { OFF }, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn profile_state_overrides_env() {
        // unset: the variable decides, read once
        let reads = Cell::new(0);
        let env_on = || {
            reads.set(reads.get() + 1);
            true
        };
        let state = AtomicU8::new(UNSET);
        assert!(enabled_in(&state, env_on));
        assert!(enabled_in(&state, env_on));
        assert_eq!(reads.get(), 1, "the variable is read once");

        // set before the first read: the variable is never read
        let state = AtomicU8::new(UNSET);
        set_in(&state, false);
        assert!(!enabled_in(&state, || panic!("the variable must not be read once the switch is set")));
        set_in(&state, true);
        assert!(enabled_in(&state, || false));

        // set after the variable was read: the setter wins
        let state = AtomicU8::new(UNSET);
        assert!(!enabled_in(&state, || false));
        set_in(&state, true);
        assert!(enabled_in(&state, || false));
        set_in(&state, false);
        assert!(!enabled_in(&state, || true));
    }
}
