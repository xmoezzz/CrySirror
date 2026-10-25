// SPDX-License-Identifier: MPL-2.0
// Reflection capture (ADR 0024): six perspective faces rendered with
// explicit forward/up bases → equirectangular radiance with the same
// direction convention as the environment prefilter.
struct Faces {
    // forward (xyz), up (xyz) per face
    f: array<vec4<f32>, 12>,
};
@group(0) @binding(0) var<uniform> faces: Faces;
@group(0) @binding(1) var face_tex: texture_2d_array<f32>;
@group(0) @binding(2) var face_sampler: sampler;

fn probe_dir_from_uv(uv: vec2<f32>) -> vec3<f32> {
    let phi = (uv.x - 0.5) * 2.0 * PI;
    let theta = uv.y * PI;
    return vec3<f32>(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
}

@fragment
fn fs_cube_to_equirect(i: FsOut) -> @location(0) vec4<f32> {
    let d = probe_dir_from_uv(i.uv);
    var best = 0;
    var bd = -2.0;
    for (var k = 0; k < 6; k = k + 1) {
        let s = dot(d, faces.f[2 * k].xyz);
        if s > bd {
            bd = s;
            best = k;
        }
    }
    let f = faces.f[2 * best].xyz;
    let u = faces.f[2 * best + 1].xyz;
    let r = cross(f, u);
    let uv = vec2<f32>(dot(d, r) / bd * 0.5 + 0.5, 0.5 - dot(d, u) / bd * 0.5);
    return vec4<f32>(textureSampleLevel(face_tex, face_sampler, uv, best, 0.0).rgb, 1.0);
}
