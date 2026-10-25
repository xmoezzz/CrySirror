// SPDX-License-Identifier: MPL-2.0
// Single-sample depth + view-space normal + motion vector prepass.
struct PreOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) cur: vec4<f32>,
    @location(3) prev: vec4<f32>,
    @location(4) world:vec3<f32>,
    @location(5) world_normal:vec3<f32>,
};
@vertex
fn vs_main(v: VertexIn) -> PreOut {
    return transform_vertex(v);
}
// Entry points cannot call each other; both vertex paths share this.
fn transform_vertex(v: VertexIn) -> PreOut {
    let world = object.model * vec4<f32>(v.position, 1.0);
    let prev_world = object.prev_model * vec4<f32>(v.position, 1.0);
    var o: PreOut;
    o.clip = frame.view_proj * world;
    o.normal = (frame.view * vec4<f32>((object.normal * vec4<f32>(v.normal, 0.0)).xyz, 0.0)).xyz;
    o.uv = v.uv;
    o.world=world.xyz;
    o.world_normal=(object.normal*vec4<f32>(v.normal,0.0)).xyz;
    o.cur = frame.view_proj_nojitter * world;
    o.prev = frame.prev_view_proj * prev_world;
    return o;
}
struct PreTargets {
    @location(0) normal: vec4<f32>,
    @location(1) velocity: vec2<f32>,
};
@fragment
fn fs_main(i: PreOut, @builtin(front_facing) front: bool) -> PreTargets {
    let uv=material_uv(i.uv);
    let surface=sample_surface(uv,i.world,i.world_normal,frame.camera_pos.xyz,frame.misc.z);
    if material.alpha.x == 1.0 && surface.color.a < material.alpha.y {discard;}
    var n = normalize(i.normal);
    if !front {
        n = -n;
    }
    let cur = i.cur.xy / i.cur.w;
    let prev = i.prev.xy / i.prev.w;
    var o: PreTargets;
    o.normal = vec4<f32>(n, 1.0);
    // Screen-space delta in UV units (y down): current - previous.
    o.velocity = (cur - prev) * vec2<f32>(0.5, -0.5);
    return o;
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn) -> PreOut {
    let k = skin_matrix(s, object.skin.x);
    let kp = skin_matrix(s, object.skin.y);
    var w = v;
    w.position = (k * vec4<f32>(v.position, 1.0)).xyz;
    w.normal = (k * vec4<f32>(v.normal, 0.0)).xyz;
    var o = transform_vertex(w);
    o.prev = frame.prev_view_proj * object.prev_model * (kp * vec4<f32>(v.position, 1.0));
    return o;
}
