// SPDX-License-Identifier: MPL-2.0
// Temporal anti-aliasing resolve: reprojection + YCoCg neighbourhood clamp.
struct Taa {
    // xy 1/size, z history valid, w current-frame weight
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> taa: Taa;
@group(0) @binding(1) var current: texture_2d<f32>;
@group(0) @binding(2) var taa_sampler: sampler;
@group(0) @binding(3) var history: texture_2d<f32>;
@group(0) @binding(4) var velocity: texture_2d<f32>;

fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}
fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}
// Tonemap-weighted space reduces flicker from very bright samples.
fn compress(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + max(c.r, max(c.g, c.b)));
}
fn expand(c: vec3<f32>) -> vec3<f32> {
    return c / max(1.0 - max(c.r, max(c.g, c.b)), 1e-4);
}

@fragment
fn fs_taa(i: FsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i.clip.xy);
    let cur = compress(textureLoad(current, px, 0).rgb);
    if taa.params.z < 0.5 {
        return vec4<f32>(expand(cur), 1.0);
    }
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    var best = 0.0;
    var vel = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let q = px + vec2<i32>(x, y);
            let c = to_ycocg(compress(textureLoad(current, q, 0).rgb));
            lo = min(lo, c);
            hi = max(hi, c);
            // Longest motion in the neighbourhood keeps edges coherent.
            let v = textureLoad(velocity, q, 0).xy;
            if dot(v, v) > best {
                best = dot(v, v);
                vel = v;
            }
        }
    }
    let prev_uv = i.uv - vel;
    if any(prev_uv < vec2<f32>(0.0)) || any(prev_uv > vec2<f32>(1.0)) {
        return vec4<f32>(expand(cur), 1.0);
    }
    let hist = to_ycocg(compress(textureSampleLevel(history, taa_sampler, prev_uv, 0.0).rgb));
    let clamped = from_ycocg(clamp(hist, lo, hi));
    let out = mix(clamped, cur, taa.params.w);
    return vec4<f32>(expand(out), 1.0);
}
