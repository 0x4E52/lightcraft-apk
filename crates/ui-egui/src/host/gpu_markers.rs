//! Files in the settings folder that remember a GPU start that died, so the next start avoids
//! it and says why.
//!
//! - `gpu-init.marker` (issue #136): written while the GPU compute device is created
//!   (`lightcraft_engine::gpu::backend::set_init_marker`), removed once that returned. Left
//!   behind, the process died inside the driver: [`gpu_crash_check_at`] reports it and GPU
//!   rendering starts off.
//! - `gpu-lost.marker` (issue #1337): written when the window stopped drawing after the GPU device
//!   was lost ([`record_gpu_lost`]); [`gpu_lost_check_at`] reports it once.
//! - `window-init.marker` / `window-renderer` ([`Files`]): the window's renderer being started,
//!   and (Windows) the one that last worked.
//!
//! The settings folder is a parameter (`None`: nothing is read or written — `--memory`,
//! `LIGHTCRAFT_NO_PREFS`). How the user forces another backend differs per host (an environment
//! variable on the desktop, a setting or a system property on Android), so the notices take the
//! host's `how_to_force` sentence.

use std::path::{Path, PathBuf};

use lightcraft_engine::gpu::backend::{self, WindowRenderer};

/// `<config>/gpu-init.marker`.
pub fn gpu_marker_path(config: Option<&Path>) -> Option<PathBuf> {
    config.map(|d| d.join("gpu-init.marker"))
}

/// `<config>/gpu-lost.marker`.
pub fn gpu_lost_marker_path(config: Option<&Path>) -> Option<PathBuf> {
    config.map(|d| d.join("gpu-lost.marker"))
}

/// Arm the GPU crash sentinel on `marker`; if the previous launch left it behind (it died inside
/// the GPU driver while creating the device), the notice to show — GPU rendering then starts off.
/// `how_to_force` ends the notice's sentence on trying another graphics backend
/// ([`gpu_crash_notice`]).
///
/// A `--memory` session (`in_memory`) writes nothing (issue #169): it neither arms the sentinel —
/// which would create the settings folder — nor removes a marker it finds. It still honours one,
/// so the GPU starts off there too, and the next ordinary launch reports and clears it.
pub fn gpu_crash_check_at(marker: Option<PathBuf>, in_memory: bool, how_to_force: &str) -> Option<String> {
    gpu_crash_check_with(marker, in_memory, how_to_force, backend::set_init_marker)
}

/// [`gpu_crash_check_at`] arming the sentinel with `arm` (tests pass their own, so they never
/// touch the process-wide marker that GPU device creation in other tests writes).
fn gpu_crash_check_with(marker: Option<PathBuf>, in_memory: bool, how_to_force: &str, arm: impl FnOnce(Option<PathBuf>)) -> Option<String> {
    let left = if in_memory {
        marker.as_deref().and_then(backend::read_init_marker)
    } else {
        let left = marker.as_deref().and_then(backend::take_init_marker);
        arm(marker);
        left
    };
    left.map(|what| gpu_crash_notice(&what, how_to_force))
}

/// What to tell the user after a start died creating the GPU device (`what`: the marker's text).
/// `how_to_force` completes "to try another graphics backend, …": the desktop names
/// `LIGHTCRAFT_GPU_BACKEND`, Android its setting and system property.
pub fn gpu_crash_notice(what: &str, how_to_force: &str) -> String {
    format!(
        "LightCraft closed unexpectedly while starting the GPU last time ({what}), so GPU rendering is now off and photos render on the CPU. \
         To try the GPU again, turn on Settings ▸ Performance ▸ Use the GPU for rendering; to try another graphics backend, {how_to_force}."
    )
}

/// Why the GPU was lost, when the GPU pipeline stopped after a device failure in this process (a
/// driver reset, a hang, the Windows watchdog): `None` for other reasons it is off, or when it
/// never failed.
pub fn gpu_lost_reason() -> Option<String> {
    lightcraft_engine::gpu::unavailable_reason().and_then(|r| r.strip_prefix("stopped after a GPU failure: ").map(str::to_string))
}

/// Record that LightCraft stopped after the GPU was lost, for the next launch.
pub fn record_gpu_lost(marker: Option<&Path>, reason: &str) {
    let Some(m) = marker else { return };
    if let Some(d) = m.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = std::fs::write(m, format!("{reason}\n")) {
        log::warn!("could not write {}: {e}", m.display());
    }
}

/// A GPU loss the last launch stopped on: its notice, once (the marker is removed, except by a
/// `--memory` session, which leaves the disk as it found it).
pub fn gpu_lost_check_at(marker: Option<&Path>, in_memory: bool) -> Option<String> {
    let m = marker?;
    let reason = std::fs::read_to_string(m).ok()?;
    if !in_memory {
        let _ = std::fs::remove_file(m);
    }
    Some(format!(
        "LightCraft stopped last time after the graphics driver reset the GPU ({}) while photos rendered on it, so GPU rendering \
         is now off and photos render on the CPU. To try the GPU again, turn on Settings ▸ Performance ▸ Use the GPU for rendering; \
         an up-to-date graphics driver may help.",
        reason.trim()
    ))
}

/// The window renderer's files under the settings folder (`None` dir: write nothing —
/// `--memory`, `LIGHTCRAFT_NO_PREFS`).
pub struct Files {
    dir: Option<PathBuf>,
}

impl Files {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }
    fn marker(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("window-init.marker"))
    }
    fn remembered(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("window-renderer"))
    }
    fn read(p: Option<PathBuf>) -> Option<WindowRenderer> {
        std::fs::read_to_string(p?).ok().as_deref().and_then(WindowRenderer::parse)
    }
    fn write(p: Option<PathBuf>, r: WindowRenderer) {
        let Some(p) = p else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(&p, format!("{}\n", r.name())) {
            log::warn!("window: could not write {}: {e}", p.display());
        }
    }
    /// The renderer that last worked (written by [`Files::started`] on Windows).
    pub fn remembered_renderer(&self) -> Option<WindowRenderer> {
        Self::read(self.remembered())
    }
    /// The renderer the last start died in (marker left behind), removing the marker.
    pub fn take_failed(&self) -> Option<WindowRenderer> {
        let m = self.marker()?;
        let r = Self::read(Some(m.clone()));
        let _ = std::fs::remove_file(m);
        r
    }
    /// Before the window opens: record what is being started.
    pub fn arm(&self, r: WindowRenderer) {
        Self::write(self.marker(), r);
    }
    /// The window's renderer is up: clear the marker and (Windows) remember it.
    pub fn started(&self, r: WindowRenderer) {
        if let Some(m) = self.marker() {
            let _ = std::fs::remove_file(m);
        }
        if cfg!(windows) {
            Self::write(self.remembered(), r);
        }
    }
    /// The marker is still there: the window never came up.
    pub fn armed(&self) -> bool {
        self.marker().is_some_and(|m| m.exists())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lightcraft_engine::gpu::backend::Backends;

    const HINT: &str = "start LightCraft with LIGHTCRAFT_GPU_BACKEND=dx12, vulkan or off";

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-host-gpu-markers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// Issue #169: a `--memory` session neither arms the GPU crash sentinel (which would create
    /// the settings folder) nor removes a marker a crashed launch left; it still reports it, and
    /// the next ordinary launch finds it, reports it and clears it.
    #[test]
    fn memory_session_leaves_the_gpu_marker_and_folder_alone() {
        let d = dir("memory");
        let m = d.join("gpu-init.marker");
        let mut armed: Vec<Option<PathBuf>> = Vec::new();
        // nothing left behind: no notice, and the folder is not created
        assert_eq!(gpu_crash_check_with(Some(m.clone()), true, HINT, |p| armed.push(p)), None);
        assert!(!d.exists(), "a --memory session must not create the settings folder");
        assert!(armed.is_empty(), "a --memory session doesn't arm the sentinel");
        // a marker left by a crashed launch: reported, kept
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(&m, "GPU device creation started (backends METAL)").unwrap();
        let notice = gpu_crash_check_with(Some(m.clone()), true, HINT, |p| armed.push(p)).unwrap();
        assert!(notice.contains("GPU rendering is now off"), "{notice}");
        assert!(m.exists(), "the marker stays for the next ordinary launch");
        // the next ordinary launch reports it once, removes it and arms the sentinel on it
        assert!(gpu_crash_check_with(Some(m.clone()), false, HINT, |p| armed.push(p)).is_some());
        assert!(!m.exists());
        assert_eq!(gpu_crash_check_with(Some(m.clone()), false, HINT, |p| armed.push(p)), None);
        assert_eq!(armed, [Some(m.clone()), Some(m.clone())]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Issue #1337: the window stopped drawing after the graphics driver reset the GPU during an
    /// export. The stop is recorded; the next launch says why once and renders on the CPU (a
    /// `--memory` session leaves the record).
    #[test]
    fn a_gpu_lost_while_running_turns_the_gpu_off_next_time() {
        let d = dir("lost");
        let m = gpu_lost_marker_path(Some(&d)).unwrap();
        assert_eq!(gpu_lost_check_at(Some(&m), false), None, "nothing recorded");
        assert_eq!(gpu_lost_check_at(None, false), None, "no settings folder");
        record_gpu_lost(Some(&m), "device lost (Unknown): Device is lost");
        let notice = gpu_lost_check_at(Some(&m), true).unwrap();
        assert!(
            notice.contains("(device lost (Unknown): Device is lost)")
                && notice.contains("GPU rendering is now off")
                && notice.contains("Use the GPU for rendering"),
            "{notice}"
        );
        assert!(m.exists(), "a --memory session leaves it");
        assert!(gpu_lost_check_at(Some(&m), false).is_some());
        assert!(!m.exists());
        assert_eq!(gpu_lost_check_at(Some(&m), false), None, "said once");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Issue #136: a GPU init marker left by a crashed launch is reported once and removed.
    #[test]
    fn gpu_crash_marker_is_reported_once() {
        let d = dir("crash");
        std::fs::create_dir_all(&d).unwrap();
        let m = gpu_marker_path(Some(&d)).unwrap();
        assert_eq!(backend::take_init_marker(&m), None);
        std::fs::write(&m, "GPU device creation started (backends VULKAN | DX12)").unwrap();
        let what = backend::take_init_marker(&m).unwrap();
        let notice = gpu_crash_notice(&what, HINT);
        assert!(notice.contains("GPU rendering is now off") && notice.contains("Use the GPU for rendering"), "{notice}");
        assert!(!m.exists());
        assert_eq!((gpu_marker_path(None), gpu_lost_marker_path(None)), (None, None));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The notice tells the user how to force another backend the way their host allows: the
    /// desktop's environment variable, Android's setting or system property.
    #[test]
    fn crash_notice_uses_host_hint() {
        let desktop = gpu_crash_notice("device creation", HINT);
        assert!(desktop.ends_with("to try another graphics backend, start LightCraft with LIGHTCRAFT_GPU_BACKEND=dx12, vulkan or off."), "{desktop}");
        let android = gpu_crash_notice("device creation", "use Settings ▸ Performance, or adb shell setprop debug.lightcraft.gpu_backend gl");
        assert!(android.contains("setprop debug.lightcraft.gpu_backend gl") && !android.contains("LIGHTCRAFT_GPU_BACKEND"), "{android}");
        assert!(android.contains("(device creation)"));
    }

    /// The marker names the renderer being started; left behind, the next start reads it as the
    /// failed one (and removes it); a good start removes it and remembers the renderer (Windows).
    #[test]
    fn marker_records_the_failed_renderer() {
        let d = dir("window");
        let f = Files::new(Some(d.clone()));
        let dx = WindowRenderer::Wgpu(Backends::DX12);
        f.arm(dx);
        assert!(f.armed());
        assert_eq!(f.take_failed(), Some(dx));
        assert!(!f.armed());
        assert_eq!(f.take_failed(), None);
        f.arm(WindowRenderer::Glow);
        f.started(WindowRenderer::Glow);
        assert!(!f.armed());
        assert_eq!(f.remembered_renderer(), cfg!(windows).then_some(WindowRenderer::Glow));
        // nothing written without a settings folder
        let none = Files::new(None);
        none.arm(dx);
        assert!(!none.armed());
        let _ = std::fs::remove_dir_all(&d);
    }
}
