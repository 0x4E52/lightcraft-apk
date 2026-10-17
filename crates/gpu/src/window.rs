//! Helpers for the device that draws a host's window (egui-wgpu's), shared by the desktop app and
//! the Android host. The same wgpu as eframe's (one version in the workspace), so the devices are
//! eframe's own.
//!
//! wgpu's default handler for errors outside an error scope panics ("Handling wgpu errors as fatal
//! by default"); a crash report showed FXC failing to compile egui's own shaders on some Windows
//! machines and the panic killing the app at start-up. [`install_error_handler`] logs instead, and
//! [`check_pipeline`] turns an error while building a pipeline into a `Result`.

/// Log errors that happen outside an error scope instead of wgpu's default panic.
pub fn install_error_handler(device: &wgpu::Device) {
    device.on_uncaptured_error(std::sync::Arc::new(|e| log::error!("window: wgpu error: {e}")));
}

/// Run `build` inside validation, internal and out-of-memory error scopes: the first error is
/// returned instead of reaching the uncaptured-error handler.
pub fn check_pipeline(device: &wgpu::Device, build: impl FnOnce(&wgpu::Device)) -> Result<(), String> {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    build(device);
    let errors = [pollster::block_on(validation.pop()), pollster::block_on(internal.pop()), pollster::block_on(oom.pop())];
    match errors.into_iter().flatten().next() {
        Some(e) => Err(e.to_string()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend;

    /// The crash report: a pipeline that fails validation must come back as an error, never as
    /// wgpu's default panic. Needs an adapter; skipped where there is none (CI without a GPU).
    #[test]
    fn pipeline_errors_are_captured_not_panics() {
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
        // a shader that does not validate, then a pipeline using it (like a failed FXC compile)
        let r = check_pipeline(&device, |d| {
            let m = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("broken"),
                source: wgpu::ShaderSource::Wgsl("@vertex fn vs_main() -> @builtin(position) vec4<f32> { return undefined_thing; }".into()),
            });
            let _ = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("egui_pipeline"),
                layout: None,
                vertex: wgpu::VertexState { module: &m, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            });
        });
        assert!(r.is_err(), "{r:?}");
        // nothing built: no error
        assert_eq!(check_pipeline(&device, |_| {}), Ok(()));
        // outside a scope: logged by our handler, not a panic
        let _ = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl("not wgsl".into()) });
    }
}
