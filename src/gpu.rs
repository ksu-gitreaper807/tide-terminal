//! Explicit application-local adapter policy; only Auto permits another device class.
use eframe::{
    egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew},
    wgpu,
};
use std::sync::Arc;
use tide_terminal::config::GpuMode;

fn rank(mode: GpuMode, device: wgpu::DeviceType, backend: wgpu::Backend) -> Option<(u8, u8)> {
    use wgpu::DeviceType::*;
    let class = match (mode, device) {
        (GpuMode::Integrated, IntegratedGpu)
        | (GpuMode::Discrete, DiscreteGpu)
        | (GpuMode::Software, Cpu) => 0,
        (GpuMode::Auto, IntegratedGpu) => 0,
        (GpuMode::Auto, DiscreteGpu) => 1,
        (GpuMode::Auto, VirtualGpu) => 2,
        (GpuMode::Auto, Cpu) => 3,
        (GpuMode::Auto, Other) => 4,
        _ => return None,
    };
    Some((
        class,
        match backend {
            wgpu::Backend::Vulkan => 0,
            wgpu::Backend::Gl => 1,
            _ => 2,
        },
    ))
}

pub fn configuration(mode: GpuMode) -> WgpuConfiguration {
    let setup = WgpuSetupCreateNew {
        power_preference: if mode == GpuMode::Discrete {
            wgpu::PowerPreference::HighPerformance
        } else {
            wgpu::PowerPreference::LowPower
        },
        native_adapter_selector: Some(Arc::new(move |adapters, surface| {
            adapters.iter()
                .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)))
                .filter_map(|adapter| {
                    let info = adapter.get_info();
                    rank(mode, info.device_type, info.backend).map(|rank| (rank, adapter))
                })
                .min_by_key(|(rank, _)| *rank)
                .map(|(_, adapter)| adapter.clone())
                .ok_or_else(|| format!("No surface-compatible {} adapter is available. Check its driver, or explicitly choose gui --gpu auto or --gpu software.", mode.label()))
        })),
        ..Default::default()
    };
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        present_mode: wgpu::PresentMode::AutoVsync,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_modes_do_not_silently_change_device_class() {
        use wgpu::{Backend::Vulkan, DeviceType::*};
        for (mode, wanted) in [
            (GpuMode::Integrated, IntegratedGpu),
            (GpuMode::Discrete, DiscreteGpu),
            (GpuMode::Software, Cpu),
        ] {
            for candidate in [IntegratedGpu, DiscreteGpu, VirtualGpu, Cpu, Other] {
                assert_eq!(rank(mode, candidate, Vulkan).is_some(), candidate == wanted);
            }
        }
        assert!(
            rank(GpuMode::Auto, IntegratedGpu, Vulkan) < rank(GpuMode::Auto, DiscreteGpu, Vulkan)
        );
        assert!(rank(GpuMode::Auto, DiscreteGpu, Vulkan) < rank(GpuMode::Auto, Cpu, Vulkan));
    }
}
