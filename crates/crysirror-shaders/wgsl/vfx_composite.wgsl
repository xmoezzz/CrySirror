// SPDX-License-Identifier: MPL-2.0
@group(0) @binding(0) var layer: texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let xy = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(xy * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
}

@fragment fn fs(@builtin(position) point: vec4<f32>) -> @location(0) vec4<f32> {
    let accumulated = textureLoad(layer, vec2<i32>(point.xy), 0);
    return vec4<f32>(accumulated.rgb, clamp(accumulated.a, 0.0, 1.0));
}
