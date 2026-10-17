//! Where the desktop app's log file lives and when it is opened. The logger itself (standard
//! error plus a rotated file, panics recorded) is shared with the Android host:
//! `lightcraft_ui_egui::host::log_file`.
//!
//! A launch from a desktop menu or the Dock has no terminal, so the file is what a bug report can
//! attach: `<settings folder>/logs/lightcraft.log`, next to `ui.json` (`settings_dir` in
//! `main.rs`; Linux `$XDG_CONFIG_HOME/lightcraft/logs`, by default `~/.config/lightcraft/logs`).
//! Never in the library folder. Runs with `LIGHTCRAFT_NO_PREFS` (tests, scripts) and `--memory`
//! sessions log to standard error only, so they don't rotate away the user's own logs.

use std::path::{Path, PathBuf};

pub use lightcraft_ui_egui::host::log_file::{AppLogger, install, record_panics};

/// `logs` in the settings folder (`None`: no settings folder, so no log file).
pub fn log_dir(settings: Option<&Path>) -> Option<PathBuf> {
    settings.map(|d| d.join("logs"))
}

/// Open the log file in `dir` (rotating the previous ones) and return its path; without a
/// folder, or when the file can't be created, records go to standard error only. Called once the
/// arguments are parsed, so `--version`, `--help` and a usage error leave no file behind; records
/// logged until now are written to it first.
pub fn open_file(logger: &AppLogger, dir: Option<PathBuf>) -> Option<PathBuf> {
    match dir {
        Some(dir) => match logger.attach_dir(&dir) {
            Ok(path) => {
                log::info!("LightCraft {}, log file {}", env!("CARGO_PKG_VERSION"), path.display());
                Some(path)
            }
            // The file sink has given up by now, so this goes to standard error only.
            Err(e) => {
                log::warn!("no log file: {e}");
                None
            }
        },
        None => {
            logger.no_file();
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lightcraft_ui_egui::host::log_file::{DEFAULT_FILTER, Filter, LOG_FILE};

    #[test]
    fn the_log_file_opens_in_the_settings_folder_or_not_at_all() {
        let settings = std::env::temp_dir().join(format!("lc-app-logging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&settings);
        let logger = AppLogger::new(Filter::parse(DEFAULT_FILTER), false);
        let path = open_file(&logger, log_dir(Some(&settings))).unwrap();
        assert_eq!(path, settings.join("logs").join(LOG_FILE));
        assert!(path.exists());
        assert_eq!(log_dir(None), None);
        assert_eq!(open_file(&AppLogger::new(Filter::parse(DEFAULT_FILTER), false), None), None);
        let _ = std::fs::remove_dir_all(&settings);
    }
}
