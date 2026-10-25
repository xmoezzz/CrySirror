// SPDX-License-Identifier: MPL-2.0
// Public ABI v1. Pure surface functions; no engine bindings are exposed.
struct SurfaceInput {
    uv: vec2<f32>,
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    view_direction: vec3<f32>,
    time: f32,
    color: vec4<f32>,
    emission: vec3<f32>,
    roughness: f32,
    metallic: f32,
    parameters: array<vec4<f32>, 4>,
};
struct SurfaceOutput {
    color: vec4<f32>,
    emission: vec3<f32>,
    roughness: f32,
    metallic: f32,
};
fn surface_default(i: SurfaceInput) -> SurfaceOutput {
    return SurfaceOutput(i.color, i.emission, i.roughness, i.metallic);
}
