// SPDX-License-Identifier: MPL-2.0
// Hemisphere SSAO from the prepass, then a depth-aware 4x4 blur.
struct Ssao {
    proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    // x radius (view units), y bias, z intensity, w frame index
    params: vec4<f32>,
    // xy size, zw 1/size
    screen: vec4<f32>,
};
@group(0) @binding(0) var<uniform> ssao: Ssao;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var normal_tex: texture_2d<f32>;

fn view_pos(px: vec2<i32>) -> vec3<f32> {
    let d = textureLoad(depth_tex, px, 0);
    let uv = (vec2<f32>(px) + vec2<f32>(0.5)) * ssao.screen.zw;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let v = ssao.inv_proj * ndc;
    return v.xyz / v.w;
}
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_ssao(i: FsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i.clip.xy);
    let d = textureLoad(depth_tex, px, 0);
    if d >= 1.0 {
        return vec4<f32>(1.0);
    }
    let p = view_pos(px);
    let n = normalize(textureLoad(normal_tex, px, 0).xyz);
    let noise = ign(i.clip.xy + vec2<f32>(ssao.params.w * 5.588238));
    let angle = noise * 2.0 * PI;
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.y) < 0.99);
    var t = normalize(cross(up, n));
    t = t * cos(angle) + cross(n, t) * sin(angle);
    let b = cross(n, t);
    let radius = ssao.params.x;
    var occ = 0.0;
    let count = 16u;
    for (var k = 0u; k < count; k = k + 1u) {
        let xi = vec2<f32>(fract(f32(k) * 0.618034 + noise), radical_inverse(k));
        let r = sqrt(xi.x);
        let phi = 2.0 * PI * xi.y;
        var dir = t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(0.0, 1.0 - xi.x));
        let scale = mix(0.1, 1.0, (f32(k) / f32(count)) * (f32(k) / f32(count)));
        let s = p + dir * radius * scale;
        let c = ssao.proj * vec4<f32>(s, 1.0);
        let suv = c.xy / c.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
        if any(suv < vec2<f32>(0.0)) || any(suv > vec2<f32>(1.0)) {
            continue;
        }
        let sp = view_pos(vec2<i32>(suv * ssao.screen.xy));
        let range = smoothstep(0.0, 1.0, radius / max(abs(p.z - sp.z), 1e-4));
        occ = occ + select(0.0, range, sp.z >= s.z + ssao.params.y);
    }
    let ao = clamp(1.0 - occ / f32(count) * ssao.params.z, 0.0, 1.0);
    return vec4<f32>(ao, ao, ao, 1.0);
}

@group(0) @binding(3) var ao_raw: texture_2d<f32>;
@fragment
fn fs_blur(i: FsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i.clip.xy);
    let size = vec2<i32>(ssao.screen.xy);
    let center = view_pos(px).z;
    var sum = 0.0;
    var w = 0.0;
    for (var y = -2; y < 2; y = y + 1) {
        for (var x = -2; x < 2; x = x + 1) {
            let q = clamp(px + vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
            let dz = abs(view_pos(q).z - center);
            let wt = 1.0 / (1e-3 + dz * dz * 50.0);
            sum = sum + textureLoad(ao_raw, q, 0).r * wt;
            w = w + wt;
        }
    }
    let ao = sum / w;
    return vec4<f32>(ao, ao, ao, 1.0);
}
