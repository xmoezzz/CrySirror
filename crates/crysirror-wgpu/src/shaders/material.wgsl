// SPDX-License-Identifier: MPL-2.0
struct Material {
    base_color: vec4<f32>,
    // rgb emissive factor, a: emissive strength
    emissive: vec4<f32>,
    // metallic, roughness, normal scale, occlusion strength
    params: vec4<f32>,
    // x: opaque=0/mask=1/alpha=2/additive=3/premultiplied=4, y: cutoff, z: unlit, w: opacity channel alpha
    alpha: vec4<f32>,
    uv_transform: vec4<f32>,
    // x: depth fade, y: effect kind, z: normal encoding (0 rgb, 1 rg, 2 ag), w: specular factor
    surface:vec4<f32>,
    effect_params:array<vec4<f32>,4>,
};
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var t_base: texture_2d<f32>;
@group(1) @binding(2) var t_mr: texture_2d<f32>;
@group(1) @binding(3) var t_normal: texture_2d<f32>;
@group(1) @binding(4) var t_occlusion: texture_2d<f32>;
@group(1) @binding(5) var t_emissive: texture_2d<f32>;
@group(1) @binding(6) var s_material: sampler;
@group(1) @binding(7) var t_opacity: texture_2d<f32>;
@group(1) @binding(8) var t_specular: texture_2d<f32>;

// Tangent-space normal in [-1,1] from the material's encoding.
fn decode_normal(t: vec4<f32>) -> vec3<f32> {
    if material.surface.z < 0.5 {
        return t.xyz * 2.0 - vec3<f32>(1.0);
    }
    var xy = t.xy * 2.0 - vec2<f32>(1.0);
    if material.surface.z > 1.5 {
        xy = vec2<f32>(t.a, t.g) * 2.0 - vec2<f32>(1.0);
    }
    return vec3<f32>(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}
fn material_uv(uv:vec2<f32>)->vec2<f32>{return uv*material.uv_transform.xy+material.uv_transform.zw;}
fn opacity_channel(v:vec4<f32>)->f32{return select(v.r,v.a,material.alpha.w>0.5);}

