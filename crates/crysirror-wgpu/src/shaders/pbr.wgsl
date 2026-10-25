// SPDX-License-Identifier: MPL-2.0
// Metallic-roughness PBR (glTF 2.0): Cook-Torrance GGX/Smith/Schlick,
// directional sun with PCF shadows, clustered point/spot/rect lights,
// split-sum IBL and SH9 diffuse.
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    return transform_vertex(v);
}
// Entry points cannot call each other; both vertex paths share this.
fn transform_vertex(v: VertexIn) -> VertexOut {
    let world = object.model * vec4<f32>(v.position, 1.0);
    var o: VertexOut;
    o.clip = frame.view_proj * world;
    o.world = world.xyz;
    o.normal = (object.normal * vec4<f32>(v.normal, 0.0)).xyz;
    o.tangent = vec4<f32>((object.model * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    o.uv = v.uv;
    return o;
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let k = skin_matrix(s, object.skin.x);
    var w = v;
    w.position = (k * vec4<f32>(v.position, 1.0)).xyz;
    w.normal = (k * vec4<f32>(v.normal, 0.0)).xyz;
    w.tangent = vec4<f32>((k * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    return transform_vertex(w);
}

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}
fn v_smith_ggx_correlated(nv: f32, nl: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}
fn f_schlick(c: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - c, 5.0);
}
fn f_schlick_roughness(c: f32, f0: vec3<f32>, r: f32) -> vec3<f32> {
    return f0 + (max(vec3<f32>(1.0 - r), f0) - f0) * pow(1.0 - c, 5.0);
}

// Clustered local lights (ADR 0018); constants mirror lights.rs.
struct Light {
    position_range: vec4<f32>,
    direction_kind: vec4<f32>,
    intensity: vec4<f32>,
    cone: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
};
@group(0) @binding(9) var<storage, read> lights: array<Light>;
@group(0) @binding(10) var<storage, read> light_grid: array<vec2<u32>>;
@group(0) @binding(11) var<storage, read> light_indices: array<u32>;
// Local light tile matrices, then the virtual shadow page table (ADR 0027):
// per entry x = packed absolute page coords, y = physical page + 1 (0 = none).
// Then the irradiance volume probes (ADR 0028): 4 vec4 each (r, g, b SH L1
// already convolved to irradiance; w.x of the 4th = valid).
struct ShadowData {
    tiles: array<mat4x4<f32>, 64>,
    pages: array<vec2<u32>, 65536>,
    gi: array<vec4<f32>>,
};
@group(0) @binding(12) var<storage, read> shadow_data: ShadowData;
@group(0) @binding(13) var shadow_atlas: texture_depth_2d;
// Mirrors lights::SHADOW_TILES_PER_ROW / SHADOW_ATLAS / SHADOW_TILE; the
// atlas is 8192×4096: local tiles left, virtual shadow pages right (vsm.rs).
const ATLAS_TILES: u32 = 8u;
const ATLAS_DIMS: vec2<f32> = vec2<f32>(8192.0, 4096.0);
const ATLAS_TILE: f32 = 512.0;
const VSM_PAGE: f32 = 128.0;
const VSM_PHYS_X: f32 = 4096.0;
const VSM_PHYS_ROW: u32 = 32u;
const VSM_WINDOW: i32 = 64;

// PCF visibility from the local shadow atlas; point lights pick the cube face
// by the major axis (faces +X, -X, +Y, -Y, +Z, -Z as in lights::CUBE_FACES).
fn local_shadow(slot: f32, kind: f32, world: vec3<f32>, light_pos: vec3<f32>, ng: vec3<f32>) -> f32 {
    if slot < 0.0 {
        return 1.0;
    }
    var tile = u32(slot);
    let d = world - light_pos;
    if kind < 0.5 {
        let a = abs(d);
        var face = 0u;
        if a.x >= a.y && a.x >= a.z {
            face = select(1u, 0u, d.x > 0.0);
        } else if a.y >= a.z {
            face = select(3u, 2u, d.y > 0.0);
        } else {
            face = select(5u, 4u, d.z > 0.0);
        }
        tile = tile + face;
    }
    // Normal offset of ~1.5 shadow texels at this distance (90° frustum).
    let offset = length(d) * 2.0 / ATLAS_TILE * 1.5;
    let p = shadow_data.tiles[tile] * vec4<f32>(world + ng * offset, 1.0);
    if p.w <= 0.0 {
        return 1.0;
    }
    let ndc = p.xyz / p.w;
    let origin = vec2<f32>(f32(tile % ATLAS_TILES), f32(tile / ATLAS_TILES)) * ATLAS_TILE / ATLAS_DIMS;
    let texel = 1.0 / ATLAS_DIMS;
    let lo = origin + 1.5 * texel;
    let hi = origin + ATLAS_TILE / ATLAS_DIMS - 1.5 * texel;
    let uv = origin + (ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * ATLAS_TILE / ATLAS_DIMS;
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let q = clamp(uv + vec2<f32>(f32(x), f32(y)) * texel, lo, hi);
            sum = sum + textureSampleCompareLevel(shadow_atlas, shadow_sampler, q, ndc.z - 1e-5);
        }
    }
    return sum / 9.0;
}

const CLUSTER_X: u32 = 16u;
const CLUSTER_Y: u32 = 9u;
const CLUSTER_Z: u32 = 24u;

fn cluster_of(frag: vec2<f32>, view_depth: f32) -> u32 {
    let t = frag * frame.screen.zw;
    let x = min(u32(t.x * f32(CLUSTER_X)), CLUSTER_X - 1u);
    let y = min(u32(t.y * f32(CLUSTER_Y)), CLUSTER_Y - 1u);
    let s = log(max(view_depth, frame.clusters.x) / frame.clusters.x) * frame.clusters.w;
    let z = min(u32(max(s * f32(CLUSTER_Z), 0.0)), CLUSTER_Z - 1u);
    return (z * CLUSTER_Y + y) * CLUSTER_X + x;
}

// Dielectric specular weight (KHR_materials_specular); 1 for metals.
var<private> spec_weight: f32 = 1.0;

// Cook-Torrance response to unit illuminance from direction l (times n·l).
fn brdf(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, base: vec3<f32>, metallic: f32, f0: vec3<f32>, a: f32, nv: f32) -> vec3<f32> {
    let nl = max(dot(n, l), 0.0);
    if nl <= 0.0 {
        return vec3<f32>(0.0);
    }
    let h = normalize(v + l);
    let f = f_schlick(max(dot(v, h), 0.0), f0) * spec_weight;
    let spec = d_ggx(max(dot(n, h), 0.0), a) * v_smith_ggx_correlated(nv, nl, a) * f;
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    return (kd * base / PI + spec) * nl;
}

// Smooth range window (UE/Frostbite): (1 - (d/r)^4)^2.
fn range_window(d2: f32, r: f32) -> f32 {
    let q = d2 / (r * r);
    let w = clamp(1.0 - q * q, 0.0, 1.0);
    return w * w;
}

fn local_lights(world: vec3<f32>, clip: vec2<f32>, ng: vec3<f32>, n: vec3<f32>, v: vec3<f32>, base: vec3<f32>, metallic: f32, f0: vec3<f32>, a: f32, nv: f32) -> vec3<f32> {
    if frame.clusters.z < 0.5 {
        return vec3<f32>(0.0);
    }
    let depth = -(frame.view * vec4<f32>(world, 1.0)).z;
    let cell = light_grid[cluster_of(clip, depth)];
    var sum = vec3<f32>(0.0);
    for (var k = 0u; k < cell.y; k = k + 1u) {
        let li = lights[light_indices[cell.x + k]];
        let kind = li.direction_kind.w;
        let r = li.position_range.w;
        var p = li.position_range.xyz;
        let center = world - p;
        let dc2 = dot(center, center);
        if dc2 >= r * r {
            continue;
        }
        let exponent = li.cone.z;
        if kind > 2.5 {
            // Volume lights: illuminance shaped only by the range sphere.
            var shape = range_window(dc2, r);
            if exponent > 0.0 {
                shape = pow(clamp(1.0 - dc2 / (r * r), 0.0, 1.0), exponent);
            }
            let e = li.intensity.rgb * shape;
            if kind > 3.5 {
                sum = sum + base * (1.0 - metallic) * e / PI;
            } else {
                sum = sum + brdf(n, v, -li.direction_kind.xyz, base, metallic, f0, a, nv) * e;
            }
            continue;
        }
        if kind > 1.5 {
            // Rect: closest point on the emitter, Lambertian (one-sided) emission.
            let hx = max(length(li.right.xyz), 1e-5);
            let hy = max(length(li.up.xyz), 1e-5);
            let ax = li.right.xyz / hx;
            let ay = li.up.xyz / hy;
            p = p + ax * clamp(dot(center, ax), -hx, hx) + ay * clamp(dot(center, ay), -hy, hy);
        }
        let to = p - world;
        let d2 = max(dot(to, to), 1e-4);
        let l = to * inverseSqrt(d2);
        var emit = 1.0;
        if kind > 1.5 {
            emit = max(dot(li.direction_kind.xyz, -l), 0.0);
        } else if kind > 0.5 {
            let t = clamp((dot(li.direction_kind.xyz, -l) - li.cone.x) * li.cone.y, 0.0, 1.0);
            emit = t * t;
        }
        if emit <= 0.0 {
            continue;
        }
        var atten = range_window(d2, r) / d2;
        if exponent > 0.0 {
            atten = pow(clamp(1.0 - d2 / (r * r), 0.0, 1.0), exponent);
        }
        var vis = 1.0;
        if kind < 1.5 {
            vis = local_shadow(li.intensity.w, kind, world, li.position_range.xyz, ng);
        }
        sum = sum + brdf(n, v, l, base, metallic, f0, a, nv) * li.intensity.rgb * (emit * atten * vis);
    }
    return sum;
}

fn cascade_shadow(world: vec3<f32>, nl: f32, cascade: u32) -> f32 {
    let p = frame.cascades[cascade] * vec4<f32>(world, 1.0);
    let ndc = p.xyz / p.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let texel = frame.env.w;
    let bias = max(0.0015 * (1.0 - nl), 0.0002) * f32(cascade + 1u);
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            sum = sum + textureSampleCompareLevel(
                shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * texel, i32(cascade), ndc.z - bias);
        }
    }
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)) && ndc.z <= 1.0;
    return select(1.0, sum / 9.0, inside);
}

fn vsm_pack(a: vec2<i32>) -> u32 {
    return (u32(a.x + 32768) & 0xFFFFu) | ((u32(a.y + 32768) & 0xFFFFu) << 16u);
}
// Virtual shadow map lookup (ADR 0027): finest level whose texel covers the
// pixel footprint, falling back to coarser levels for missing pages.
fn vsm_shadow(world: vec3<f32>, nl: f32) -> f32 {
    let texel0 = frame.vsm.x;
    let levels = u32(frame.vsm.y);
    let footprint = length(world - frame.camera_pos.xyz) * frame.vsm2.y;
    let first = u32(clamp(ceil(log2(max(footprint / texel0, 1e-6))), 0.0, f32(levels - 1u)));
    let lp = (frame.vsm_view * vec4<f32>(world, 1.0)).xyz;
    let depth = (-lp.z - frame.vsm.z) / (frame.vsm.w - frame.vsm.z);
    for (var l = first; l < min(first + 3u, levels); l = l + 1u) {
        let pw = texel0 * f32(1u << l) * VSM_PAGE;
        let pc = lp.xy / pw;
        let abs_page = vec2<i32>(floor(pc));
        let slot = ((abs_page % VSM_WINDOW) + VSM_WINDOW) % VSM_WINDOW;
        let e = shadow_data.pages[l * u32(VSM_WINDOW * VSM_WINDOW) + u32(slot.y * VSM_WINDOW + slot.x)];
        if e.y == 0u || e.x != vsm_pack(abs_page) {
            continue;
        }
        let phys = e.y - 1u;
        let base = vec2<f32>(VSM_PHYS_X + f32(phys % VSM_PHYS_ROW) * VSM_PAGE, f32(phys / VSM_PHYS_ROW) * VSM_PAGE);
        let local = fract(pc);
        let t = base + vec2<f32>(local.x, 1.0 - local.y) * VSM_PAGE;
        let texel_world = pw / VSM_PAGE;
        let tan_t = sqrt(max(1.0 - nl * nl, 0.0)) / max(nl, 0.1);
        let bias = texel_world * (1.5 + 2.0 * tan_t) / (frame.vsm.w - frame.vsm.z);
        var sum = 0.0;
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let q = clamp(t + vec2<f32>(f32(x), f32(y)), base + vec2<f32>(1.5), base + vec2<f32>(VSM_PAGE - 1.5));
                sum = sum + textureSampleCompareLevel(shadow_atlas, shadow_sampler, q / ATLAS_DIMS, depth - bias);
            }
        }
        return sum / 9.0;
    }
    return 1.0;
}

// Diffuse irradiance from the GI volume (ADR 0028): trilinear over valid
// probes with a backface weight. Surfaces on the volume boundary (walls,
// floors) clamp into it; the result fades to `fallback` over one cell outside.
// Octahedral direction encoding into [-1, 1]² (visibility moments).
fn sign_nz(v: vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(-1.0), vec2<f32>(1.0), v >= vec2<f32>(0.0));
}
fn oct_encode(d: vec3<f32>) -> vec2<f32> {
    let p = d.xy / (abs(d.x) + abs(d.y) + abs(d.z));
    return select(p, (1.0 - abs(p.yx)) * sign_nz(p), d.z < 0.0);
}
// Probe distance moments (mean, mean²) of octahedral texel (x, y); borders
// wrap the way the octahedron folds, so bilinear taps cross edges correctly.
const GI_VIS: u32 = 16384u;
const GI_OCT: i32 = 8;
fn gi_moments(probe: u32, xy: vec2<i32>) -> vec2<f32> {
    var x = xy.x;
    var y = xy.y;
    if x < 0 || x >= GI_OCT {
        x = clamp(x, 0, GI_OCT - 1);
        y = GI_OCT - 1 - y;
    }
    if y < 0 || y >= GI_OCT {
        y = clamp(y, 0, GI_OCT - 1);
        x = GI_OCT - 1 - x;
    }
    let k = u32(x + y * GI_OCT);
    let v = shadow_data.gi[GI_VIS + probe * 32u + k / 2u];
    return select(v.xy, v.zw, (k & 1u) == 1u);
}
// Chebyshev visibility of a point at distance r from the probe along dir.
fn gi_visibility(probe: u32, dir: vec3<f32>, r: f32) -> f32 {
    let t = (oct_encode(dir) * 0.5 + 0.5) * f32(GI_OCT) - 0.5;
    let i0 = vec2<i32>(floor(t));
    let f = t - floor(t);
    let m = mix(
        mix(gi_moments(probe, i0), gi_moments(probe, i0 + vec2<i32>(1, 0)), f.x),
        mix(gi_moments(probe, i0 + vec2<i32>(0, 1)), gi_moments(probe, i0 + vec2<i32>(1, 1)), f.x),
        f.y);
    if r <= m.x {
        return 1.0;
    }
    let variance = max(abs(m.y - m.x * m.x), 1e-4);
    let d = r - m.x;
    let c = variance / (variance + d * d);
    return max(c * c * c, 0.0);
}

fn gi_irradiance(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    if frame.gi_extent.w < 0.5 {
        return fallback;
    }
    let ext = frame.gi_extent.xyz;
    let cnt = vec3<u32>(frame.gi_counts.xyz);
    let cells = vec3<f32>(cnt - vec3<u32>(1u));
    let cell = 2.0 * ext / cells;
    let rel0 = p - frame.gi_center.xyz;
    let outside = max(abs(rel0) - ext, vec3<f32>(0.0)) / cell;
    if any(outside >= vec3<f32>(1.0)) {
        return fallback;
    }
    let rel = clamp(rel0, -ext, ext);
    let t = (rel + ext) / (2.0 * ext) * cells;
    let b = vec3<u32>(clamp(floor(t), vec3<f32>(0.0), cells - vec3<f32>(1.0)));
    let f = t - vec3<f32>(b);
    // Visibility is tested from a point pushed off the surface toward the
    // viewer, so the surface itself does not self-occlude.
    let biased = p + (n * 0.2 + v * 0.8) * 0.3 * min(cell.x, min(cell.y, cell.z));
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var c = 0u; c < 8u; c = c + 1u) {
        let o = vec3<u32>(c & 1u, (c >> 1u) & 1u, (c >> 2u) & 1u);
        let idx = b + o;
        let i = idx.x + cnt.x * (idx.y + cnt.y * idx.z);
        if shadow_data.gi[i * 4u + 3u].x < 0.5 {
            continue;
        }
        let tri = select(1.0 - f, f, o == vec3<u32>(1u));
        let probe = frame.gi_center.xyz - ext + vec3<f32>(idx) / cells * 2.0 * ext;
        let toward = normalize(probe - p + n * 1e-3);
        let back = (dot(toward, n) + 1.0) * 0.5;
        var w = back * back + 0.2;
        if frame.gi_counts.w > 0.5 {
            let off = biased - probe;
            let r = length(off);
            w = w * gi_visibility(i, off / max(r, 1e-4), r);
        }
        w = max(w, 1e-6);
        // Crush tiny weights so fully occluded probes stop contributing.
        if w < 0.2 {
            w = w * w * w / 0.04;
        }
        w = w * tri.x * tri.y * tri.z;
        let nn = vec4<f32>(1.0, n);
        let e = vec3<f32>(dot(shadow_data.gi[i * 4u], nn), dot(shadow_data.gi[i * 4u + 1u], nn), dot(shadow_data.gi[i * 4u + 2u], nn));
        sum = sum + max(e, vec3<f32>(0.0)) * w;
        wsum = wsum + w;
    }
    if wsum <= 1e-7 {
        return fallback;
    }
    let fade = 1.0 - max(outside.x, max(outside.y, outside.z));
    return mix(fallback, sum / wsum * frame.gi_center.w, fade);
}

// Cascaded sun shadows selected by view depth, blended across cascade seams.
fn sun_shadow(world: vec3<f32>, nl: f32) -> f32 {
    if frame.vsm2.x > 0.5 {
        return vsm_shadow(world, nl);
    }
    let depth = -(frame.view * vec4<f32>(world, 1.0)).z;
    if depth > frame.splits.w {
        return 1.0;
    }
    var c = 0u;
    for (var i = 0u; i < CASCADES - 1u; i = i + 1u) {
        if depth > frame.splits[i] {
            c = i + 1u;
        }
    }
    let s = cascade_shadow(world, nl, c);
    let end = frame.splits[c];
    let blend = clamp((end - depth) / (end * 0.1), 0.0, 1.0);
    if c < CASCADES - 1u && blend < 1.0 {
        return mix(cascade_shadow(world, nl, c + 1u), s, blend);
    }
    return s;
}

// Box-projected reflection captures (ADR 0024): weights fade in over the
// blend distance inside each box; uncovered weight keeps `fallback`.
fn probe_specular(p: vec3<f32>, r: vec3<f32>, roughness: f32, fallback: vec3<f32>) -> vec3<f32> {
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var k = 0u; k < 8u; k = k + 1u) {
        let a = frame.probes[3u * k];
        let b = frame.probes[3u * k + 1u];
        let parallax = frame.probes[3u * k + 2u].x > 0.5;
        let rel = p - a.xyz;
        let inside = b.xyz - abs(rel);
        if any(inside <= vec3<f32>(0.0)) {
            continue;
        }
        let w = clamp(min(inside.x, min(inside.y, inside.z)) / max(b.w, 1e-4), 0.0, 1.0);
        let rr = select(r, vec3<f32>(1e-6), abs(r) < vec3<f32>(1e-6));
        let far = max((b.xyz - rel) / rr, (-b.xyz - rel) / rr);
        let dir = select(r, normalize(rel + r * min(far.x, min(far.y, far.z))), parallax);
        let uv = vec2<f32>(atan2(dir.z, dir.x) / (2.0 * PI) + 0.5, acos(clamp(dir.y, -1.0, 1.0)) / PI);
        acc = acc + textureSampleLevel(probe_maps, env_sampler, uv, k, roughness * frame.env.z).rgb * a.w * w;
        wsum = wsum + w;
    }
    if wsum >= 1.0 {
        return acc / wsum;
    }
    return acc + fallback * (1.0 - wsum);
}

// Irradiance capture output (ADR 0028): radiance and the hit distance,
// negative for back faces, which contribute no light and mark probes that
// sit inside geometry.
fn gi_capture(color: vec3<f32>, world: vec3<f32>, front: bool) -> vec4<f32> {
    let d = max(length(world - frame.camera_pos.xyz), 1e-3);
    return vec4<f32>(select(vec3<f32>(0.0), color, front), select(-d, d, front));
}

@fragment
fn fs_main(i: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Sample everything first: derivatives stay in uniform control flow.
    let uv=material_uv(i.uv);
    let surface=sample_surface(uv,i.world,i.normal,frame.camera_pos.xyz,frame.misc.z);
    let base=surface.color;
    let tn = decode_normal(textureSample(t_normal, s_material, uv));
    let specular_color = textureSample(t_specular, s_material, uv).rgb * material.surface.w;
    let occ_tex = textureSample(t_occlusion, s_material, uv).r;
    if material.alpha.x == 1.0 && base.a < material.alpha.y {
        discard;
    }

    var alpha=select(1.0,clamp(base.a,0.0,1.0),material.alpha.x>=2.0);
    if material.alpha.x>=2.0 && material.surface.x>0.0 {
        let depth=textureLoad(opaque_depth,vec2<i32>(i.clip.xy),0);
        let uv_depth=i.clip.xy*frame.screen.zw;
        let ndc=vec4<f32>(uv_depth*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),depth,1.0);
        let p=frame.inv_view_proj*ndc;
        let opaque_view=-(frame.view*vec4<f32>(p.xyz/p.w,1.0)).z;
        let fragment_view=-(frame.view*vec4<f32>(i.world,1.0)).z;
        alpha=alpha*clamp((opaque_view-fragment_view)/material.surface.x,0.0,1.0);
    }
    let emissive=surface.emission;
    if material.alpha.z>0.5 {
        let color=base.rgb+emissive;
        if frame.misc.w > 0.5 {
            return gi_capture(color, i.world, front);
        }
        return vec4<f32>(select(color,color*alpha,material.alpha.x==4.0),alpha);
    }
    let roughness = surface.roughness;
    let metallic = surface.metallic;
    var ng = normalize(i.normal);
    if !front {
        ng = -ng;
    }
    let t = normalize(i.tangent.xyz - ng * dot(ng, i.tangent.xyz));
    let b = cross(ng, t) * i.tangent.w;
    let n = normalize(mat3x3<f32>(t, b, ng) * vec3<f32>(tn.xy * material.params.z, tn.z));
    let v = normalize(frame.camera_pos.xyz - i.world);
    let nv = max(dot(n, v), 1e-4);
    let f0 = mix(min(vec3<f32>(0.04) * specular_color, vec3<f32>(1.0)), base.rgb, metallic);
    let a = roughness * roughness;
    spec_weight = mix(material.surface.w, 1.0, metallic);

    // Direct sun.
    let l = normalize(-frame.sun_dir.xyz);
    let h = normalize(v + l);
    let nl = max(dot(n, l), 0.0);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let f = f_schlick(vh, f0) * spec_weight;
    let spec = d_ggx(nh, a) * v_smith_ggx_correlated(nv, nl, a) * f;
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    var visibility = 1.0;
    if frame.sun_color.w > 0.5 && nl > 0.0 {
        visibility = sun_shadow(i.world, max(dot(ng, l), 0.0));
    }
    let direct = (kd * base.rgb / PI + spec) * frame.sun_color.rgb * frame.sun_dir.w * nl * visibility;

    // Image-based lighting (split sum + SH9).
    let fr = f_schlick_roughness(nv, f0, roughness) * spec_weight;
    let kd_ibl = (vec3<f32>(1.0) - fr) * (1.0 - metallic);
    let irradiance = sh_irradiance(n);
    let r = reflect(-v, n);
    var prefiltered = textureSampleLevel(env_prefiltered, env_sampler, equirect_uv(r), roughness * frame.env.z).rgb;
    prefiltered = probe_specular(i.world, r, roughness, prefiltered);
    // Screen-space reflections replace the environment where rays hit (ADR 0023).
    if frame.ssr.x > 0.0 && material.alpha.x < 2.0 {
        let s = textureLoad(ssr_texture, vec2<i32>(i.clip.xy), 0);
        let gloss = 1.0 - smoothstep(frame.ssr.y * 0.5, frame.ssr.y, roughness);
        prefiltered = mix(prefiltered, s.rgb, clamp(s.a * gloss * frame.ssr.x, 0.0, 1.0));
    }
    let lut = textureSampleLevel(brdf_lut, env_sampler, vec2<f32>(nv, roughness), 0.0).rg;
    let specular_ibl = prefiltered * (fr * lut.x + lut.y * spec_weight);
    var occlusion = mix(1.0, occ_tex, material.params.w);
    if frame.misc.x > 0.0 && material.alpha.x<2.0 {
        let ssao = textureLoad(ao_texture, vec2<i32>(i.clip.xy), 0).r;
        occlusion = occlusion * mix(1.0, ssao, min(frame.misc.x, 1.0));
    }
    // The GI volume holds absolute irradiance (sky included), so the
    // environment intensity only scales the fallback and the specular term.
    let diffuse_irr = gi_irradiance(i.world, n, v, irradiance * frame.env.x);
    let ambient = (kd_ibl * base.rgb * diffuse_irr / PI + specular_ibl * frame.env.x) * occlusion;
    let local = local_lights(i.world, i.clip.xy, ng, n, v, base.rgb, metallic, f0, max(a, 0.002), nv);
    let fog = height_fog(i.world);
    let color=mix(direct+local+ambient+emissive, fog.rgb, fog.a);
    if frame.misc.w > 0.5 {
        return gi_capture(color, i.world, front);
    }
    return vec4<f32>(select(color,color*alpha,material.alpha.x==4.0),alpha);
}
