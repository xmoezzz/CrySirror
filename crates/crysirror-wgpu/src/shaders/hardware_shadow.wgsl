// SPDX-License-Identifier: MPL-2.0
// Visibility reconstructed from the same single-sample depth/normal prepass.
enable wgpu_ray_query;
struct Params { inverse_view_projection:mat4x4<f32>, inverse_view:mat4x4<f32>, towards_sun:vec4<f32>, size:vec4<u32> }
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var scene:acceleration_structure;
@group(0) @binding(2) var depth:texture_depth_2d;
@group(0) @binding(3) var normals:texture_2d<f32>;
@group(0) @binding(4) var output:texture_storage_2d<rgba16float,write>;
@group(0) @binding(5) var ao:texture_2d<f32>;
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=params.size.xy){return;}
    let pixel=vec2<i32>(id.xy);
    let d=textureLoad(depth,pixel,0);
    let n=textureLoad(normals,pixel,0);
    var visibility=1.0;
    if d<1.0 && n.w>0.0 {
        let uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(params.size.xy);
        let ndc=vec2<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0);
        let world_h=params.inverse_view_projection*vec4<f32>(ndc,d,1.0);
        let world=world_h.xyz/world_h.w;
        let normal_w=(params.inverse_view*vec4<f32>(n.xyz,0.0)).xyz;
        let normal_squared=dot(normal_w,normal_w);
        var normal=vec3<f32>(0.0,1.0,0.0);
        if normal_squared>1e-12 && normal_squared<1e30 {normal=normal_w*inverseSqrt(normal_squared);}
        let light=params.towards_sun.xyz;
        // Fixed world-unit bias for the initial opaque hard-shadow profile.
        let origin=world+normal*0.002+light*0.001;
        var query:ray_query;
        rayQueryInitialize(&query,scene,RayDesc(5u,255u,0.0001,1e20,origin,light));
        // Opaque triangles need no candidate loop. See hardware_rt.wgsl for
        // Naga 29.0.3's synchronous Metal intersector/ready-flag constraint.
        let done=rayQueryProceed(&query);
        visibility=select(1.0,0.0,rayQueryGetCommittedIntersection(&query).kind==1u);
    }
    textureStore(output,pixel,vec4<f32>(textureLoad(ao,pixel,0).r,visibility,0.0,1.0));
}

// Empty caster scene: preserve ambient occlusion, clear only visibility to one.
// This entry uses no acceleration structure and needs no dummy/zero-sized BLAS.
@compute @workgroup_size(8,8)
fn clear(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=textureDimensions(output)){return;}
    let pixel=vec2<i32>(id.xy);
    textureStore(output,pixel,vec4<f32>(textureLoad(ao,pixel,0).r,1.0,0.0,1.0));
}
