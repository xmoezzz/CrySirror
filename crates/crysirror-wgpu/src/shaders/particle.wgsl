// SPDX-License-Identifier: MPL-2.0
// Camera-facing particle billboards (instanced triangle strips).
struct PIn {
    @builtin(vertex_index) vi: u32,
    @location(0) pos: vec3<f32>,
    @location(1) size: f32,
    @location(2) color: vec4<f32>,
};
struct POut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};
@vertex
fn vs_main(i: PIn) -> POut {
    var corners = array<vec2<f32>, 4>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0));
    let c = corners[i.vi];
    let right = vec3<f32>(frame.view[0][0], frame.view[1][0], frame.view[2][0]);
    let up = vec3<f32>(frame.view[0][1], frame.view[1][1], frame.view[2][1]);
    let world = i.pos + (right * c.x + up * c.y) * i.size;
    var o: POut;
    o.clip = frame.view_proj * vec4<f32>(world, 1.0);
    o.uv = c;
    o.color = i.color;
    return o;
}
fn falloff(uv: vec2<f32>) -> f32 {
    let r2 = dot(uv, uv);
    return select(0.0, (1.0 - r2) * (1.0 - r2), r2 < 1.0);
}
@fragment
fn fs_add(i: POut) -> @location(0) vec4<f32> {
    let a = falloff(i.uv) * i.color.a;
    return vec4<f32>(i.color.rgb * a, 0.0);
}
@fragment
fn fs_alpha(i: POut) -> @location(0) vec4<f32> {
    return vec4<f32>(i.color.rgb, falloff(i.uv) * i.color.a);
}
