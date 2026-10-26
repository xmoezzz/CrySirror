// SPDX-License-Identifier: MPL-2.0
// Appended after material.wgsl (group 0), the surface ABI/presets and
// surface_sample.wgsl: bakes the exact raster coverage alpha of a material
// over its untransformed UV square into one alpha-atlas tile.
struct Bake { rect:vec4<f32> }
@group(1) @binding(0) var<uniform> bake:Bake;
@vertex fn vs_bake(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return vec4<f32>(p*2.0-1.0,0.0,1.0);
}
@fragment fn fs_bake(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    let uv=(position.xy-bake.rect.xy)/bake.rect.zw;
    let s=sample_surface(uv,vec3<f32>(0.0),vec3<f32>(0.0,1.0,0.0),vec3<f32>(0.0,1.0,0.0),0.0);
    return vec4<f32>(s.color.a);
}
