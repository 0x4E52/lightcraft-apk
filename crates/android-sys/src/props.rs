//! The diagnostics switches an Android app cannot get from environment variables: system
//! properties set from a shell (`adb shell setprop debug.lightcraft.<name> <value>`).
//!
//! | Property | Value | Same as |
//! |---|---|---|
//! | `debug.lightcraft.profile` | `1`/`true`/`on`/`yes` | `LIGHTCRAFT_PROFILE` |
//! | `debug.lightcraft.log` | `debug`, `info` or `warn` | `LIGHTCRAFT_LOG` |
//! | `debug.lightcraft.gpu_backend` | `vulkan`, `gl`, `auto`, `off` … | `LIGHTCRAFT_GPU_BACKEND` |
//! | `debug.lightcraft.control` | a TCP port, 1 to 65535 | `--control <port>` (APK-21) |
//!
//! Only the shell (or root) can set a `debug.*` property, so another app cannot switch these on.
//! A missing, empty or unusable value leaves the switch at its default. [`settings_from`] is pure
//! (a closure answers "what is this property"), so it is tested on every target; [`read`] asks
//! the system and exists on Android only.

/// `debug.lightcraft.profile`: print per-stage pipeline timings.
pub const PROFILE: &str = "debug.lightcraft.profile";
/// `debug.lightcraft.log`: the log level of LightCraft's own crates.
pub const LOG: &str = "debug.lightcraft.log";
/// `debug.lightcraft.gpu_backend`: which graphics backend the window and the GPU compute use.
pub const GPU_BACKEND: &str = "debug.lightcraft.gpu_backend";
/// `debug.lightcraft.control`: the port of the debug control server.
pub const CONTROL: &str = "debug.lightcraft.control";

/// Longest backend list taken from a property (Android's own limit is 91 bytes).
const MAX_BACKEND_LEN: usize = 64;

/// What the `debug.lightcraft.*` properties ask for. The default asks for nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagSettings {
    /// Timings on (`debug.lightcraft.profile`).
    pub profile: bool,
    /// `debug`, `info` or `warn` (`debug.lightcraft.log`), lower-case; `None` keeps the default level.
    pub log: Option<String>,
    /// A backend name or list as `LIGHTCRAFT_GPU_BACKEND` takes it, lower-case (`debug.lightcraft.gpu_backend`).
    /// Whether the name is one the graphics layer knows is decided there: an unknown one changes nothing.
    pub gpu_backend: Option<String>,
    /// The control server's port (`debug.lightcraft.control`); `None`: no server.
    pub control: Option<u16>,
}

/// The settings `props` holds. `props(name)` answers a property's value (`None`: unset). Values
/// that mean nothing are ignored, never an error.
pub fn settings_from(props: impl Fn(&str) -> Option<String>) -> DiagSettings {
    let get = |name: &str| props(name).map(|v| v.trim().to_ascii_lowercase()).filter(|v| !v.is_empty());
    DiagSettings {
        profile: get(PROFILE).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes")),
        log: get(LOG).filter(|v| matches!(v.as_str(), "debug" | "info" | "warn")),
        gpu_backend: get(GPU_BACKEND)
            .filter(|v| v.len() <= MAX_BACKEND_LEN && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ',' | '_' | '-'))),
        control: get(CONTROL).and_then(|v| v.parse::<u16>().ok()).filter(|port| *port != 0),
    }
}

/// The settings the system's `debug.lightcraft.*` properties hold right now.
#[cfg(target_os = "android")]
pub fn read() -> DiagSettings {
    settings_from(|name| android_properties::getprop(name).value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn props(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn props_map_to_settings() {
        let s = settings_from(props(&[(PROFILE, "1"), (LOG, "Debug"), (GPU_BACKEND, " GL "), (CONTROL, "18555"), ("debug.lightcraft.other", "x")]));
        assert_eq!(s, DiagSettings { profile: true, log: Some("debug".into()), gpu_backend: Some("gl".into()), control: Some(18555) });
        for on in ["true", "ON", "Yes"] {
            assert!(settings_from(props(&[(PROFILE, on)])).profile, "{on}");
        }
        assert_eq!(settings_from(props(&[(GPU_BACKEND, "vulkan,gl")])).gpu_backend.as_deref(), Some("vulkan,gl"));
        // nothing set: nothing asked for
        assert_eq!(settings_from(|_| None), DiagSettings::default());
    }

    #[test]
    fn unknown_values_are_ignored() {
        let s = settings_from(props(&[(PROFILE, "maybe"), (LOG, "chatty"), (GPU_BACKEND, "vulkan; reboot"), (CONTROL, "http")]));
        assert_eq!(s, DiagSettings::default());
        // blank, zero, out of range and negative values too
        for (name, value) in
            [(PROFILE, "0"), (PROFILE, ""), (LOG, "  "), (GPU_BACKEND, ""), (CONTROL, "0"), (CONTROL, "70000"), (CONTROL, "-1"), (CONTROL, "")]
        {
            assert_eq!(settings_from(props(&[(name, value)])), DiagSettings::default(), "{name}={value:?}");
        }
        // a backend list longer than any property can be is dropped, not cut
        let long = "gl,".repeat(40);
        assert_eq!(settings_from(props(&[(GPU_BACKEND, &long)])).gpu_backend, None);
    }
}
