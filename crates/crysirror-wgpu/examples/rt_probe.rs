// SPDX-License-Identifier: MPL-2.0
use crysirror_wgpu::{Gpu, RenderingPath};
fn main() -> anyhow::Result<()> {
    let instance = wgpu::Instance::default();
    let reports: Vec<_> = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .iter()
        .map(|a| crysirror_wgpu::rt_capabilities::report(a, None))
        .collect();
    println!("{}", serde_json::to_string_pretty(&reports)?);
    let gpu = pollster::block_on(Gpu::request_for(
        &instance,
        None,
        RenderingPath::HardwareRaytrace,
    ))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&gpu.hardware_rt_report())?
    );
    Ok(())
}
