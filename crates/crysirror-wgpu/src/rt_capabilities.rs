// SPDX-License-Identifier: MPL-2.0
//! Vendor-neutral ray-query negotiation. Advertising a feature, requesting it
//! on a device, and rendering a hardware-RT frame are separate facts.
use anyhow::{Result, ensure};
use serde::Serialize;

/// The initial engine BLAS budget, independent of any vendor's marketing name.
pub const MAX_RT_PRIMITIVES: u32 = 1_000_000;

/// Acceleration-structure limits actually advertised or enabled by wgpu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AccelerationLimits {
    pub max_blas_primitive_count: u32,
    pub max_blas_geometry_count: u32,
    pub max_tlas_instance_count: u32,
    pub max_acceleration_structures_per_shader_stage: u32,
}
impl From<&wgpu::Limits> for AccelerationLimits {
    fn from(limits: &wgpu::Limits) -> Self {
        Self {
            max_blas_primitive_count: limits.max_blas_primitive_count,
            max_blas_geometry_count: limits.max_blas_geometry_count,
            max_tlas_instance_count: limits.max_tlas_instance_count,
            max_acceleration_structures_per_shader_stage: limits
                .max_acceleration_structures_per_shader_stage,
        }
    }
}
impl AccelerationLimits {
    fn unavailable_reason(self) -> Option<&'static str> {
        if self.max_blas_primitive_count == 0 {
            Some("the BLAS primitive limit is zero")
        } else if self.max_blas_geometry_count == 0 {
            Some("the BLAS geometry limit is zero")
        } else if self.max_tlas_instance_count == 0 {
            Some("the TLAS instance limit is zero")
        } else if self.max_acceleration_structures_per_shader_stage == 0 {
            Some("the acceleration-structure shader binding limit is zero")
        } else {
            None
        }
    }
}

/// Adapter support and device opt-in are reported independently. `available`
/// does not claim a tested frame; `enabled` means the device requested the
/// required API/limits. Neither field certifies dedicated RT silicon, physical
/// traversal units, performance, or full-material coverage.
#[derive(Clone, Debug, Serialize)]
pub struct HardwareRtReport {
    pub adapter_name: String,
    pub backend: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub device_type: String,
    pub ray_query_advertised: bool,
    pub ray_query_enabled: bool,
    pub available: bool,
    pub enabled: bool,
    /// Portable wgpu exposes API support, not an authoritative dedicated-RT-unit
    /// flag. None means no physical-silicon verification was performed; native
    /// API enablement or a vendor/model name must never turn this into Some(true).
    pub dedicated_rt_hardware_verified: Option<bool>,
    /// Explains unavailable support or a device that has not opted in.
    pub reason: Option<String>,
    pub adapter_limits: AccelerationLimits,
    pub device_limits: Option<AccelerationLimits>,
}

/// Build a report from real adapter/device information. A raster device may
/// report `available=true, enabled=false`; it has not secretly enabled RT.
pub fn report(adapter: &wgpu::Adapter, device: Option<&wgpu::Device>) -> HardwareRtReport {
    let info = adapter.get_info();
    let features = adapter.features();
    let limits = adapter.limits();
    let enabled = device.map(|d| (d.features(), d.limits()));
    report_values(
        &info,
        features,
        &limits,
        enabled.as_ref().map(|(f, l)| (*f, l)),
    )
}

fn report_values(
    info: &wgpu::AdapterInfo,
    features: wgpu::Features,
    limits: &wgpu::Limits,
    device: Option<(wgpu::Features, &wgpu::Limits)>,
) -> HardwareRtReport {
    let advertised = features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
    let native_backend = matches!(
        info.backend,
        wgpu::Backend::Vulkan | wgpu::Backend::Dx12 | wgpu::Backend::Metal
    );
    let adapter_limits = AccelerationLimits::from(limits);
    let unavailable = if !native_backend {
        Some("hardware ray queries require the native Vulkan, DX12 or Metal backend".into())
    } else if info.device_type == wgpu::DeviceType::Cpu {
        Some("the adapter is CPU/software; it cannot be reported as hardware ray tracing".into())
    } else if !advertised {
        Some("the adapter/backend/driver does not advertise EXPERIMENTAL_RAY_QUERY".into())
    } else if let Some(reason) = adapter_limits.unavailable_reason() {
        Some(reason.into())
    } else {
        negotiate_limits(limits)
            .err()
            .map(|error| format!("{error:#}"))
    };
    let available = unavailable.is_none();
    let ray_query_enabled = device
        .is_some_and(|(features, _)| features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY));
    let device_limits = device.map(|(_, limits)| AccelerationLimits::from(limits));
    let disabled_reason = if device.is_none() {
        Some("no device was requested; adapter support alone does not enable hardware RT".into())
    } else if !ray_query_enabled {
        Some("the device did not request EXPERIMENTAL_RAY_QUERY".into())
    } else {
        device_limits
            .and_then(AccelerationLimits::unavailable_reason)
            .map(|reason| format!("device {reason}"))
    };
    let enabled = available && disabled_reason.is_none();
    HardwareRtReport {
        adapter_name: info.name.clone(),
        backend: info.backend.to_str().to_owned(),
        vendor_id: info.vendor,
        device_id: info.device,
        device_type: format!("{:?}", info.device_type),
        ray_query_advertised: advertised,
        ray_query_enabled,
        available,
        enabled,
        dedicated_rt_hardware_verified: None,
        reason: unavailable.or(disabled_reason),
        adapter_limits,
        device_limits,
    }
}

/// Request only the limits used by the initial engine hardware ray-query path.
/// This is used only after the adapter's feature support has been checked.
/// No limits are fabricated or raised above the actual adapter limits.
pub fn negotiate_limits(adapter_limits: &wgpu::Limits) -> Result<wgpu::Limits> {
    if let Some(reason) = AccelerationLimits::from(adapter_limits).unavailable_reason() {
        anyhow::bail!("hardware ray-query limits unavailable: {reason}");
    }
    let mut limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter_limits.clone());
    limits.max_blas_primitive_count = adapter_limits
        .max_blas_primitive_count
        .min(MAX_RT_PRIMITIVES);
    limits.max_blas_geometry_count = 1;
    limits.max_tlas_instance_count = 1;
    limits.max_acceleration_structures_per_shader_stage = 1;
    ensure!(
        limits.check_limits(adapter_limits),
        "adapter limits do not satisfy the renderer's baseline and hardware ray-query requirements"
    );
    Ok(limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> wgpu::Limits {
        wgpu::Limits::default().using_minimum_supported_acceleration_structure_values()
    }
    fn adapter(
        backend: wgpu::Backend,
        vendor: u32,
        device_type: wgpu::DeviceType,
    ) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: "synthetic capability input; not a hardware execution".into(),
            vendor,
            device: 123,
            device_type,
            backend,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            subgroup_min_size: 4,
            subgroup_max_size: 32,
            transient_saves_memory: false,
        }
    }
    #[test]
    fn native_backends_are_capability_driven_across_vendor_identifiers() {
        // PCI-style identifiers are metadata; none participates in selection.
        // This matrix exercises policy, not these manufacturers' physical GPUs.
        for vendor in [0x10de, 0x1002, 0x8086, 0x106b, 0x9999] {
            for backend in [
                wgpu::Backend::Vulkan,
                wgpu::Backend::Dx12,
                wgpu::Backend::Metal,
            ] {
                for kind in [
                    wgpu::DeviceType::DiscreteGpu,
                    wgpu::DeviceType::IntegratedGpu,
                ] {
                    let info = adapter(backend, vendor, kind);
                    let available = limits();
                    let requested = negotiate_limits(&available).unwrap();
                    let enabled = report_values(
                        &info,
                        wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                        &available,
                        Some((wgpu::Features::EXPERIMENTAL_RAY_QUERY, &requested)),
                    );
                    assert!(enabled.available && enabled.enabled);
                    assert!(enabled.reason.is_none());
                    assert_eq!(enabled.dedicated_rt_hardware_verified, None);
                    let missing = report_values(&info, wgpu::Features::empty(), &available, None);
                    assert!(!missing.available && !missing.enabled);
                }
            }
        }
    }
    #[test]
    fn feature_advertisement_is_not_device_enablement() {
        let info = adapter(
            wgpu::Backend::Metal,
            0x106b,
            wgpu::DeviceType::IntegratedGpu,
        );
        let l = limits();
        let supported = report_values(&info, wgpu::Features::EXPERIMENTAL_RAY_QUERY, &l, None);
        assert!(supported.available && !supported.enabled);
        let raster = report_values(
            &info,
            wgpu::Features::EXPERIMENTAL_RAY_QUERY,
            &l,
            Some((wgpu::Features::empty(), &wgpu::Limits::downlevel_defaults())),
        );
        assert!(raster.available && !raster.enabled);
        assert!(raster.reason.unwrap().contains("did not request"));
        let no_limits = report_values(
            &info,
            wgpu::Features::EXPERIMENTAL_RAY_QUERY,
            &l,
            Some((
                wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                &wgpu::Limits::downlevel_defaults(),
            )),
        );
        assert!(no_limits.available && !no_limits.enabled);
        assert!(no_limits.reason.unwrap().contains("device"));
    }
    #[test]
    fn software_or_unsupported_backends_cannot_be_called_hardware() {
        let l = limits();
        for backend in [
            wgpu::Backend::Gl,
            wgpu::Backend::BrowserWebGpu,
            wgpu::Backend::Noop,
        ] {
            let info = adapter(backend, 0x10de, wgpu::DeviceType::DiscreteGpu);
            assert!(
                !report_values(&info, wgpu::Features::EXPERIMENTAL_RAY_QUERY, &l, None).available
            );
        }
        let info = adapter(wgpu::Backend::Vulkan, 0, wgpu::DeviceType::Cpu);
        let report = report_values(&info, wgpu::Features::EXPERIMENTAL_RAY_QUERY, &l, None);
        assert!(!report.available);
        assert!(report.reason.unwrap().contains("CPU/software"));
    }
    #[test]
    fn every_acceleration_limit_is_required_and_requests_are_bounded() {
        for index in 0..4 {
            let mut l = limits();
            match index {
                0 => l.max_blas_primitive_count = 0,
                1 => l.max_blas_geometry_count = 0,
                2 => l.max_tlas_instance_count = 0,
                _ => l.max_acceleration_structures_per_shader_stage = 0,
            }
            assert!(negotiate_limits(&l).is_err());
        }
        let mut adapter = limits();
        adapter.max_blas_primitive_count = 17;
        let request = negotiate_limits(&adapter).unwrap();
        assert_eq!(request.max_blas_primitive_count, 17);
        assert_eq!(request.max_blas_geometry_count, 1);
        assert_eq!(request.max_tlas_instance_count, 1);
        assert_eq!(request.max_acceleration_structures_per_shader_stage, 1);
        assert!(request.check_limits(&adapter));
        assert_eq!(
            negotiate_limits(&limits())
                .unwrap()
                .max_blas_primitive_count,
            MAX_RT_PRIMITIVES
        );
    }
}
