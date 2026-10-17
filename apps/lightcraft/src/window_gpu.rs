//! The window's renderer and its fallback chain (crash report: on some Windows machines FXC fails
//! to compile egui's own shaders on DX12 — "The parameter is incorrect. (0x80070057)" in
//! `egui_pipeline` — and wgpu's default error handler panicked, killing the app at start-up).
//!
//! - **Windows:** each renderer of [`backend::window_renderer_order`] (DX12, Vulkan, GL, then
//!   eframe's glow) is *probed* before the window opens: a device is created with an error handler
//!   that logs instead of panicking, and egui's pipeline is built inside error scopes. The first
//!   that builds cleanly is handed to eframe as an existing device (so its error handler stays).
//! - **Everywhere:** a marker file names the renderer being started and is removed once the
//!   window's renderer is up. Left behind — the start died or panicked — the next launch tries that
//!   renderer last; [`relaunch_after_failure`] starts that next launch right away.
//! - The renderer that worked is remembered (`<config>/window-renderer`, Windows only) and tried
//!   first next time. `LIGHTCRAFT_GPU_BACKEND=dx12|vulkan|gl|glow` (or `WGPU_BACKEND`) forces one.
//!
//! The marker files ([`Files`]) are shared with the Android host (`lightcraft_ui_egui::host::
//! gpu_markers`), and so are the device's error handler and pipeline check
//! (`lightcraft_engine::gpu::window`); the probe, the choice and the relaunch stay here.

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupExisting};
use eframe::wgpu;
use lightcraft_engine::gpu::backend::{self, Backends, WindowRenderer};
use lightcraft_engine::gpu::window::{check_pipeline, install_error_handler};
pub use lightcraft_ui_egui::host::gpu_markers::Files;

/// Environment variable counting automatic relaunches after a failed start (bounds the chain).
const RETRY_VAR: &str = "LIGHTCRAFT_WINDOW_RETRY";
/// Never more relaunches than renderers to try.
const MAX_RETRIES: u32 = 4;

/// The window's renderer for this start.
pub struct Chosen {
    pub renderer: eframe::Renderer,
    pub wgpu_options: WgpuConfiguration,
    pub which: WindowRenderer,
    /// For the user: the last start failed with another renderer, or one failed now.
    pub notice: Option<String>,
}

/// The renderer for this start: see the module docs.
pub fn choose(files: &Files) -> Chosen {
    let forced =
        backend::forced_window_renderer(std::env::var("LIGHTCRAFT_GPU_BACKEND").ok().as_deref(), std::env::var("WGPU_BACKEND").ok().as_deref());
    let failed = files.take_failed();
    let remembered = if cfg!(windows) { files.remembered_renderer() } else { None };
    let compiled = wgpu::Instance::enabled_backend_features();
    let os = std::env::consts::OS;
    let order = backend::window_renderer_order(forced, remembered, failed, os, compiled, true);
    let mut notice = failed.map(|f| format!("The window could not start with its {} renderer last time.", f.name()));
    let mut problems = Vec::new();
    let chosen = if cfg!(windows) {
        pick_probed(&order, &mut problems)
    } else if forced == Some(WindowRenderer::Glow) || (forced.is_none() && failed.is_some_and(|f| f != WindowRenderer::Glow)) {
        // elsewhere eframe's own wgpu set-up stays the default; glow after a failed start
        Some((WindowRenderer::Glow, None))
    } else {
        Some((WindowRenderer::Wgpu(backend::window_backends()), None))
    };
    let (which, existing) = chosen.unwrap_or((WindowRenderer::Glow, None));
    if !problems.is_empty() {
        notice = Some(format!("{} {}", notice.unwrap_or_default(), problems.join(" ")).trim().to_string());
    }
    let notice = notice.map(|n| {
        format!(
            "{n} {app} now draws its window with {}. To choose another, start {app} with {var}=dx12, vulkan, gl or glow \
             (see docs/gpu-pipeline.md → Troubleshooting).",
            which.name(),
            app = "LightCraft",
            var = "LIGHTCRAFT_GPU_BACKEND",
        )
    });
    log::info!("window: renderer {} (order {:?})", which.name(), order.iter().map(|r| r.name()).collect::<Vec<_>>());
    let mut wgpu_options = base_options();
    let renderer = match (which, existing) {
        (WindowRenderer::Glow, _) => eframe::Renderer::Glow,
        (WindowRenderer::Wgpu(_), Some(e)) => {
            wgpu_options.wgpu_setup = WgpuSetup::Existing(e);
            eframe::Renderer::Wgpu
        }
        (WindowRenderer::Wgpu(b), None) => {
            if let WgpuSetup::CreateNew(n) = &mut wgpu_options.wgpu_setup {
                n.instance_descriptor.backends = b;
            }
            eframe::Renderer::Wgpu
        }
    };
    Chosen { renderer, wgpu_options, which, notice }
}

/// eframe's defaults with the app's backends and DX12 shader compiler (issues #136, #471).
pub fn base_options() -> WgpuConfiguration {
    let mut c = WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(n) = &mut c.wgpu_setup {
        n.instance_descriptor.backends = backend::window_backends();
        n.instance_descriptor.backend_options = backend::backend_options();
        // no indirect-call validation pipeline: Intel Metal fails to compile it and the window never opens (#250)
        n.instance_descriptor.flags = backend::instance_flags_from_env(n.instance_descriptor.flags);
    }
    c
}

/// The first renderer of `order` that probes cleanly (glow is taken without a probe).
fn pick_probed(order: &[WindowRenderer], problems: &mut Vec<String>) -> Option<(WindowRenderer, Option<WgpuSetupExisting>)> {
    for &r in order {
        match r {
            WindowRenderer::Glow => return Some((r, None)),
            WindowRenderer::Wgpu(b) => {
                let probed = std::panic::catch_unwind(|| probe(b)).unwrap_or_else(|_| Err("panicked".into()));
                match probed {
                    Ok(e) => return Some((r, Some(e))),
                    Err(e) => {
                        log::error!("window: the {} renderer failed: {e}", r.name());
                        problems.push(format!("The {} renderer failed ({e}).", r.name()));
                    }
                }
            }
        }
    }
    None
}

/// Create a device on `backends` and build egui's pipeline on it; any wgpu error is returned.
fn probe(backends: Backends) -> Result<WgpuSetupExisting, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags: backend::instance_flags_from_env(wgpu::InstanceFlags::from_build_config().with_env()),
        backend_options: backend::backend_options(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance),
        ..Default::default()
    }))
    .map_err(|e| format!("no adapter: {e}"))?;
    let base = if adapter.get_info().backend == wgpu::Backend::Gl { wgpu::Limits::downlevel_webgl2_defaults() } else { wgpu::Limits::default() };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("egui wgpu device"),
        required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..base },
        ..Default::default()
    }))
    .map_err(|e| format!("{}: no device: {e}", adapter.get_info().name))?;
    install_error_handler(&device);
    check_pipeline(&device, |d| {
        drop(eframe::egui_wgpu::Renderer::new(d, wgpu::TextureFormat::Bgra8Unorm, eframe::egui_wgpu::RendererOptions::default()));
    })?;
    Ok(WgpuSetupExisting { instance, adapter, device, queue })
}

/// The start failed (error or panic) before the window's renderer came up: start the app again,
/// which then tries the next renderer. `false`: not relaunched (it came up, nothing written to
/// disk, or the retries are used up).
pub fn relaunch_after_failure(files: &Files) -> bool {
    if !files.armed() {
        return false;
    }
    let tries = std::env::var(RETRY_VAR).ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    if tries >= MAX_RETRIES {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    log::warn!("window: the renderer failed to start; starting again with the next one");
    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(RETRY_VAR, (tries + 1).to_string())
        .spawn()
        .map_err(|e| log::error!("window: could not start again: {e}"))
        .is_ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Without a settings folder no marker is written, so a failed start never relaunches (the
    /// chain is bounded by what is on disk).
    #[test]
    fn no_settings_folder_never_relaunches() {
        let none = Files::new(None);
        none.arm(WindowRenderer::Wgpu(Backends::DX12));
        assert!(!none.armed() && !relaunch_after_failure(&none));
    }

    /// egui's own pipeline builds cleanly under the shared pipeline check (the crash report:
    /// FXC failing on it must come back as an error). Needs an adapter; skipped where there is
    /// none (CI without a GPU).
    #[test]
    fn egui_pipeline_builds_cleanly() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: backend::window_backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            lightcraft_testkit::skip!(Gpu, "no GPU adapter");
            return;
        };
        let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
            lightcraft_testkit::skip!(Gpu, "no GPU device");
            return;
        };
        install_error_handler(&device);
        check_pipeline(&device, |d| {
            drop(eframe::egui_wgpu::Renderer::new(d, wgpu::TextureFormat::Bgra8Unorm, eframe::egui_wgpu::RendererOptions::default()));
        })
        .unwrap();
    }
}
