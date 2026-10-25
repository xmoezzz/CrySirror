// SPDX-License-Identifier: MPL-2.0
// ---- Tonemapping ----
struct Post {
    // x: exposure multiplier, y: tonemap (0 PBR Neutral, 1 ACES, 2 none), z: bloom strength
    params: vec4<f32>,
    // x: auto exposure enabled, y: compensation (stops)
    adapt: vec4<f32>,
};
@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var hdr_sampler: sampler;
@group(0) @binding(3) var bloom: texture_2d<f32>;
@group(0) @binding(4) var adapted: texture_2d<f32>;

fn pbr_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    var c = c_in;
    let x = min(c.r, min(c.g, c.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    c = c - vec3<f32>(offset);
    let peak = max(c.r, max(c.g, c.b));
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_tonemap(i: FsOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(hdr, hdr_sampler, i.uv, 0.0).rgb;
    c = mix(c, textureSampleLevel(bloom, hdr_sampler, i.uv, 0.0).rgb, post.params.z);
    c = select(c, vec3<f32>(0.0), c != c);
    c = c * post.params.x;
    if post.adapt.x > 0.5 {
        // Metered average → middle gray.
        let avg_log = textureLoad(adapted, vec2<i32>(0, 0), 0).r;
        c = c * (0.18 * exp2(post.adapt.y - avg_log));
    }
    var o: vec3<f32>;
    if post.params.y < 0.5 {
        o = pbr_neutral(c);
    } else if post.params.y < 1.5 {
        o = aces(c);
    } else {
        o = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    // The output view is sRGB: write linear values.
    return vec4<f32>(o, 1.0);
}
