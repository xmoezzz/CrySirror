// SPDX-License-Identifier: MPL-2.0
// Virtual shadow page marking (ADR 0027): every visible prepass pixel marks
// the page it will sample (same level rule as `vsm_shadow` in pbr.wgsl).
struct Mark {
    inv_view_proj: mat4x4<f32>,
    light_view: mat4x4<f32>,
    // xyz camera, w footprint per meter
    camera: vec4<f32>,
    // x level-0 texel, y levels, zw screen size
    params: vec4<f32>,
    // window centre per level: (x, y) for levels 2k and 2k+1
    centers: array<vec4<i32>, 8>,
};
@group(0) @binding(0) var<uniform> mark: Mark;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var<storage, read_write> requests: array<atomic<u32>>;

const PAGE: f32 = 128.0;
const WINDOW: i32 = 64;

fn center(l: u32) -> vec2<i32> {
    let v = mark.centers[l / 2u];
    return select(v.xy, v.zw, (l & 1u) == 1u);
}

@compute @workgroup_size(8, 8)
fn cs_mark(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(mark.params.zw);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let d = textureLoad(depth_tex, vec2<i32>(id.xy), 0);
    if d >= 1.0 {
        return;
    }
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(size);
    let w = mark.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let world = w.xyz / w.w;
    let texel0 = mark.params.x;
    let levels = u32(mark.params.y);
    let footprint = length(world - mark.camera.xyz) * mark.camera.w;
    let first = u32(clamp(ceil(log2(max(footprint / texel0, 1e-6))), 0.0, f32(levels - 1u)));
    let lp = (mark.light_view * vec4<f32>(world, 1.0)).xyz;
    for (var l = first; l < levels; l = l + 1u) {
        let pw = texel0 * f32(1u << l) * PAGE;
        let a = vec2<i32>(floor(lp.xy / pw));
        let rel = a - center(l);
        if any(rel < vec2<i32>(-WINDOW / 2)) || any(rel >= vec2<i32>(WINDOW / 2)) {
            continue;
        }
        let slot = ((a % WINDOW) + WINDOW) % WINDOW;
        atomicOr(&requests[l * u32(WINDOW * WINDOW) + u32(slot.y * WINDOW + slot.x)], 1u);
        return;
    }
}
