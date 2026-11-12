//! Application-local GPU policy. Never silently select a discrete or software adapter.
use eframe::{
    egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew},
    wgpu,
};
use std::sync::Arc;

pub fn integrated_configuration() -> WgpuConfiguration {
    let setup = WgpuSetupCreateNew {
        power_preference: wgpu::PowerPreference::LowPower,
        native_adapter_selector: Some(Arc::new(|adapters, surface| {
            adapters.iter()
                .filter(|adapter|adapter.get_info().device_type==wgpu::DeviceType::IntegratedGpu)
                .filter(|adapter|surface.is_none_or(|surface|adapter.is_surface_supported(surface)))
                .min_by_key(|adapter|match adapter.get_info().backend{wgpu::Backend::Vulkan=>0,wgpu::Backend::Gl=>1,_=>2})
                .cloned()
                .ok_or_else(||"No compatible integrated GPU is available. Tide requires the iGPU; check its graphics driver and render-device access.".into())
        })),
        ..Default::default()
    };
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        present_mode: wgpu::PresentMode::AutoVsync,
        ..Default::default()
    }
}
