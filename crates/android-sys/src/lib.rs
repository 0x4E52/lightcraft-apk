//! The Android host's calls into Java (JNI), behind a safe API.
//!
//! Next to `crates/sysmem` and `apps/lightcraft-android`, this is one of the three crates allowed
//! `unsafe`, and it holds exactly one unsafe operation: the cast of android-activity's Activity
//! pointer to a JNI object in `jni_env::with_activity`. Everything else here is safe code.
//!
//! Every module is split into a pure part that compiles and is tested on every target (wasm32
//! included: this crate is L0 standalone) and a thin `#[cfg(target_os = "android")]` JNI part.
//! See [`jni_env`] for the rules every JNI caller follows.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod jni_env;

pub use jni_env::{Error, user_message};
