//! LightCraft's Android host: what `android_main` (in `apps/lightcraft-android`) runs.
//!
//! `run` (Android only) guards against a second start in one process, does the quick start-up
//! work ([`paths`], [`memory`], the log and panic hook), opens the library on a worker
//! ([`loader`]) and runs eframe until the Activity ends; the app it shows is the loading frame,
//! then [`state::HostState`]'s LightCraft UI with the [`services`] Android provides. A wrapper
//! around eframe's event handler saves, trims and leaves the loop as the Activity comes and goes
//! ([`lifecycle_core`]). A renderer that won't start is reported in a dialog and the next start
//! tries the other one ([`renderer`]). The pure parts ([`Exit`], [`exit_code`], [`try_start`],
//! and every module but the eframe app and the wrapper) compile and are tested on every target;
//! the Android-only dependencies (eframe, winit, android-activity) sit behind
//! `cfg(target_os = "android")`, so `cargo test -p lightcraft-android-host` builds on Linux
//! without them. No `unsafe`: the entry symbol lives in
//! `apps/lightcraft-android` and JNI in `crates/android-sys`.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "android")]
mod app;
pub mod control;
pub mod input;
pub mod insets;
#[cfg(target_os = "android")]
mod lifecycle;
pub mod lifecycle_core;
pub mod loader;
pub mod memory;
pub mod paths;
pub mod renderer;
mod run;
pub mod services;
pub mod state;

#[cfg(target_os = "android")]
pub use run::run;
pub use run::{Exit, REOPENED, exit_code, try_start};
