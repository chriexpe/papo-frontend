//! GPU policy for Papo.
//!
//! Desktop converges on eframe/egui-wgpu. The platform graphics API is an
//! implementation detail selected by wgpu (D3D12 on Windows, Vulkan on Linux,
//! Metal on macOS). Android keeps its existing eframe/Glow path for now.
//!
//! The old UI glass renderer is intentionally not mirrored here as a WGPU
//! paint callback: egui-wgpu callbacks execute inside the main render pass and
//! WebGPU forbids sampling from the render attachment while it is bound for
//! rendering. Exact backdrop blur therefore needs a scene texture/offscreen
//! composition pass. That compositor is the next step of this module.

#[cfg(not(target_os = "android"))]
pub fn desktop_renderer() -> eframe::Renderer {
    eframe::Renderer::Wgpu
}

/// Log the actual native backend chosen by wgpu. This is useful when a visual
/// or driver bug is reported: Papo code stays backend-neutral, while wgpu may
/// be using D3D12, Vulkan or Metal underneath.
#[cfg(not(target_os = "android"))]
pub fn log_adapter(cc: &eframe::CreationContext<'_>) {
    if let Some(state) = &cc.wgpu_render_state {
        let info = state.adapter.get_info();
        log::info!(
            "renderer: wgpu backend={:?} adapter={} driver={} ({:?})",
            info.backend,
            info.name,
            info.driver,
            info.device_type
        );
    }
}
