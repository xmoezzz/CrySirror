// SPDX-License-Identifier: MPL-2.0
// Irradiance probe update (ADR 0028): project the six captured faces onto
// L1 spherical harmonics, convolved with the clamped cosine, and store them
// in the shadow-data buffer's GI section (4 vec4 per probe: r, g, b, state).
// The capture alpha is the signed hit distance (negative: back face, 0:
// miss). From it, each probe also gets 8×8 octahedral distance moments
// (mean, mean²) for Chebyshev visibility, and probes that see mostly back
// faces (inside geometry) are switched off.
// Faces use the probes::FACES bases (forward, up), as probe.wgsl does.
struct Job {
    // x probe index, y first vec4 of the GI section, z first vec4 of the
    // visibility section
    probe: vec4<u32>,
    faces: array<vec4<f32>, 12>,
    // x distance clamp (misses count as this far), y back-face fraction
    // above which the probe is disabled
    params: vec4<f32>,
};
@group(0) @binding(0) var faces_tex: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> data: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> job: Job;

const PI: f32 = 3.14159265359;
const OCT: u32 = 8u;
const SUB: u32 = 4u;
var<workgroup> acc: array<array<vec4<f32>, 3>, 64>;
var<workgroup> wacc: array<vec2<f32>, 64>;
var<workgroup> moments: array<vec2<f32>, 64>;

fn face_dir(k: u32, x: f32, y: f32) -> vec3<f32> {
    let f = job.faces[2u * k].xyz;
    let up = job.faces[2u * k + 1u].xyz;
    return f + cross(f, up) * x + up * y;
}

fn sign_nz(v: vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(-1.0), vec2<f32>(1.0), v >= vec2<f32>(0.0));
}
fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var v = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if v.z < 0.0 {
        let xy = (1.0 - abs(v.yx)) * sign_nz(v.xy);
        v = vec3<f32>(xy, v.z);
    }
    return normalize(v);
}

// Signed hit distance stored by the capture in direction d.
fn capture_alpha(d: vec3<f32>, n: u32) -> f32 {
    var best = 0u;
    var best_dot = -2.0;
    for (var k = 0u; k < 6u; k = k + 1u) {
        let c = dot(d, job.faces[2u * k].xyz);
        if c > best_dot {
            best_dot = c;
            best = k;
        }
    }
    let f = job.faces[2u * best].xyz;
    let up = job.faces[2u * best + 1u].xyz;
    let q = d / dot(d, f);
    let x = dot(q, cross(f, up));
    let y = dot(q, up);
    let u = clamp(u32((x * 0.5 + 0.5) * f32(n)), 0u, n - 1u);
    let v = clamp(u32((0.5 - y * 0.5) * f32(n)), 0u, n - 1u);
    return textureLoad(faces_tex, vec2<i32>(i32(u), i32(v)), i32(best), 0).a;
}

@compute @workgroup_size(64)
fn cs_project(@builtin(local_invocation_index) li: u32) {
    let n = textureDimensions(faces_tex).x;
    let per_face = n * n;
    var r = vec4<f32>(0.0);
    var g = vec4<f32>(0.0);
    var b = vec4<f32>(0.0);
    var w = 0.0;
    var back = 0.0;
    for (var i = li; i < 6u * per_face; i = i + 64u) {
        let k = i / per_face;
        let j = i % per_face;
        let u = j % n;
        let v = j / n;
        let x = (f32(u) + 0.5) / f32(n) * 2.0 - 1.0;
        let y = 1.0 - (f32(v) + 0.5) / f32(n) * 2.0;
        let dir = face_dir(k, x, y);
        let len2 = dot(dir, dir);
        let d = dir * inverseSqrt(len2);
        // Solid angle of a cube-face texel ∝ 1 / (1 + x² + y²)^(3/2).
        let dw = 1.0 / (len2 * sqrt(len2));
        let s = textureLoad(faces_tex, vec2<i32>(i32(u), i32(v)), i32(k), 0);
        let basis = vec4<f32>(0.282095, 0.488603 * d);
        r = r + basis * s.r * dw;
        g = g + basis * s.g * dw;
        b = b + basis * s.b * dw;
        w = w + dw;
        back = back + select(0.0, dw, s.a < 0.0);
    }
    acc[li] = array<vec4<f32>, 3>(r, g, b);
    wacc[li] = vec2<f32>(w, back);

    // Distance moments of octahedral texel li over SUB×SUB directions.
    let tx = li % OCT;
    let ty = li / OCT;
    var m = vec2<f32>(0.0);
    for (var s = 0u; s < SUB * SUB; s = s + 1u) {
        let e = (vec2<f32>(f32(tx), f32(ty)) + (vec2<f32>(f32(s % SUB), f32(s / SUB)) + 0.5) / f32(SUB)) / f32(OCT) * 2.0 - 1.0;
        let a = capture_alpha(oct_decode(e), n);
        let dist = select(min(abs(a), job.params.x), job.params.x, a == 0.0);
        m = m + vec2<f32>(dist, dist * dist);
    }
    moments[li] = m / f32(SUB * SUB);
    workgroupBarrier();
    let probe = job.probe.x;
    if li < 32u {
        data[job.probe.z + probe * 32u + li] = vec4<f32>(moments[2u * li], moments[2u * li + 1u]);
    }
    if li == 0u {
        var sum = array<vec4<f32>, 3>(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
        var ws = vec2<f32>(0.0);
        for (var t = 0u; t < 64u; t = t + 1u) {
            sum[0] = sum[0] + acc[t][0];
            sum[1] = sum[1] + acc[t][1];
            sum[2] = sum[2] + acc[t][2];
            ws = ws + wacc[t];
        }
        // Normalize the weights to 4π, then convolve: A0 = π, A1 = 2π/3.
        let norm = 4.0 * PI / ws.x;
        let conv = vec4<f32>(PI * 0.282095, vec3<f32>(2.0 * PI / 3.0 * 0.488603));
        let base = job.probe.y + probe * 4u;
        let back_fraction = ws.y / ws.x;
        data[base] = sum[0] * norm * conv;
        data[base + 1u] = sum[1] * norm * conv;
        data[base + 2u] = sum[2] * norm * conv;
        // x: usable, y: back-face fraction, z: refreshed at least once.
        data[base + 3u] = vec4<f32>(select(1.0, 0.0, back_fraction > job.params.y), back_fraction, 1.0, 0.0);
    }
}
