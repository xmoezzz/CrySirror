// SPDX-License-Identifier: MPL-2.0
// Screen-space reflections (ADR 0023). Own implementation: a mirror ray from
// the prepass view position/normal is marched in screen space with
// perspective-correct depth, refined by bisection, and the hit is
// reprojected into the previous frame's HDR color. Output: rgb radiance,
// a = confidence (0 = no hit; the forward pass keeps the environment).
struct Ssr {
    proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    // View space of this frame → clip space of the previous frame.
    reproject: mat4x4<f32>,
    // x max distance, y thickness, z steps, w previous frame valid
    params: vec4<f32>,
    // xy size, zw 1/size
    screen: vec4<f32>,
    // x frame index (noise), y near plane
    misc: vec4<f32>,
};
@group(0) @binding(0) var<uniform> ssr: Ssr;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var normal_tex: texture_2d<f32>;
@group(0) @binding(3) var prev_color: texture_2d<f32>;
@group(0) @binding(4) var prev_sampler: sampler;

fn view_at(uv: vec2<f32>, d: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let v = ssr.inv_proj * ndc;
    return v.xyz / v.w;
}
fn depth_at(uv: vec2<f32>) -> f32 {
    let px = clamp(vec2<i32>(uv * ssr.screen.xy), vec2<i32>(0), vec2<i32>(ssr.screen.xy) - vec2<i32>(1));
    return textureLoad(depth_tex, px, 0);
}
fn pixel(uv: vec2<f32>) -> vec2<i32> {
    return clamp(vec2<i32>(uv * ssr.screen.xy), vec2<i32>(0), vec2<i32>(ssr.screen.xy) - vec2<i32>(1));
}
fn to_screen(p: vec3<f32>) -> vec3<f32> {
    let c = ssr.proj * vec4<f32>(p, 1.0);
    let n = c.xy / c.w;
    // uv (y down) and 1/w for perspective-correct interpolation.
    return vec3<f32>(n.x * 0.5 + 0.5, 0.5 - n.y * 0.5, 1.0 / c.w);
}
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_ssr(i: FsOut) -> @location(0) vec4<f32> {
    if ssr.params.w < 0.5 {
        return vec4<f32>(0.0);
    }
    let px = vec2<i32>(i.clip.xy);
    let d0 = textureLoad(depth_tex, px, 0);
    if d0 >= 1.0 {
        return vec4<f32>(0.0);
    }
    let uv0 = (vec2<f32>(px) + vec2<f32>(0.5)) * ssr.screen.zw;
    let p0 = view_at(uv0, d0);
    let n = normalize(textureLoad(normal_tex, px, 0).xyz);
    let r = reflect(normalize(p0), n);
    // Clip the ray to the near plane (view space looks down -z).
    var len = ssr.params.x;
    let near = ssr.misc.y;
    if r.z > 0.0 {
        len = min(len, (-near * 1.01 - p0.z) / r.z);
    }
    if len <= 0.0 {
        return vec4<f32>(0.0);
    }
    let p1 = p0 + r * len;
    let s0 = to_screen(p0 + n * 0.01);
    let s1 = to_screen(p1);
    let steps = max(ssr.params.z, 4.0);
    let jitter = ign(i.clip.xy + vec2<f32>(ssr.misc.x * 5.588238));
    var lo = 0.0;
    var hit = -1.0;
    let thickness = ssr.params.y;
    for (var k = 1.0; k <= steps; k = k + 1.0) {
        let t = (k - 1.0 + jitter) / steps;
        let s = mix(s0, s1, t);
        if any(s.xy < vec2<f32>(0.0)) || any(s.xy > vec2<f32>(1.0)) {
            break;
        }
        let scene_d = depth_at(s.xy);
        // Crossings next to the origin or on surfaces facing along the ray are
        // self-intersections of the start surface; keep marching past them.
        let away = length((s.xy - s0.xy) * ssr.screen.xy) > 1.5;
        if away && scene_d < 1.0 && (1.0 / s.z) > -view_at(s.xy, scene_d).z
            && dot(normalize(textureLoad(normal_tex, pixel(s.xy), 0).xyz), r) < 0.2 {
            // Crossed behind a surface: bisect between the last miss and here,
            // then apply the thickness test at the refined point.
            var a = lo;
            var b = t;
            for (var j = 0; j < 6; j = j + 1) {
                let m = 0.5 * (a + b);
                let sm = mix(s0, s1, m);
                let dm = depth_at(sm.xy);
                if dm < 1.0 && (1.0 / sm.z) > -view_at(sm.xy, dm).z { b = m; } else { a = m; }
            }
            let sb = mix(s0, s1, b);
            let db = depth_at(sb.xy);
            if db < 1.0 && (1.0 / sb.z) + view_at(sb.xy, db).z < thickness {
                hit = b;
                break;
            }
        }
        lo = t;
    }
    if hit < 0.0 {
        return vec4<f32>(0.0);
    }
    let b = hit;
    let s = mix(s0, s1, b);
    let hit_view = view_at(s.xy, depth_at(s.xy));
    let pc = ssr.reproject * vec4<f32>(hit_view, 1.0);
    let puv = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
    if pc.w <= 0.0 || any(puv < vec2<f32>(0.0)) || any(puv > vec2<f32>(1.0)) {
        return vec4<f32>(0.0);
    }
    let color = textureSampleLevel(prev_color, prev_sampler, puv, 0.0).rgb;
    // Confidence fades near screen borders, with world distance and toward the camera.
    let edge = smoothstep(vec2<f32>(0.0), vec2<f32>(0.08), puv) * smoothstep(vec2<f32>(0.0), vec2<f32>(0.08), vec2<f32>(1.0) - puv);
    let travel = clamp(length(hit_view - p0) / ssr.params.x, 0.0, 1.0);
    let fade = edge.x * edge.y * (1.0 - travel * travel) * clamp(1.0 - r.z * 2.0, 0.0, 1.0);
    return vec4<f32>(max(color, vec3<f32>(0.0)), clamp(fade, 0.0, 1.0));
}
