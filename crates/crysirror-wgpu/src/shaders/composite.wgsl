@group(0) @binding(0) var before: texture_2d<f32>;
@group(0) @binding(1) var after: texture_2d<f32>;
struct Params {
    // x progress 0..1, y rule vague, z 1 when a rule wipe is active.
    a: vec4<f32>,
    // Destination root rect in canvas pixels [x, y, w, h].
    rect: vec4<f32>,
}
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var rule: texture_2d<f32>;
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
fn gamma(c: vec3<f32>) -> vec3<f32> {
    let x=clamp(c,vec3<f32>(0.0),vec3<f32>(1.0));
    return select(1.055*pow(x,vec3<f32>(1.0/2.4))-0.055,x*12.92,x<=vec3<f32>(0.0031308));
}
fn linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c+0.055)/1.055,vec3<f32>(2.4)),c/12.92,c<=vec3<f32>(0.04045));
}
// KiriKiri universal transition weight of the new image (krkrz fd5c4baa
// TVPInitUnivTransBlendTable): phase = progress*(255+vague), table value
// 255 below phase-vague, 0 at/above phase, linear between; blend uses /256.
fn rule_weight(pos: vec2<f32>) -> f32 {
    let progress = params.a.x;
    if (progress >= 1.0) { return 1.0; }
    let uv = (pos - params.rect.xy) / params.rect.zw;
    if (any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) { return progress; }
    let dims = vec2<f32>(textureDimensions(rule));
    let rp = vec2<i32>(clamp(floor(uv * dims), vec2<f32>(0.0), dims - 1.0));
    // Rule textures are sRGB-decoded on load; recover the stored 8-bit level.
    let g = gamma(textureLoad(rule, rp, 0).rgb);
    let level = floor((g.r + g.g + g.b) / 3.0 * 255.0 + 0.5);
    let vague = params.a.y;
    let phasemax = floor(progress * (255.0 + vague));
    let phase = phasemax - vague;
    var t = 0.0;
    if (level < phase) {
        t = 255.0;
    } else if (level < phasemax) {
        t = clamp(255.0 - floor((level - phase) * 255.0 / vague), 0.0, 255.0);
    }
    return t / 256.0;
}
@fragment fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let a=textureLoad(before,vec2<i32>(pos.xy),0);
    let b=textureLoad(after,vec2<i32>(pos.xy),0);
    var w = params.a.x;
    if (params.a.z > 0.5) { w = rule_weight(pos.xy); }
    let alpha=mix(a.a,b.a,w);
    // Inputs are renderer-composited premultiplied linear pixels. Mix the
    // isolated gamma-space colors and output straight alpha for normal layers.
    let color=mix(gamma(a.rgb/max(a.a,0.000001))*a.a,gamma(b.rgb/max(b.a,0.000001))*b.a,w);
    return vec4<f32>(linear(color/max(alpha,0.000001)),alpha);
}
