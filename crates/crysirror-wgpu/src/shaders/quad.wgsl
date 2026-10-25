struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) filter_index: u32,
}
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@vertex fn vs_main(@location(0) position: vec4<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) filter_slot: f32) -> Out {
    var out: Out;
    out.position = position;
    out.uv = uv;
    out.color = color;
    out.filter_index = u32(max(filter_slot, 0.0));
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, in.uv) * in.color;
}

@fragment fn fs_premultiply(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(image, image_sampler, in.uv) * in.color;
    return vec4<f32>(c.rgb * c.a, c.a);
}

// ---- Layer filters (ADR 0021): the Siglus/tona3 sprite effect stages. ----
struct LayerFilter {
    // mono, invert, bright, dark
    a: vec4<f32>,
    // color rate, target rgb
    b: vec4<f32>,
    // add rgb, flags (1 tone curve, 2 mask, 4 gray alpha, 8 alpha test)
    c: vec4<f32>,
    // curve row, curve saturation, distortion kind (0 none, 1 mosaic, 2 raster h,
    // 3 raster v, 4 radial blur, 5 threshold fade, 6 inverted threshold), p0
    d: vec4<f32>,
    // p1, p2, p3, p4
    e: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> filters: array<LayerFilter>;
@group(1) @binding(1) var mask_image: texture_2d<f32>;
@group(1) @binding(2) var curve_image: texture_2d<f32>;

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}
// Out-of-texture taps are transparent (Siglus sample_tex0_safe).
fn tap(uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(image, image_sampler, uv, 0.0);
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    return select(vec4<f32>(0.0), vec4<f32>(to_gamma(c.rgb), c.a), inside);
}
fn raster_amp(progress: f32) -> f32 {
    let rp = clamp(1.0 - progress, 1e-4, 1.0);
    let lv = max((1.0 - rp) * 100.0, 1e-4);
    return 1.0 - ((log(lv) / log(10.0)) + 1.0) / 3.0;
}
fn distorted(uv: vec2<f32>, f: LayerFilter) -> vec4<f32> {
    let kind = f.d.z;
    let p0 = f.d.w;
    let p1 = f.e.x;
    let p2 = f.e.y;
    let p3 = f.e.z;
    if kind > 0.5 && kind < 1.5 {
        let cu = max(p0, 1e-5);
        let cv = max(p0 * max(p1, 1e-5), 1e-5);
        var c = tap(vec2<f32>(floor(uv.x / cu) * cu, floor(uv.y / cv) * cv));
        c.a = 1.0;
        return c;
    }
    if kind > 1.5 && kind < 3.5 {
        let vertical = kind > 2.5;
        let fnn = max(p0, 1.0);
        var t = fract(select(uv.y, uv.x, vertical) * fnn);
        t = (t - fnn * 0.1) / fnn;
        let o = sin(3.14159265 * p3 * p2 + t * 3.14159265 * p1) * raster_amp(p3);
        return tap(select(vec2<f32>(uv.x + o, uv.y), vec2<f32>(uv.x, uv.y + o), vertical));
    }
    if kind > 3.5 && kind < 4.5 {
        let dims = vec2<f32>(textureDimensions(image, 0));
        let texel = 1.0 / max(max(dims.x, dims.y), 1.0);
        var dir = vec2<f32>(p0, p1) - uv;
        let len = length(dir);
        if len <= 1e-5 || p2 <= 1e-5 {
            return tap(uv);
        }
        dir = normalize(dir) * texel * p2 * len * max(p3, 0.0);
        var w = array<f32, 10>(0.19, 0.17, 0.15, 0.13, 0.11, 0.09, 0.07, 0.05, 0.03, 0.01);
        var c = vec4<f32>(0.0);
        for (var i = 0; i < 10; i = i + 1) {
            c = c + tap(uv + dir * f32(i)) * w[i];
        }
        c.a = 1.0;
        return c;
    }
    var c = tap(uv);
    if kind > 4.5 {
        let y = dot(vec3<f32>(0.2989, 0.5886, 0.1145), c.rgb);
        let hit = select(y <= p1, y >= p1, kind > 5.5);
        if hit {
            c.a = c.a * p0;
        }
    }
    return c;
}
fn flag(f: LayerFilter, bit: u32) -> bool {
    return (u32(f.c.w) & bit) != 0u;
}
fn filtered(in: Out) -> vec4<f32> {
    let f = filters[in.filter_index];
    let tint = vec4<f32>(to_gamma(in.color.rgb), in.color.a);
    var color = distorted(in.uv, f) * tint;
    let org_a = color.a;
    let mono_y = dot(color.rgb, vec3<f32>(0.2989, 0.5886, 0.1145));
    if flag(f, 1u) {
        var g = mix(color.rgb, vec3<f32>(mono_y), f.d.y);
        g = clamp(g, vec3<f32>(0.0), vec3<f32>(1.0));
        let row = f.d.x;
        let r = to_gamma(textureSampleLevel(curve_image, image_sampler, vec2<f32>(g.r, row), 0.0).rgb).r;
        let gg = to_gamma(textureSampleLevel(curve_image, image_sampler, vec2<f32>(g.g, row), 0.0).rgb).g;
        let b = to_gamma(textureSampleLevel(curve_image, image_sampler, vec2<f32>(g.b, row), 0.0).rgb).b;
        color = vec4<f32>(r, gg, b, color.a);
    }
    color = vec4<f32>(mix(color.rgb, vec3<f32>(1.0) - color.rgb, f.a.y), color.a);
    color = vec4<f32>(mix(color.rgb, vec3<f32>(mono_y), f.a.x), color.a);
    color = vec4<f32>(color.rgb + vec3<f32>(f.a.z) - vec3<f32>(f.a.w), color.a);
    color = vec4<f32>(mix(color.rgb, f.b.yzw, f.b.x), color.a);
    color = vec4<f32>(color.rgb + f.c.xyz, org_a);
    let final_gray = dot(color.rgb, vec3<f32>(0.2989, 0.5886, 0.1145));
    if flag(f, 2u) {
        let m = textureSampleLevel(mask_image, image_sampler, in.uv, 0.0);
        color = color * vec4<f32>(to_gamma(m.rgb), m.a);
    }
    if flag(f, 4u) {
        color.a = final_gray;
    }
    if flag(f, 8u) && color.a < 1.0 / 255.0 {
        discard;
    }
    return vec4<f32>(to_linear(color.rgb), clamp(color.a, 0.0, 1.0));
}
@fragment fn fs_filtered(in: Out) -> @location(0) vec4<f32> {
    return filtered(in);
}
@fragment fn fs_filtered_premultiply(in: Out) -> @location(0) vec4<f32> {
    let c = filtered(in);
    return vec4<f32>(c.rgb * c.a, c.a);
}
