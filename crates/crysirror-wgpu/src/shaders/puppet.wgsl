// SPDX-License-Identifier: MPL-2.0
// 2D puppet parts (ADR 0029): textured meshes rendered premultiplied into
// the layer's own target, with stencil masks, then un-premultiplied for the
// layer compositor (which expects straight alpha).
struct In {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@vertex fn vs_part(i: In) -> Out {
    var o: Out;
    o.position = vec4<f32>(i.position, 0.0, 1.0);
    o.uv = i.uv;
    o.color = i.color;
    return o;
}
@fragment fn fs_part(i: Out) -> @location(0) vec4<f32> {
    let c = textureSample(image, image_sampler, i.uv) * i.color;
    return vec4<f32>(c.rgb * c.a, c.a);
}
// Mask coverage: stencil only, where the mask part is substantially opaque.
@fragment fn fs_mask(i: Out) -> @location(0) vec4<f32> {
    let a = textureSample(image, image_sampler, i.uv).a * i.color.a;
    if a < 0.5 {
        discard;
    }
    return vec4<f32>(0.0);
}

@vertex fn vs_full(@builtin(vertex_index) v: u32) -> Out {
    let p = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
    var o: Out;
    o.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    o.color = vec4<f32>(1.0);
    return o;
}
@fragment fn fs_unpremultiply(i: Out) -> @location(0) vec4<f32> {
    let c = textureLoad(image, vec2<i32>(i.position.xy), 0);
    if c.a <= 1e-5 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(c.rgb / c.a, c.a);
}
