// SPDX-License-Identifier: MPL-2.0
// Hardware AS traversal only; no software triangle traversal fallback.
enable wgpu_ray_query;
struct Params { inverse_view_projection:mat4x4<f32>, background:vec4<f32>, size_count:vec4<u32> }
struct Metadata { normal:vec4<f32>, color:vec4<f32> }
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var scene:acceleration_structure;
@group(0) @binding(2) var<storage,read> metadata:array<Metadata>;
@group(0) @binding(3) var output:texture_storage_2d<rgba16float,write>;

fn occluded(origin:vec3<f32>,direction:vec3<f32>)->bool {
    var shadow:ray_query;
    rayQueryInitialize(&shadow,scene,RayDesc(5u,255u,0.0001,1e20,origin,direction));
    // FORCE_OPAQUE triangles produce no candidate callbacks; one proceed
    // completes traversal. Naga 29.0.3's Metal synchronous intersector keeps its
    // emulated ready flag true, so looping on that flag would never terminate.
    let shadow_done=rayQueryProceed(&shadow);
    return rayQueryGetCommittedIntersection(&shadow).kind==1u;
}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=params.size_count.xy){return;}
    let uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(params.size_count.xy);
    let ndc=vec2<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0);
    let a=params.inverse_view_projection*vec4<f32>(ndc,0.0,1.0);
    let b=params.inverse_view_projection*vec4<f32>(ndc,1.0,1.0);
    let origin=a.xyz/a.w;
    let ray=b.xyz/b.w-origin;
    let direction=normalize(ray);
    var primary:ray_query;
    rayQueryInitialize(&primary,scene,RayDesc(1u,255u,0.0001,length(ray),origin,direction));
    let primary_done=rayQueryProceed(&primary);
    let hit=rayQueryGetCommittedIntersection(&primary);
    var color=params.background;
    if hit.kind==1u && hit.primitive_index<params.size_count.z {
        let surface=metadata[hit.primitive_index];
        let light=normalize(vec3<f32>(0.45,0.85,0.3));
        let point=origin+direction*hit.t;
        let visibility=select(1.0,0.0,occluded(point+light*0.002,light));
        let diffuse=max(dot(surface.normal.xyz,light),0.0);
        color=vec4<f32>(surface.color.rgb*(0.18+0.82*diffuse*visibility),1.0);
    }
    textureStore(output,vec2<i32>(id.xy),color);
}
