// SPDX-License-Identifier: MPL-2.0
// Shared PBR definitions (ADR 0010). Prepended to pbr/prepass/sky shaders.
const PI: f32 = 3.14159265359;
const CASCADES: u32 = 4u;

struct Frame {
    // Jittered (TAA) view-projection used for rasterization.
    view_proj: mat4x4<f32>,
    // Unjittered matrices for reconstruction and motion vectors.
    inv_view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    view_proj_nojitter: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    cascades: array<mat4x4<f32>, 4>,
    // View-space far distance of each cascade.
    splits: vec4<f32>,
    camera_pos: vec4<f32>,
    // xyz: direction light travels, w: illuminance multiplier
    sun_dir: vec4<f32>,
    // rgb: linear color, w: 1 when shadows are enabled
    sun_color: vec4<f32>,
    // x: IBL intensity, y: env rotation (rad), z: prefiltered max mip, w: shadow texel size
    env: vec4<f32>,
    // xy: target size, zw: 1/size
    screen: vec4<f32>,
    // x: AO enabled (strength), y: frame index, z: time,
    // w: irradiance capture (1: output signed hit distance in alpha, ADR 0028)
    misc: vec4<f32>,
    sh: array<vec4<f32>, 9>,
    // x near, y far, z local light count, w 1/ln(far/near)
    clusters: vec4<f32>,
    // rgb color, a density
    fog_color: vec4<f32>,
    // x height falloff, y base height, z start distance, w max opacity (0 = off)
    fog_params: vec4<f32>,
    // x sun scattering, y sun exponent
    fog_sun: vec4<f32>,
    // x reflection intensity, y SSR fade roughness, z native glossy, w per-pixel GI intensity
    ssr: vec4<f32>,
    // Reflection probe slots (ADR 0024): [3k] position+intensity, [3k+1] half extent+blend
    // (extent 0 = empty), [3k+2].x parallax (1 box-projected, 0 infinite)
    probes: array<vec4<f32>, 24>,
    // Virtual shadow maps (ADR 0027): light view; x level-0 texel, y levels, z/w depth range
    vsm_view: mat4x4<f32>,
    vsm: vec4<f32>,
    // x enabled, y pixel footprint per meter of distance
    vsm2: vec4<f32>,
    // Irradiance volume (ADR 0028): centre + intensity, half extent + enabled, probe counts
    gi_center: vec4<f32>,
    gi_extent: vec4<f32>,
    gi_counts: vec4<f32>,
    rt_shadow: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var env_prefiltered: texture_2d<f32>;
@group(0) @binding(4) var env_sampler: sampler;
@group(0) @binding(5) var brdf_lut: texture_2d<f32>;
@group(0) @binding(6) var env_source: texture_2d<f32>;
@group(0) @binding(7) var ao_texture: texture_2d<f32>;
@group(0) @binding(8) var opaque_depth:texture_depth_2d;
@group(0) @binding(14) var ssr_texture: texture_2d_array<f32>;
@group(0) @binding(15) var probe_maps: texture_2d_array<f32>;

fn rotate_y(d: vec3<f32>, a: f32) -> vec3<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec3<f32>(c * d.x + s * d.z, d.y, -s * d.x + c * d.z);
}

// Equirectangular mapping; the CPU SH projection uses the exact inverse.
fn equirect_uv(d: vec3<f32>) -> vec2<f32> {
    let r = rotate_y(normalize(d), -frame.env.y);
    return vec2<f32>(atan2(r.z, r.x) / (2.0 * PI) + 0.5, acos(clamp(r.y, -1.0, 1.0)) / PI);
}

// Irradiance from 9 SH coefficients already multiplied (on the CPU) by the
// basis normalization and the clamped-cosine convolution constants.
fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    let r = rotate_y(normalize(n), -frame.env.y);
    let x = r.x;
    let y = r.y;
    let z = r.z;
    let e = frame.sh[0].xyz
        + frame.sh[1].xyz * y + frame.sh[2].xyz * z + frame.sh[3].xyz * x
        + frame.sh[4].xyz * (x * y) + frame.sh[5].xyz * (y * z)
        + frame.sh[6].xyz * (3.0 * z * z - 1.0)
        + frame.sh[7].xyz * (x * z) + frame.sh[8].xyz * (x * x - y * y);
    return max(e, vec3<f32>(0.0));
}


struct Object {
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
    prev_model: mat4x4<f32>,
    // x: palette offset, y: previous-frame palette offset, z: 1 when skinned
    skin: vec4<u32>,
};
@group(2) @binding(0) var<uniform> object: Object;
@group(2) @binding(1) var<storage, read> palettes: array<mat4x4<f32>>;

struct SkinIn {
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
};
fn skin_matrix(s: SkinIn, base: u32) -> mat4x4<f32> {
    return palettes[base + s.joints.x] * s.weights.x
        + palettes[base + s.joints.y] * s.weights.y
        + palettes[base + s.joints.z] * s.weights.z
        + palettes[base + s.joints.w] * s.weights.w;
}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};

// Exponential height fog (opacity, inscattered color) for a ray from the
// camera to `world`: ∫ density·e^(−falloff·(h−base)) along the ray.
fn height_fog(world: vec3<f32>) -> vec4<f32> {
    if frame.fog_params.w <= 0.0 {
        return vec4<f32>(0.0);
    }
    let cam = frame.camera_pos.xyz;
    let ray = world - cam;
    let len = length(ray);
    let dist = max(len - frame.fog_params.z, 0.0);
    let falloff = frame.fog_params.x;
    let dy = ray.y * dist / max(len, 1e-4);
    var integral = frame.fog_color.a * dist * exp(-falloff * (cam.y - frame.fog_params.y));
    let k = falloff * dy;
    if abs(k) > 1e-4 {
        integral = integral * (1.0 - exp(-k)) / k;
    }
    let opacity = min(1.0 - exp(-max(integral, 0.0)), frame.fog_params.w);
    var color = frame.fog_color.rgb;
    if frame.fog_sun.x > 0.0 && frame.sun_dir.w > 0.0 {
        let cos_sun = max(dot(ray / max(len, 1e-4), -frame.sun_dir.xyz), 0.0);
        color = color + frame.sun_color.rgb * frame.fog_sun.x * pow(cos_sun, frame.fog_sun.y);
    }
    return vec4<f32>(color, opacity);
}
