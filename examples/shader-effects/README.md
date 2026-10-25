# Shader presets and developer WGSL

Script-free native project; left to right: Fog, Fresnel, Dissolve, developer waves.
The UV quad GLB and WGSL are self-authored; no fonts or game assets are required.
Material settings live in project.json. Animation uses the saved world clock.

```sh
cargo run --release -p crysirror-desktop -- shader-check examples/shader-effects/project.json \
  --client game --asset waves.wgsl
cargo run --release -p crysirror-desktop -- render examples/shader-effects/project.json \
  --frames 121 --step-ms 16 --output dist/shader-effects.png
```

The first command compiles the public surface ABI and the assembled color,
prepass and shadow WGSL without creating a GPU. Actual rendering separately
compiles pipelines for the GPU. MCP exposes the same check as `shader_validate`.
Restart the host to pick up changed code assets; there is no hot reload yet.
Bump project content_revision when changing code semantics that affect saves.

Preset material:

```json
{
  "alpha_mode": "blend", "unlit": true, "double_sided": true,
  "base_color_factor": [0.6, 0.8, 1.0, 0.8], "depth_fade": 2,
  "effect": {"kind": "fog", "density": 2, "noise_scale": 4,
             "speed": [0.03, 0.01], "edge_fade": 0.15}
}
```

Fog is an animated surface/sheet effect with UV noise and edge fade; combine
with `depth_fade` for soft intersections. It is not global/volumetric fog.
Fresnel adds view-dependent emission (`power`, `intensity`); Dissolve changes
coverage and edge emission (`threshold`, `edge_width`, `edge_color`). Fog requires
a transparent alpha mode; Dissolve requires mask or transparency. Authored UVs
are required for Fog. Raw cube/triangle UVs are zero.

Custom material (same JSON, Rust or `Entity.setMaterial` path):

```json
{"alpha_mode":"blend","unlit":true,"effect":{
  "kind":"custom","asset":"waves.wgsl",
  "parameters":[[0.7,0.04,0.8,0.3],[0,0,0,0],[0,0,0,0],[0,0,0,0]]
}}
```

Code remains a separate file under that client's asset root:

```wgsl
fn crysirror_surface(i: SurfaceInput) -> SurfaceOutput {
    var o = surface_default(i);
    let wave = 0.5 + 0.5*sin(i.uv.y*24.0 - i.time*2.0);
    o.color = vec4<f32>(mix(i.color.rgb, i.parameters[0].rgb, wave), i.color.a);
    return o;
}
```

ABI v1 inputs: transformed UV0, world position/geometric normal, view direction, simulation
time in seconds, sampled linear color/opacity, emission, roughness, metallic and
four vec4 parameters. Outputs: `color`, `emission`, `roughness`, `metallic`.
The engine samples textures before this hook and owns lighting, alpha mode,
cutoff, depth, shadows, skinning and compositing. The hook runs in all coverage
passes. Use `unlit` when the custom color should bypass PBR lighting.

Declare the function and optional pure helpers/constants; no entry points or
global variables/resource bindings. WGSL parsing and Naga validation report
stage/source diagnostics. Limits: 64 KiB source, 64 functions and 256 types
(including ABI helpers/types), 32 scoped custom modules per renderer. This first
interface does not replace vertex, fullscreen or compute passes, add custom
texture bindings, or implement a shader graph/hot reload. Developer GPU code
has no execution-time budget supplied by this ABI.

The ABI definitions are in
[crysirror-wgpu/src/shaders/surface_abi.wgsl](../../crates/crysirror-wgpu/src/shaders/surface_abi.wgsl).
Implementation and actual validation: [ADR 0016](../../docs/adr/0016-shader-effects.md).
