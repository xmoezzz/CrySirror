// SPDX-License-Identifier: MPL-2.0
struct Light {view_proj:mat4x4<f32>,camera_time:vec4<f32>};
@group(0) @binding(0) var<uniform> light:Light;
struct Object {model:mat4x4<f32>,normal:mat4x4<f32>,prev_model:mat4x4<f32>,skin:vec4<u32>};
@group(1) @binding(0) var<uniform> object:Object;
@group(1) @binding(1) var<storage,read> palettes:array<mat4x4<f32>>;
struct Out {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
    @location(1) world:vec3<f32>,
    @location(2) normal:vec3<f32>,
};
fn shadow_vertex(position:vec3<f32>,normal:vec3<f32>,uv:vec2<f32>)->Out {
    let world=object.model*vec4<f32>(position,1.0);
    var o:Out; o.position=light.view_proj*world; o.uv=uv; o.world=world.xyz;
    o.normal=(object.normal*vec4<f32>(normal,0.0)).xyz; return o;
}
@vertex fn vs_main(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>)->Out {
    return shadow_vertex(position,normal,uv);
}
@vertex fn vs_skinned(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,@location(4) joints:vec4<u32>,@location(5) weights:vec4<f32>)->Out {
    let b=object.skin.x;
    let k=palettes[b+joints.x]*weights.x+palettes[b+joints.y]*weights.y+palettes[b+joints.z]*weights.z+palettes[b+joints.w]*weights.w;
    return shadow_vertex((k*vec4<f32>(position,1.0)).xyz,(k*vec4<f32>(normal,0.0)).xyz,uv);
}
@fragment fn fs_main(i:Out) {
    let uv=material_uv(i.uv);
    let surface=sample_surface(uv,i.world,i.normal,light.camera_time.xyz,light.camera_time.w);
    if material.alpha.x==1.0 && surface.color.a<material.alpha.y {discard;}
}
