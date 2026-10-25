// SPDX-License-Identifier: MPL-2.0
// Auto exposure: mean log2 luminance of the HDR frame on a 32×32 grid,
// adapted toward the clamped target and stored in a 1×1 R32Float texture.
struct ExposureParams {
    // x min log2 luminance, y max, z speed up (1/s), w speed down (1/s)
    range: vec4<f32>,
    // x dt seconds, y 1 when the previous state is valid
    time: vec4<f32>,
};
@group(0) @binding(0) var<uniform> ex: ExposureParams;
@group(0) @binding(1) var hdr_in: texture_2d<f32>;
@group(0) @binding(2) var previous: texture_2d<f32>;

@fragment
fn fs_exposure(i: FsOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(hdr_in));
    var sum = 0.0;
    var weight = 0.0;
    for (var y = 0u; y < 32u; y = y + 1u) {
        for (var x = 0u; x < 32u; x = x + 1u) {
            let uv = (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / 32.0;
            let c = textureLoad(hdr_in, vec2<i32>(uv * size), 0).rgb;
            let lum = dot(select(c, vec3<f32>(0.0), c != c), vec3<f32>(0.2126, 0.7152, 0.0722));
            // Center-weighted, as metering in most engines.
            let w = 1.0 - 0.5 * length(uv - vec2<f32>(0.5));
            sum = sum + log2(max(lum, 1e-5)) * w;
            weight = weight + w;
        }
    }
    let target_log = clamp(sum / weight, ex.range.x, ex.range.y);
    if ex.time.y < 0.5 {
        return vec4<f32>(target_log, 0.0, 0.0, 1.0);
    }
    let prev = textureLoad(previous, vec2<i32>(0, 0), 0).r;
    let speed = select(ex.range.w, ex.range.z, target_log > prev);
    let k = 1.0 - exp(-ex.time.x * speed);
    return vec4<f32>(prev + (target_log - prev) * k, 0.0, 0.0, 1.0);
}
