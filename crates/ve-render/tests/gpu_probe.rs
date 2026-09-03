//! Can this machine give us a compute device at all?
//!
//! Run before relying on the GPU backend: it reports the adapter rather than
//! asserting one exists, because a machine without a suitable GPU is a
//! supported configuration — the CPU evaluator is the fallback, and is the
//! authority for export regardless (spec.md 7.8).

#[test]
fn report_gpu_availability() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }));

    match adapter {
        Ok(adapter) => {
            let info = adapter.get_info();
            println!(
                "adapter: {} ({:?}, {:?})",
                info.name, info.device_type, info.backend
            );
            let limits = adapter.limits();
            println!(
                "  max storage buffer: {} MB, max workgroups: {}",
                limits.max_storage_buffer_binding_size / (1024 * 1024),
                limits.max_compute_workgroups_per_dimension
            );

            let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("ve-probe"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                ..Default::default()
            }));
            match device {
                Ok(_) => println!("  device created: compute is available"),
                Err(err) => println!("  device creation failed: {err}"),
            }
        }
        Err(err) => println!("no adapter: {err}"),
    }
}
