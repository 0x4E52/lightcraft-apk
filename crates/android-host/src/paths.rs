//! Where the Android host keeps its files: everything under the app's internal storage.
//!
//! android-activity's `AndroidApp::internal_data_path()` is NativeActivity's `internalDataPath`,
//! i.e. `Context.getFilesDir()` = `/data/user/0/<application id>/files`, so a path seen with
//! `adb shell run-as <id>` starts with `files/`. Internal storage is private to the app and is
//! deleted on uninstall or "Clear data" (decision 9; the first-start notice is APK-11b).
//!
//! ```text
//! files/config/    settings folder: ui.json, panic.log, models/, cache/ (engine::config::use_dir)
//! files/library/   the library (catalog, previews, Copy-imported originals)
//! files/logs/      lightcraft.log and the two before it
//! files/inbox/     files shared to LightCraft, before they are imported (APK-24, APK-25)
//! ```

use std::path::{Path, PathBuf};

/// The host's folders, all inside the app's internal storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The settings folder, given to `lightcraft_engine::config::use_dir`.
    pub config: PathBuf,
    /// The library folder the session opens.
    pub library: PathBuf,
    /// The log folder (`host::log_file`).
    pub logs: PathBuf,
    /// Shared files waiting to be imported.
    pub inbox: PathBuf,
}

/// The name of the panic log inside the settings folder (`engine::guard::install_hook`).
pub const PANIC_LOG: &str = "panic.log";

impl Paths {
    /// The folders inside `files` (the internal storage folder, `…/files`). Pure: nothing is
    /// created.
    pub fn from_internal(files: &Path) -> Paths {
        Paths { config: files.join("config"), library: files.join("library"), logs: files.join("logs"), inbox: files.join("inbox") }
    }

    /// The panic log, `<config>/panic.log`.
    pub fn panic_log(&self) -> PathBuf {
        self.config.join(PANIC_LOG)
    }

    /// Create the folders that must exist before the first write (settings, logs, inbox; the
    /// library folder is the session's to create). Quick: a few `mkdir`s on internal storage.
    pub fn create(&self) -> Result<(), String> {
        for dir in [&self.config, &self.logs, &self.inbox] {
            std::fs::create_dir_all(dir).map_err(|e| format!("LightCraft could not create its folder {}: {e}", dir.display()))?;
        }
        Ok(())
    }
}

/// The internal storage folder from what android-activity reports: `Err` with a message for the
/// user when there is none (the device's storage is unavailable).
pub fn internal_dir(reported: Option<PathBuf>) -> Result<PathBuf, String> {
    reported.filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| {
        "Android gave LightCraft no folder in its internal storage, so it can't keep a library. Restart the device and open LightCraft again."
            .to_owned()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn paths_from_internal_dir() {
        let files = Path::new("/data/user/0/ai.storyteller.lightcraft/files");
        let p = Paths::from_internal(files);
        assert_eq!(p.config, files.join("config"));
        assert_eq!(p.library, files.join("library"));
        assert_eq!(p.logs, files.join("logs"));
        assert_eq!(p.inbox, files.join("inbox"));
        assert_eq!(p.panic_log(), files.join("config").join("panic.log"));
        // what `run-as <id>` sees: every folder under files/
        for dir in [&p.config, &p.library, &p.logs, &p.inbox] {
            assert!(dir.strip_prefix(files.parent().unwrap()).unwrap().starts_with("files"), "{}", dir.display());
        }
        assert_eq!(internal_dir(Some(files.to_path_buf())), Ok(files.to_path_buf()));
        assert!(internal_dir(None).unwrap_err().contains("no folder in its internal storage"));
        assert!(internal_dir(Some(PathBuf::new())).is_err());
    }

    #[test]
    fn create_makes_the_folders_but_not_the_library() {
        let files = std::env::temp_dir().join(format!("lc-android-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files);
        let p = Paths::from_internal(&files);
        p.create().unwrap();
        assert!(p.config.is_dir() && p.logs.is_dir() && p.inbox.is_dir());
        assert!(!p.library.exists(), "the session creates the library");
        p.create().unwrap();
        // a file where the folders should go is an error to log, not a panic
        let blocked = files.join("blocked");
        std::fs::write(&blocked, "not a folder").unwrap();
        let e = Paths::from_internal(&blocked).create().unwrap_err();
        assert!(e.contains("could not create its folder"), "{e}");
        let _ = std::fs::remove_dir_all(&files);
    }
}
