// SPDX-License-Identifier: MPL-2.0
//! WGSL surface ABI v1, compilation diagnostics and restricted resource boundary.
use anyhow::{ensure, Context, Result};
pub const SURFACE_ABI: &str = include_str!("shaders/surface_abi.wgsl");
pub(crate) const PRESETS: &str = include_str!("shaders/surface_presets.wgsl");
fn validate(source: &str, label: &str) -> Result<naga::Module> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| anyhow::anyhow!("{label}: {}", e.emit_to_string(source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| anyhow::anyhow!("{label}: {}", e.emit_to_string(source)))?;
    Ok(module)
}
pub fn validate_custom(source: &str) -> Result<()> {
    ensure!(
        !source.is_empty() && source.len() <= 65_536,
        "custom WGSL source must be 1..65536 bytes"
    );
    let module = validate(&format!("{SURFACE_ABI}\n{source}"), "custom surface ABI v1")?;
    ensure!(
        module.entry_points.is_empty() && module.global_variables.is_empty(),
        "custom surface functions cannot declare entry points or global variables/bindings"
    );
    ensure!(
        module.functions.len() <= 64 && module.types.len() <= 256,
        "custom surface module complexity limit"
    );
    let function = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("crysirror_surface"))
        .map(|(_, f)| f)
        .context("missing crysirror_surface function")?;
    ensure!(
        function.arguments.len() == 1
            && module.types[function.arguments[0].ty].name.as_deref() == Some("SurfaceInput")
            && function
                .result
                .as_ref()
                .is_some_and(|r| module.types[r.ty].name.as_deref() == Some("SurfaceOutput")),
        "custom ABI requires fn crysirror_surface(SurfaceInput) -> SurfaceOutput"
    );
    Ok(())
}
pub(crate) fn common(hook: &str) -> String {
    format!(
        "{}\n{}\n{SURFACE_ABI}\n{hook}\n{}",
        include_str!("shaders/pbr_common.wgsl"),
        include_str!("shaders/material.wgsl"),
        include_str!("shaders/surface_sample.wgsl")
    )
}
pub(crate) fn modules(custom: Option<&str>) -> Result<[String; 3]> {
    if let Some(s) = custom {
        validate_custom(s)?;
    }
    let hook = custom.unwrap_or(PRESETS);
    let common = common(hook);
    let color = format!("{common}\n{}", include_str!("shaders/pbr.wgsl"));
    let depth = format!("{common}\n{}", include_str!("shaders/prepass.wgsl"));
    let shadow = format!(
        "{}\n{SURFACE_ABI}\n{hook}\n{}\n{}",
        include_str!("shaders/material.wgsl").replace("@group(1)", "@group(2)"),
        include_str!("shaders/surface_sample.wgsl"),
        include_str!("shaders/shadow.wgsl")
    );
    for (label, source) in [("color", &color), ("prepass", &depth), ("shadow", &shadow)] {
        validate(source, label).with_context(|| format!("compile material {label}"))?;
    }
    Ok([color, depth, shadow])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_and_custom_compile_for_all_coverage_passes() {
        modules(None).unwrap();
        modules(Some("fn crysirror_surface(i:SurfaceInput)->SurfaceOutput {var o=surface_default(i);o.color=vec4<f32>(i.uv,abs(sin(i.time)),i.parameters[0].x);return o;}")).unwrap();
    }
    #[test]
    fn custom_errors_are_diagnostics_not_gpu_panics() {
        for source in ["garbage","fn other() {}","@group(7) @binding(0) var t:texture_2d<f32>; fn crysirror_surface(i:SurfaceInput)->SurfaceOutput{return surface_default(i);}","@compute @workgroup_size(1) fn main() {}","fn crysirror_surface(i:SurfaceInput)->f32{return 1.0;}"]{assert!(modules(Some(source)).is_err(),"{source}");}
        assert!(validate_custom(&" ".repeat(65_537)).is_err());
    }
}

/// Validate custom ABI and all color/depth/shadow modules without a GPU.
pub fn check(source: &str) -> Result<()> {
    modules(Some(source))?;
    Ok(())
}
