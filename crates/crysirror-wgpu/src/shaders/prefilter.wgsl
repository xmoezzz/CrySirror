// SPDX-License-Identifier: MPL-2.0
// ---- GGX prefiltering of an equirectangular environment ----
struct Prefilter {
    // x: roughness, y: source width, z: source mip count, w: unused
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> prefilter: Prefilter;
@group(0) @binding(1) var src_env: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

fn dir_from_uv(uv: vec2<f32>) -> vec3<f32> {
    let phi = (uv.x - 0.5) * 2.0 * PI;
    let theta = uv.y * PI;
    return vec3<f32>(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
}
fn uv_from_dir(d: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(atan2(d.z, d.x) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
}

@fragment
fn fs_prefilter(i: FsOut) -> @location(0) vec4<f32> {
    let n = dir_from_uv(i.uv);
    let roughness = prefilter.params.x;
    if roughness < 0.01 {
        return vec4<f32>(textureSampleLevel(src_env, src_sampler, i.uv, 0.0).rgb, 1.0);
    }
    let a = roughness * roughness;
    let width = prefilter.params.y;
    let max_mip = prefilter.params.z - 1.0;
    // Solid angle of one source texel at the equator.
    let sa_texel = 4.0 * PI / (width * width * 0.5);
    var color = vec3<f32>(0.0);
    var weight = 0.0;
    let count = 256u;
    for (var k = 0u; k < count; k = k + 1u) {
        let h = importance_ggx(vec2<f32>(f32(k) / f32(count), radical_inverse(k)), n, a);
        let l = normalize(2.0 * dot(n, h) * h - n);
        let nl = dot(n, l);
        if nl > 0.0 {
            let nh = max(dot(n, h), 0.0);
            let a2 = a * a;
            let d = nh * nh * (a2 - 1.0) + 1.0;
            let pdf = a2 / (PI * d * d) / 4.0 + 1e-4;
            let sa_sample = 1.0 / (f32(count) * pdf);
            let mip = clamp(0.5 * log2(sa_sample / sa_texel) + 1.0, 0.0, max_mip);
            color = color + textureSampleLevel(src_env, src_sampler, uv_from_dir(l), mip).rgb * nl;
            weight = weight + nl;
        }
    }
    return vec4<f32>(color / max(weight, 1e-4), 1.0);
}
