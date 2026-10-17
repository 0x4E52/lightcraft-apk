//! Opening the library session a native host starts with.
//!
//! The library folder is the host's to name: the desktop app's `--library`, `LIGHTCRAFT_LIBRARY`,
//! the library last opened or `~/Pictures/LightCraft Library`; the Android host's internal storage.
//! A library that can't be opened is never replaced silently: the session is empty and in
//! memory, and the problem is returned for the window to ask what to do
//! ([`crate::panels::library_problem`]).

use std::path::PathBuf;

use lightcraft_engine::Session;

use crate::panels::library_problem::LibraryProblem;

/// The library session with the folders face models (`<config>/models`) and slideshow templates
/// (`<config>/slideshow-templates`) are kept in.
pub fn open_session(in_memory: bool, dir: Option<PathBuf>, seed_demo: bool) -> (Session, Option<LibraryProblem>) {
    let (mut s, problem) = open_library_session(in_memory, dir, seed_demo);
    s.face_models_dir = lightcraft_engine::config::default_face_models_dir();
    s.slide_templates_dir = lightcraft_engine::config::config_dir().map(|d| d.join(lightcraft_engine::slide_template::DIR_NAME));
    // Control requests are answered on the UI thread: Add to Apple Photos must not wait there
    s.apple_photos_background = true;
    (s, problem)
}

/// The persistent library session in `dir`, or with `in_memory` an in-memory one (demo photos
/// when `seed_demo`).
///
/// If the library can't be opened the session is empty and in memory — never seeded with the
/// demo photos, never written anywhere — and the problem is returned: the window then says so
/// and offers Try Again / Choose Another Library… / Continue Without Saving / Quit (issue #100).
pub fn open_library_session(in_memory: bool, dir: Option<PathBuf>, seed_demo: bool) -> (Session, Option<LibraryProblem>) {
    if in_memory {
        return (if seed_demo { Session::with_demo() } else { Session::new() }.with_fs().with_default_denoise_models().with_system_clock(), None);
    }
    let unopened = || Session::new().with_fs().with_default_denoise_models().with_system_clock();
    let Some(dir) = dir else {
        log::warn!("no library location (set --library or LIGHTCRAFT_LIBRARY)");
        return (unopened(), Some(LibraryProblem::new("", "There is no home folder to keep the library in. Choose a folder for it.")));
    };
    let t0 = web_time::Instant::now();
    let mut s = Session::new().with_fs().with_default_denoise_models().with_system_clock();
    match s.open_library(&dir, seed_demo) {
        Ok(r) => {
            let (replayed, torn) = (r.replayed, r.torn_bytes);
            log::info!(
                "library {}: {} photos, {replayed} log records replayed{} ({:.1} ms)",
                dir.display(),
                s.catalog.len(),
                if torn > 0 { ", torn tail repaired" } else { "" },
                t0.elapsed().as_secs_f64() * 1000.0
            );
            (s, None)
        }
        Err(e) => {
            log::error!("can't open library {}: {e}", dir.display());
            (unopened(), Some(LibraryProblem::new(dir.to_string_lossy(), e.to_string())))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Issue #236: the window answers control requests on its UI thread, so the desktop session
    /// adds to Apple Photos on a worker (headless sessions wait instead).
    #[test]
    fn the_desktop_session_adds_to_apple_photos_in_the_background() {
        let (s, problem) = open_session(true, None, false);
        assert!(problem.is_none());
        assert!(s.apple_photos_background);
        assert!(!lightcraft_engine::Session::new().apple_photos_background);
    }

    /// Without a library folder the session is empty and in memory, and the problem says why.
    #[test]
    fn no_library_folder_is_a_problem_not_a_demo() {
        let (s, problem) = open_library_session(false, None, true);
        assert!(problem.is_some());
        assert!(s.catalog.is_empty() && s.library.is_none());
    }
}
