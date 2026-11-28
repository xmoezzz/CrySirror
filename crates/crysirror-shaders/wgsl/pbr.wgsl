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
    /// Per-vertex auxiliary vector (`_AUX`); ones when the primitive has none.
    @location(4) aux: vec4<f32>,
    @location(5) blinn_view: vec3<f32>,
};

// Optional second vertex stream of primitives with an `_AUX` attribute.
struct AuxIn {
    @location(6) aux: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    return transform_vertex(v);
}

@vertex
fn vs_aux(v: VertexIn, a: AuxIn) -> VertexOut {
    var o = transform_vertex(v);
    o.aux = a.aux;
    if material.surface.y == 11.0 {
        o.aux.w = pow(clamp(a.aux.w, 0.0, 1.0), material.effect_params[1].x);
    }
    return o;
}
// Entry points cannot call each other; both vertex paths share this.
fn transform_vertex(v: VertexIn) -> VertexOut {
    let world = shell_world(v.position, v.normal, v.uv, object.model, frame.view_proj);
    var o: VertexOut;
    o.clip = frame.view_proj * world;
    o.world = world.xyz;
    o.normal = (object.normal * vec4<f32>(v.normal, 0.0)).xyz;
    o.tangent = vec4<f32>((object.model * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    o.uv = v.uv + object.uv.xy;
    o.aux = vec4<f32>(1.0);
    if material.surface.y == 11.0 {
        o.normal = normalize(o.normal);
        let view = frame.camera_pos.xyz - world.xyz;
        o.blinn_view = view / max(length(view), 1e-8);
    }
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

// Toon outline hull (CM3D2/Outline `OUTLINE` pass): the clip position moves
// along the view-space normal projected by the projection's 2x2 part, scaled
// by min(depth, 0.7) and the material's outline width.
fn outline_vertex(v: VertexIn) -> VertexOut {
    var o = transform_vertex(v);
    // P · (n_view, 0) = VP · (n_world, 0).
    let offset = (frame.view_proj * vec4<f32>(normalize(o.normal), 0.0)).xy;
    let depth = -(frame.view * vec4<f32>(o.world, 1.0)).z;
    let k = clamp(depth, 0.0, 0.7) * material.effect_params[3].y;
    o.clip = vec4<f32>(o.clip.xy + offset * k, o.clip.zw);
    return o;
}
@vertex
fn vs_outline(v: VertexIn) -> VertexOut {
    return outline_vertex(v);
}
@vertex
fn vs_outline_skinned(v: VertexIn, s: SkinIn) -> VertexOut {
    let k = skin_matrix(s, object.skin.x);
    var w = v;
    w.position = (k * vec4<f32>(v.position, 1.0)).xyz;
    w.normal = (k * vec4<f32>(v.normal, 0.0)).xyz;
    w.tangent = vec4<f32>((k * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    return outline_vertex(w);
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
            if frame.rt_shadow.y > 0.5 && li.cone.w > 0.5 {
                // Ray-traced visibility of this light at this pixel (ADR 0057).
                let traced = textureLoad(ao_texture, vec2<i32>(clip), 0);
                vis = select(traced.a, traced.b, li.cone.w < 1.5);
            } else {
                vis = local_shadow(li.intensity.w, kind, world, li.position_range.xyz, ng);
            }
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
fn sun_shadow(world: vec3<f32>, nl: f32, pixel:vec2<f32>) -> f32 {
    if frame.rt_shadow.x>0.5 {return textureLoad(ao_texture,vec2<i32>(pixel),0).g;}
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

// Exact sRGB transfer, for toon materials that shade display-referred values.
fn toon_encode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}
fn toon_decode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}
// Rows of the 256x3 toon lookup texture (material slot 1).
fn toon_lookup(row: f32, u: f32) -> vec4<f32> {
    return textureSampleLevel(t_mr, s_material, vec2<f32>(clamp(u, 0.0, 1.0), (row + 0.5) / 3.0), 0.0);
}
// The toon key light (display-referred colour × intensity), bounded per
// channel by the material's light limit (params.y) when positive.
fn toon_light() -> vec3<f32> {
    let light = toon_encode(frame.toon_color.rgb) * frame.toon_dir.w;
    if material.params.y > 0.0 {
        return min(light, vec3<f32>(material.params.y));
    }
    return light;
}
// Shadow tone of a linear base colour (ToonShading shade_from_base): hue
// kept, saturation sqrt(S)·min(10·sqrt(V), 1), value 0.5·sqrt(V) + 0.5.
fn toon_base_shade(c: vec3<f32>) -> vec3<f32> {
    let v = max(max(c.r, c.g), c.b);
    let d = v - min(min(c.r, c.g), c.b);
    var h = 0.0;
    if d > 1e-6 {
        if v == c.r {
            h = (c.g - c.b) / d;
        } else if v == c.g {
            h = 2.0 + (c.b - c.r) / d;
        } else {
            h = 4.0 + (c.r - c.g) / d;
        }
        h = fract(h / 6.0 + 1.0);
    }
    let s = d / (v + 1e-10);
    let s2 = min(sqrt(s), 1.0) * min(sqrt(v) * 10.0, 1.0);
    let v2 = sqrt(v) * 0.5 + 0.5;
    let hue = clamp(abs(fract(vec3<f32>(h) + vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return ((hue - 1.0) * s2 + 1.0) * v2;
}
// HSV saturation × 1.5 of a linear colour (the AL ambient tone).
fn toon_ambient_tone(c: vec3<f32>) -> vec3<f32> {
    let v = max(max(c.r, c.g), c.b);
    let d = v - min(min(c.r, c.g), c.b);
    if d <= 1e-6 {
        return c;
    }
    var h = 0.0;
    if v == c.r {
        h = (c.g - c.b) / d;
    } else if v == c.g {
        h = 2.0 + (c.b - c.r) / d;
    } else {
        h = 4.0 + (c.r - c.g) / d;
    }
    h = fract(h / 6.0 + 1.0);
    let s = d / (v + 1e-10) * 1.5;
    let hue = clamp(abs(fract(vec3<f32>(h) + vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return ((hue - 1.0) * s + 1.0) * v;
}
// ShaderEffect::BlockNoise: Ashima 2D simplex noise of screen blocks.
fn sn_mod289(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}
fn sn_permute(x: vec3<f32>) -> vec3<f32> {
    return sn_mod289(((x * 34.0) + 1.0) * x);
}
fn simplex2(v: vec2<f32>) -> f32 {
    let c = vec4<f32>(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
    var i = floor(v + dot(v, c.yy));
    let x0 = v - i + dot(i, c.xx);
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    var x12 = x0.xyxy + c.xxzz;
    x12 = vec4<f32>(x12.xy - i1, x12.zw);
    i = i - floor(i * (1.0 / 289.0)) * 289.0;
    let p = sn_permute(sn_permute(i.y + vec3<f32>(0.0, i1.y, 1.0)) + i.x + vec3<f32>(0.0, i1.x, 1.0));
    var m = max(vec3<f32>(0.5) - vec3<f32>(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), vec3<f32>(0.0));
    m = m * m;
    m = m * m;
    let x = 2.0 * fract(p * c.www) - 1.0;
    let h = abs(x) - 0.5;
    let ox = floor(x + 0.5);
    let a0 = x - ox;
    m = m * (1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h));
    let g = vec3<f32>(a0.x * x0.x + h.x * x0.y, a0.y * x12.x + h.y * x12.y, a0.z * x12.z + h.z * x12.w);
    return 130.0 * dot(m, g);
}
fn block_noise_color(frag: vec2<f32>) -> vec3<f32> {
    let p = material.effect_params;
    let pixel = vec2<f32>(frag.x, frame.screen.y - frag.y);
    var v = (pixel * 0.01 + vec2<f32>(10.0, 0.0)) * p[0].x;
    v = floor(v + floor(frame.misc.z * p[0].y) * 3.0);
    let t = fract(simplex2(v) * 0.5 + 0.5);
    let c = mix(p[1].rgb, p[2].rgb, t);
    return mix(c, vec3<f32>(0.789393), p[0].z);
}
fn toon_outline_color() -> vec4<f32> {
    return material.emissive;
}
fn toon_table(c: vec3<f32>) -> vec3<f32> {
    if material.effect_params[2].w > 0.5 {
        return toon_lookup(2.0, c.r * 0.9951).rgb;
    }
    return c;
}

// Outline colour: ramp(n·l) · outline colour · light · 2 + ambient · 2
// (display-referred, like the toon surface).
@fragment
fn fs_outline(i: VertexOut) -> @location(0) vec4<f32> {
    if object.uv.w > 0.0 && !bayer_visible(1.0 - object.uv.w, vec2<u32>(vec2<f32>(i.clip.x, frame.screen.y - i.clip.y))) { discard; }
    let n = normalize(i.normal);
    let l = normalize(-frame.toon_dir.xyz);
    let ramp = toon_lookup(0.0, dot(n, l) * 0.495 + 0.5).rgb;
    let light = toon_light();
    let ambient = toon_encode(sh_irradiance(n) * frame.env.x / PI);
    let p = material.effect_params;
    if material.raster.y > 0.5 {
        let edge = textureSample(t_normal, s_material, material_uv(i.uv)).rgb;
        let edge_ramp = toon_lookup(2.0, dot(n, l) * 0.495 + 0.5).rgb;
        var attenuation = 1.0;
        if frame.sun_color.w > 0.0 {
            attenuation = mix(1.0, sun_shadow(i.world, max(dot(n, normalize(-frame.sun_dir.xyz)), 0.0), i.clip.xy), frame.sun_color.w);
        }
        return vec4<f32>(toon_decode(edge * edge_ramp * light * (2.0 * attenuation)), attenuation);
    }
    var c = ramp * toon_outline_color().rgb * light * 2.0 + ambient * 2.0;
    if p[3].z > 0.5 {
        // Ramp-shaded (Unity) toon: the outline is shaded like the surface.
        c = toon_outline_color().rgb * (ramp * light + ambient * p[3].w);
    }
    return vec4<f32>(toon_decode(c), 1.0);
}

@fragment
fn fs_main(i: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if raster_culled(front) { discard; }
    // Sample everything first: derivatives stay in uniform control flow.
    if object.uv.w > 0.0 && !bayer_visible(1.0 - object.uv.w, vec2<u32>(vec2<f32>(i.clip.x, frame.screen.y - i.clip.y))) { discard; }
    let uv=material_uv(i.uv);
    var surface=sample_surface(uv,i.world,i.normal,frame.camera_pos.xyz,frame.misc.z);
    // Vertex layer blend (effect 7): the emissive slot holds the second
    // layer, mixed in by 1 − aux.x; the slot then contributes no emission.
    let layer_b = textureSample(t_emissive, s_material, uv);
    if material.surface.y == 7.0 {
        let w = clamp(i.aux.x, 0.0, 1.0);
        surface.color = vec4<f32>(mix(layer_b.rgb * material.base_color.rgb, surface.color.rgb, w), surface.color.a);
        surface.emission = vec3<f32>(0.0);
    }
    // Slope layers (effect 9, or 8 with a tint): the emissive-slot texture
    // × tint on upward-facing surfaces, the base on walls. Height-mask
    // layers (effect 8, mode 2): the base where the occlusion-slot mask's
    // green channel rises above the vertex mask (`_AUX` w).
    let dual_blinn = material.surface.y == 11.0;
    let mask_layer = (material.surface.y == 8.0 && material.effect_params[1].w > 1.5) || dual_blinn;
    let layer_mask = textureSample(t_occlusion, s_material, uv).g;
    var layer_weight = 1.0;
    if material.surface.y == 9.0 || (material.surface.y == 8.0 && material.effect_params[1].w > 0.5) || dual_blinn {
        var w = clamp(1.0 - normalize(i.normal).y, 0.0, 1.0);
        if mask_layer {
            let soft = max(select(material.effect_params[2].x, material.effect_params[0].y, dual_blinn), 1e-4);
            w = clamp((layer_mask - (1.0 + soft) * i.aux.w + 2.0 * soft) / (2.0 * soft), 0.0, 1.0);
        }
        layer_weight = w;
        let tint = select(material.effect_params[1].rgb, vec3<f32>(1.0), dual_blinn);
        let top = layer_b.rgb * tint * material.base_color.rgb;
        surface.color = vec4<f32>(mix(top, surface.color.rgb, w), surface.color.a);
        surface.emission = vec3<f32>(0.0);
    }
    let base=surface.color;
    let lm_uv = i.aux.xy * material.effect_params[0].xy + material.effect_params[0].zw;
    let lm_dx = dpdx(lm_uv);
    let lm_dy = dpdy(lm_uv);
    if material.surface.y == 10.0 {
        if material.alpha.x == 1.0 && base.a < material.alpha.y { discard; }
        let baked = textureSampleGrad(t_emissive, s_material, lm_uv, lm_dx, lm_dy).rgb;
        var attenuation = 1.0;
        if frame.sun_color.w > 0.0 {
            let nl = max(dot(normalize(i.normal), normalize(-frame.sun_dir.xyz)), 0.0);
            attenuation = mix(1.0, sun_shadow(i.world, nl, i.clip.xy), frame.sun_color.w);
        }
        let key = frame.sun_color.rgb * frame.sun_dir.w;
        let color = mix(material.effect_params[1].rgb, base.rgb * baked, attenuation) * key;
        let fog = height_fog(i.world);
        return vec4<f32>(mix(color, fog.rgb, fog.a), 1.0);
    }
    let tn = decode_normal(textureSample(t_normal, s_material, uv));
    let specular_color = textureSample(t_specular, s_material, uv).rgb * material.surface.w;
    let occ_sample = textureSample(t_occlusion, s_material, uv);
    let occ_tex = select(occ_sample.r, 1.0, material.surface.y == 6.0 || mask_layer);
    // Toon: raw base, shadow (slot 3) and highlight (slot 6) samples.
    var toon_base = textureSample(t_base, s_material, uv);
    let toon_blend = textureSample(t_emissive, s_material, uv);
    if material.alpha.z > 1.5 && material.params.w > 0.5 {
        toon_base = mix(toon_base, toon_blend, clamp(object.uv.z, 0.0, 1.0));
    }
    // Liquid layer: per-region coverage min(max(sat(l - 1)·A, l·B), region).
    // (Blobs bound in the opacity slot, regions in the highlight slot.)
    let liquid_blob = textureSample(t_opacity, s_material, uv * material.liquid_tiling.xy + material.liquid_tiling.zw).rg;
    let liquid_region = textureSample(t_specular, s_material, uv);
    if material.alpha.z > 1.5 && material.liquid_color.a > 0.5 {
        let l = object.liquid;
        let cover = min(max(clamp(l - 1.0, vec4<f32>(0.0), vec4<f32>(1.0)) * liquid_blob.r, l * liquid_blob.g), liquid_region);
        let c = clamp(cover.r + cover.g + cover.b + cover.a, 0.0, 1.0);
        toon_base = vec4<f32>(mix(toon_base.rgb, material.liquid_color.rgb, c), toon_base.a);
    }
    let toon_hi = textureSample(t_specular, s_material, uv);
    var alpha=select(1.0,clamp(base.a,0.0,1.0),material.alpha.x>=2.0);
    if material.line_params[0].x == 2.0 {
        let cloth = cloth_coverage(uv, frame.camera_pos.xyz, object.model[3].xyz, vec2<u32>(vec2<f32>(i.clip.x, frame.screen.y - i.clip.y)));
        if cloth.x < cloth.y { discard; }
        alpha = select(1.0,cloth.x,material.alpha.x >= 2.0);
    } else if material.alpha.x == 1.0 && base.a < material.alpha.y {
        discard;
    }


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
    // alpha.z: 0 lit, 1 unlit, 2 toon.
    if material.alpha.z>0.5 && material.alpha.z<1.5 {
        var color=base.rgb+emissive;
        if material.surface.y == 5.0 {
            let pixel = vec2<u32>(vec2<f32>(i.clip.x, frame.screen.y - i.clip.y));
            if !shell_visible(front, pixel, toon_base.a) { discard; }
            let n = normalize(i.normal);
            let nl = dot(n, normalize(-frame.toon_dir.xyz));
            var attenuation = 1.0;
            if frame.sun_color.w > 0.0 {
                attenuation = mix(1.0, sun_shadow(i.world, max(nl, 0.0), i.clip.xy), frame.sun_color.w);
            }
            let t = clamp(0.33 * nl + 0.1, 0.0, 1.0) * attenuation;
            let line = material.base_color.rgb;
            let y = dot(line, vec3<f32>(0.212673, 0.715152, 0.072175));
            let saturated = y + 1.2 * (line - y);
            let k = 1.5 * t * clamp(dot(line, vec3<f32>(0.340136)) + 0.5, 0.0, 1.0);
            let key = clamp(frame.toon_color.rgb * frame.toon_dir.w, vec3<f32>(0.0), vec3<f32>(1.0));
            let lit = clamp(mix(line, saturated, t) + k * key, vec3<f32>(0.0), vec3<f32>(1.0));
            let fog = height_fog(i.world);
            color = mix(lit, fog.rgb, fog.a);
        } else if material.surface.y == 4.0 {
            color = block_noise_color(i.clip.xy);
        }
        if frame.misc.w > 0.5 {
            return gi_capture(color, i.world, front);
        }
        return vec4<f32>(select(color,color*alpha,material.alpha.x>=4.0),alpha);
    }
    if material.alpha.z > 1.5 {
        let p = material.effect_params;
        var tn_toon = normalize(i.normal);
        if !front {
            tn_toon = -tn_toon;
        }
        let tv = normalize(frame.camera_pos.xyz - i.world);
        let tl = normalize(-frame.toon_dir.xyz);
        let tex = toon_table(toon_encode(toon_base.rgb));
        let rim = clamp(pow(max(1.0 - clamp(dot(tv, tn_toon), 0.0, 1.0) + p[1].w, 0.0), p[0].w), 0.0, 1.0);
        let c = tex * material.base_color.rgb + rim * p[0].rgb;
        let u = dot(tn_toon, tl) * 0.4951 + 0.5;
        var ramp = toon_table(toon_lookup(0.0, u).rgb);
        var shadow_end = toon_table(toon_lookup(0.0, 0.0).rgb);
        var cast_light = 1.0;
        if frame.sun_color.w > 0.0 {
            cast_light = mix(1.0, sun_shadow(i.world, max(dot(tn_toon, normalize(-frame.sun_dir.xyz)), 0.0), i.clip.xy), frame.sun_color.w);
        }
        var light = toon_light();
        if material.params.x > 0.5 && material.toon_light.a > 0.0 {
            // AL skin light model (ToonShading::shadow_light).
            let lin = toon_decode(tex);
            var shade = toon_base_shade(lin);
            if material.toon_shade.a > 1.5 {
                shade = material.toon_shade.rgb;
            } else if material.toon_shade.a > 0.5 {
                let v = max(max(lin.r, lin.g), lin.b);
                let sat = (v - min(min(lin.r, lin.g), lin.b)) / (v + 1e-10);
                let k = max(min(sqrt(sat), 1.0) * min(v * 10.0, 1.0), 1.0 - v);
                shade = mix(vec3<f32>(0.450842, 0.60609, 1.0), lin, k);
            } else if material.toon.a > 0.0 {
                let bent = normalize(tn_toon - vec3<f32>(0.0, 0.25, 0.0));
                let sun = toon_encode(frame.toon_color.rgb) * frame.toon_dir.w;
                let k = 0.7 - 0.2 * (clamp(dot(sun, vec3<f32>(0.299, 0.587, 0.114)), 1.0, 3.0) - 1.0);
                let f = min(pow(max(1.0 - dot(bent, tv), 1e-4), 1.8), 1.0) * k * material.toon.a;
                shade = mix(shade, material.toon.rgb, f);
            }
            let nl = dot(tn_toon, tl);
            let key = frame.toon_color.rgb * frame.toon_dir.w;
            let lw = vec3<f32>(0.212673, 0.715152, 0.072175);
            let sh = max(sh_irradiance(tn_toon) * frame.env.x / PI, vec3<f32>(0.0));
            let ambient_tone = toon_ambient_tone(clamp(sh, vec3<f32>(0.0), vec3<f32>(1.0)) * 0.7 + 0.3);
            let lf = 0.7 - 0.2 * (clamp(dot(key, lw), 1.0, 3.0) - 1.0);
            let w = lf * 0.375 * (1.0 + clamp(nl, 0.0, 1.0));
            let mixed = mix(ambient_tone, shade, w);
            let sums = clamp(log2(max(key, vec3<f32>(1e-9))) * 0.30103, vec3<f32>(0.0), vec3<f32>(3.0)) + clamp(key, vec3<f32>(0.0), vec3<f32>(1.0));
            let s = sums.r + sums.g + sums.b;
            let fl = min(1.0 - (min(s, 4.0) - 1.0) / 6.0, 1.0);
            let kk = 1.1 + 0.9 * clamp((s - 1.0) / 3.0, 0.0, 1.0);
            let fm = fl * mixed;
            let lum = dot(fm, lw);
            let graded = lum + kk * (fm - lum);
            let tone = mix(graded, material.shadow_tone_override.rgb, material.shadow_tone_override.a);
            let lit = mix(toon_lookup(0.0, 0.0).r, toon_lookup(0.0, u).r, cast_light);
            let unlit = mix(material.toon_light.rgb, sh, clamp((dot(key, vec3<f32>(1.0 / 3.0)) - 1.0) * 0.5, 0.0, 1.0));
            let kc = min(key, vec3<f32>(1.0));
            let key_side = max(kc, kc * 0.8 + 0.2);
            let lc = mix(unlit, key_side, max(cast_light, 0.5) * clamp((nl + 0.8) * 0.625, 0.0, 1.0));
            ramp = toon_encode(lin * mix(tone, vec3<f32>(1.0), lit) * lc) / max(tex, vec3<f32>(1e-4));
            shadow_end = ramp;
            light = vec3<f32>(1.0);
        } else if material.params.x > 0.5 {
            // The tone multiplies the base in linear space; as a
            // display-referred ramp factor of the base.
            let lin = toon_decode(tex);
            var shade = toon_base_shade(lin);
            if material.toon.a > 0.0 {
                let bent = normalize(tn_toon - vec3<f32>(0.0, 0.25, 0.0));
                let sun = toon_encode(frame.toon_color.rgb) * frame.toon_dir.w;
                let k = 0.7 - 0.2 * (clamp(dot(sun, vec3<f32>(0.299, 0.587, 0.114)), 1.0, 3.0) - 1.0);
                let f = min(pow(max(1.0 - dot(bent, tv), 1e-4), 1.8), 1.0) * k * material.toon.a;
                shade = mix(shade, material.toon.rgb, f);
            }
            let inv = 1.0 / max(tex, vec3<f32>(1e-4));
            ramp = toon_encode(lin * mix(shade, vec3<f32>(1.0), toon_lookup(0.0, u).r)) * inv;
            shadow_end = toon_encode(lin * mix(shade, vec3<f32>(1.0), toon_lookup(0.0, 0.0).r)) * inv;
        }
        let rate = toon_lookup(1.0, u).r;
        let shadow_layer = toon_table(occ_sample.rgb);
        let a = occ_sample.a * (1.0 - rate);
        var m = mix(c, shadow_layer, a);
        let nh = max(dot(tn_toon, normalize(tl + tv)), 0.0);
        if p[3].x > 0.5 {
            m = m + toon_table(toon_hi.rgb) * toon_hi.a * pow(nh, p[2].z) * p[2].y;
        }
        if p[3].z > 0.5 {
            ramp = mix(shadow_end, ramp, cast_light);
        } else {
            m = mix(p[1].rgb, m, cast_light);
        }
        let ambient = toon_encode(sh_irradiance(tn_toon) * frame.env.x / PI);
        let shaded = m * ramp * light + vec3<f32>(pow(nh, 48.0) * p[2].x) + c * ambient * p[3].w;
        let fog_t = height_fog(i.world);
        var line_surface = toon_decode(shaded);
        if material.line_color.a >= 0.0 {
            let mask = textureSample(t_normal, s_material, uv);
            let line = material.line_params[0];
            if line.x == 1.0 {
                let nose = material.line_params[1];
                line_surface = mix(line_surface, nose.rgb, mask.r * nose.a);
                let view = normalize(tv + vec3<f32>(0.0, 0.36, 0.03));
                let rim = clamp(0.75 - 0.75 * dot(view, tn_toon), 0.0, 1.0) * material.toon.a * (1.0 - mask.a);
                let line_base = toon_base.rgb * mix(vec3<f32>(1.0), material.toon.rgb, rim);
                line_surface = mix(line_surface, mix(line_base, material.line_color.rgb, material.line_color.a), max(mask.g, mask.b));
            } else if line.x == 2.0 {
                let tint = mix(line_surface, material.line_color.rgb, material.line_color.a);
                let edge = clamp(4.0 * mask.r * line.y, 0.0, 1.0) * line.z;
                let source_alpha = textureSampleBias(t_base, s_material, uv, line.w).a;
                line_surface = mix(line_surface, tint, (1.0 - source_alpha) * edge);
                line_surface = mix(line_surface, mask.g * tint, mask.g);
            } else {
                let line_tint = clamp(mix(line_surface, material.line_color.rgb, material.line_color.a), vec3<f32>(0.0), vec3<f32>(0.95));
                line_surface = mix(line_surface, line_tint, select(0.0, mask.g, mask.r > 0.0));
            }
        }
        let color = mix(line_surface, fog_t.rgb, fog_t.a);
        if frame.misc.w > 0.5 {
            return gi_capture(color, i.world, front);
        }
        return vec4<f32>(select(color, color * alpha, material.alpha.x >= 4.0), alpha);
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
    if dual_blinn {
        // V and the geometric normal are normalised at the vertices, then
        // interpolated before the two half vectors are normalised.
        let light = normalize(-frame.sun_dir.xyz);
        let sun_half = i.blinn_view + light;
        let ambient_half = i.blinn_view + i.normal;
        let sun_dot = clamp(dot(n, sun_half / max(length(sun_half), 1e-8)), 0.0, 1.0);
        let ambient_dot = clamp(dot(n, ambient_half / max(length(ambient_half), 1e-8)), 0.0, 1.0);
        let parameters = material.effect_params[0];
        let side_mask = textureSample(t_mr, s_material, uv).g;
        let top_mask = textureSample(t_opacity, s_material, uv).g;
        let strength = mix(top_mask, side_mask, layer_weight) * parameters.z;
        let sun_spec = strength * pow(sun_dot, parameters.w);
        let ambient_spec = strength * pow(ambient_dot, parameters.w);
        let nl = max(dot(n, light), 0.0);
        var visibility = 1.0;
        if frame.sun_color.w > 0.0 && nl > 0.0 {
            visibility = mix(1.0, sun_shadow(i.world, max(dot(ng, light), 0.0), i.clip.xy), frame.sun_color.w);
        }
        var ao = 1.0;
        if frame.misc.x > 0.0 {
            ao = mix(1.0, textureLoad(ao_texture, vec2<i32>(i.clip.xy), 0).r, min(frame.misc.x, 1.0));
        }
        let alignment = max(dot(n, ng), 0.0);
        let baked = i.aux.rgb * (parameters.x * frame.baked_lighting.x / PI) * (ao * alignment * alignment);
        let direct = frame.sun_color.rgb * (frame.sun_dir.w * nl * visibility);
        let rgb = (base.rgb + vec3<f32>(sun_spec)) * direct + (base.rgb + vec3<f32>(ao * ambient_spec)) * baked;
        let fog = height_fog(i.world);
        let color = mix(rgb, fog.rgb, fog.a);
        if frame.misc.w > 0.5 {
            return gi_capture(color, i.world, front);
        }
        return vec4<f32>(color, 1.0);
    }
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
    if frame.sun_color.w > 0.0 && nl > 0.0 {
        visibility = mix(1.0, sun_shadow(i.world, max(dot(ng, l), 0.0),i.clip.xy), frame.sun_color.w);
    }
    let direct = (kd * base.rgb / PI + spec) * frame.sun_color.rgb * frame.sun_dir.w * nl * visibility;

    // Image-based lighting (split sum + SH9).
    let fr = f_schlick_roughness(nv, f0, roughness) * spec_weight;
    let kd_ibl = (vec3<f32>(1.0) - fr) * (1.0 - metallic);
    let irradiance = sh_irradiance(n);
    let r = reflect(-v, n);
    var prefiltered = textureSampleLevel(env_prefiltered, env_sampler, equirect_uv(r), roughness * frame.env.z).rgb * frame.env.x;
    prefiltered = probe_specular(i.world, r, roughness, prefiltered);
    // Screen-space reflections replace the environment where rays hit (ADR 0023).
    if frame.ssr.x > 0.0 && material.alpha.x < 2.0 {
        let s = textureLoad(ssr_texture, vec2<i32>(i.clip.xy), 0, 0);
        let gloss = select(1.0 - smoothstep(frame.ssr.y * 0.5, frame.ssr.y, roughness), 1.0, frame.ssr.z > 0.5);
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
    var diffuse_irr = gi_irradiance(i.world, n, v, irradiance * frame.env.x);
    if frame.ssr.w > 0.0 && material.alpha.x < 2.0 {
        diffuse_irr = textureLoad(ssr_texture, vec2<i32>(i.clip.xy), 1, 0).rgb * frame.ssr.w;
    }
    if material.surface.y == 6.0 {
        let st = material.effect_params[0];
        let half_texel = vec2<f32>(0.5) / vec2<f32>(textureDimensions(t_occlusion));
        let atlas_uv = clamp(i.uv * st.xy + st.zw, half_texel, vec2<f32>(1.0) - half_texel);
        let radiance = textureSample(t_occlusion, s_material, atlas_uv).rgb;
        let direction = textureSample(t_mr, s_material, atlas_uv);
        let half_lambert = dot(n, direction.rgb - vec3<f32>(0.5)) + 0.5;
        diffuse_irr = PI * radiance * half_lambert / max(direction.a, 0.0001);
    }
    // Baked vertex irradiance (effect 8) from the `_AUX` stream.
    if material.surface.y == 8.0 {
        diffuse_irr = i.aux.rgb * material.effect_params[0].x * frame.baked_lighting.x;
    }
    let ambient = (kd_ibl * base.rgb * diffuse_irr / PI + specular_ibl) * occlusion;
    let local = local_lights(i.world, i.clip.xy, ng, n, v, base.rgb, metallic, f0, max(a, 0.002), nv);
    let fog = height_fog(i.world);
    let color=mix(direct+local+ambient+emissive, fog.rgb, fog.a);
    if frame.misc.w > 0.5 {
        return gi_capture(color, i.world, front);
    }
    return vec4<f32>(select(color,color*alpha,material.alpha.x>=4.0),alpha);
}
