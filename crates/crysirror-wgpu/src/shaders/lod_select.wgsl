// SPDX-License-Identifier: MPL-2.0
// GPU cluster-LOD selection and culling (ADR 0026 step 2). One workgroup per
// cluster: thread 0 decides (DAG cut, and frustum for the main view) and
// reserves space with an atomic add on the indirect index count; the group
// copies the cluster's indices into the compacted output.
struct Params {
    // xyz object-space eye, w projected-error threshold
    eye: vec4<f32>,
    // world frustum planes transformed to object space (distance in world units)
    planes: array<vec4<f32>, 6>,
    // x max axis scale, y frustum culling on (1) / off (0)
    misc: vec4<f32>,
    // x main args slot, y shadow args slot, z cluster count
    slots: vec4<u32>,
    // x main region first index, y shadow region first index
    firsts: vec4<u32>,
};
struct Cluster {
    start: u32,
    count: u32,
    lod_error: f32,
    parent_error: f32,
    lod_bounds: vec4<f32>,
    parent_bounds: vec4<f32>,
    bounds: vec4<f32>,
};
struct Args {
    count: atomic<u32>,
    instances: u32,
    first: u32,
    base: i32,
    first_instance: u32,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> out_indices: array<u32>;
@group(0) @binding(2) var<storage, read_write> args: array<Args>;
@group(1) @binding(0) var<storage, read> clusters: array<Cluster>;
@group(1) @binding(1) var<storage, read> src: array<u32>;

const NONE: u32 = 0xFFFFFFFFu;
var<workgroup> base_main: u32;
var<workgroup> base_shadow: u32;

fn projected(error: f32, s: vec4<f32>) -> f32 {
    if error > 3.0e38 {
        return 3.0e38;
    }
    return error / max(distance(params.eye.xyz, s.xyz) - s.w, 1e-6);
}

@compute @workgroup_size(64)
fn cs_select(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let ci = wg.x + wg.y * groups.x;
    if ci >= params.slots.z {
        return;
    }
    if li == 0u {
        let c = clusters[ci];
        let t = params.eye.w;
        let selected = projected(c.lod_error, c.lod_bounds) <= t && projected(c.parent_error, c.parent_bounds) > t;
        var visible = true;
        if params.misc.y > 0.5 {
            for (var k = 0; k < 6; k = k + 1) {
                let p = params.planes[k];
                if dot(p.xyz, c.bounds.xyz) + p.w < -c.bounds.w * params.misc.x {
                    visible = false;
                }
            }
        }
        base_main = NONE;
        base_shadow = NONE;
        if selected && visible {
            base_main = atomicAdd(&args[params.slots.x].count, c.count);
        }
        if selected {
            base_shadow = atomicAdd(&args[params.slots.y].count, c.count);
        }
    }
    let bm = workgroupUniformLoad(&base_main);
    let bs = workgroupUniformLoad(&base_shadow);
    if bm == NONE && bs == NONE {
        return;
    }
    let c = clusters[ci];
    for (var i = li; i < c.count; i = i + 64u) {
        let v = src[c.start + i];
        if bm != NONE {
            out_indices[params.firsts.x + bm + i] = v;
        }
        if bs != NONE {
            out_indices[params.firsts.y + bs + i] = v;
        }
    }
}
