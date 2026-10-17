//! Host code shared by the native apps: what the desktop app (`apps/lightcraft`) and the Android
//! host both need, so neither copies it.
//!
//! - [`prefs`]: the app settings file (`<settings>/ui.json`): reading it (a damaged one is set
//!   aside and reported), writing it atomically and only when it changed ([`prefs::PrefsWriter`]).
//! - [`session_open`]: opening the library session (or an in-memory one), reporting a library that
//!   can't be opened instead of replacing it.
//! - [`log_file`]: the logger: standard error plus a rotated log file, panics recorded, and the
//!   last error-level record kept for a host that reports a failed start.
//! - [`flush`]: saving library, view and settings while the app runs ([`flush::flush_periodic`])
//!   and when it is left or quits ([`flush::flush_leaving`]).
//! - [`gpu_markers`]: the files that remember a GPU or window renderer start that died, so the
//!   next start avoids it and says why.
//!
//! - [`control_server`]: the loopback JSON-lines control channel (`--control` on the desktop, the
//!   `debug.lightcraft.control` property with a token on Android).
//!
//! Every folder (settings, library) is a parameter: the host decides where they are (the desktop
//! from the platform's folders or `LIGHTCRAFT_CONFIG_DIR`, Android from its internal storage).
//! Desktop-only parts (file dialogs, the Windows renderer probe, relaunching) stay in the app.

pub mod control_server;
pub mod flush;
pub mod gpu_markers;
pub mod log_file;
pub mod prefs;
pub mod session_open;
