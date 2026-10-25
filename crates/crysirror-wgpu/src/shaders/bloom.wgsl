// SPDX-License-Identifier: MPL-2.0
// ---- Bloom (dual filter: 13-tap downsample, 9-tap tent upsample) ----
struct BloomStep {
    // xy: source texel size, z: 1 on the first (thresholded) downsample
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> bloom_step: BloomStep;
@group(0) @binding(1) var bloom_src: texture_2d<f32>;
@group(0) @binding(2) var bloom_sampler: sampler;

@fragment
fn fs_bloom_down(i: FsOut) -> @location(0) vec4<f32> {
    let t = bloom_step.params.xy;
    let uv = i.uv;
    var c = textureSampleLevel(bloom_src, bloom_sampler, uv, 0.0).rgb * 0.125;
    c = c + (textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-1.0, -1.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(1.0, -1.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-1.0, 1.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(1.0, 1.0), 0.0).rgb) * 0.125;
    c = c + (textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-2.0, -2.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(2.0, -2.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-2.0, 2.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(2.0, 2.0), 0.0).rgb) * 0.03125;
    c = c + (textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-2.0, 0.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(2.0, 0.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(0.0, -2.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(0.0, 2.0), 0.0).rgb) * 0.0625;
    if bloom_step.params.z > 0.5 {
        // Karis average against fireflies on the first mip; drop non-finite input.
        c = select(c, vec3<f32>(0.0), c != c || c > vec3<f32>(60000.0));
        c = c / (1.0 + max(c.r, max(c.g, c.b)));
    }
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_bloom_up(i: FsOut) -> @location(0) vec4<f32> {
    let t = bloom_step.params.xy;
    let uv = i.uv;
    var c = textureSampleLevel(bloom_src, bloom_sampler, uv, 0.0).rgb * 4.0;
    c = c + (textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-1.0, 0.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(1.0, 0.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(0.0, -1.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(0.0, 1.0), 0.0).rgb) * 2.0;
    c = c + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-1.0, -1.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(1.0, -1.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(-1.0, 1.0), 0.0).rgb
        + textureSampleLevel(bloom_src, bloom_sampler, uv + t * vec2<f32>(1.0, 1.0), 0.0).rgb;
    return vec4<f32>(c / 16.0, 1.0);
}
