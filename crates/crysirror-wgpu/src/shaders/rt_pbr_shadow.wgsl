// SPDX-License-Identifier: MPL-2.0
// PBR sun visibility from native ray queries over the renderer's instanced
// scene (ADR 0051, 0055). Alpha-tested casters are resolved by re-tracing
// past transparent hits: every query still uses one rayQueryProceed on
// force-opaque geometry (see hardware_rt.wgsl for the Naga 29.0.3 Metal
// constraint), so no candidate callbacks are needed on any backend.
enable wgpu_ray_query;
// soft: x cosine of the sun's angular radius (1 = hard), y samples per pixel, z frame.
struct Params { inverse_view_projection:mat4x4<f32>, inverse_view:mat4x4<f32>, towards_sun:vec4<f32>, size:vec4<u32>, soft:vec4<f32> }
// x: first UV corner of the instance's geometry, y: alpha slot + 1 (0 = opaque).
struct Instance { data:vec4<u32> }
// rect: atlas UV origin/size; transform: material UV scale/offset; cutoff.x.
struct Slot { rect:vec4<f32>, transform:vec4<f32>, cutoff:vec4<f32> }
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var scene:acceleration_structure;
@group(0) @binding(2) var depth:texture_depth_2d;
@group(0) @binding(3) var normals:texture_2d<f32>;
@group(0) @binding(4) var output:texture_storage_2d<rgba16float,write>;
@group(0) @binding(5) var ao:texture_2d<f32>;
@group(0) @binding(6) var<storage,read> instances:array<Instance>;
@group(0) @binding(7) var<storage,read> uvs:array<vec2<f32>>;
@group(0) @binding(8) var<storage,read> slots:array<Slot>;
@group(0) @binding(9) var atlas:texture_2d<f32>;
@group(0) @binding(10) var atlas_sampler:sampler;

const MAX_LAYERS:u32=8u;

// Coverage of the committed hit; 1 for opaque casters.
fn coverage(hit:RayIntersection)->f32 {
    let inst=instances[hit.instance_custom_data].data;
    if inst.y==0u {return 1.0;}
    let base=inst.x+hit.primitive_index*3u;
    let b=hit.barycentrics;
    let uv=uvs[base]*(1.0-b.x-b.y)+uvs[base+1u]*b.x+uvs[base+2u]*b.y;
    let slot=slots[inst.y-1u];
    let tiled=fract(uv*slot.transform.xy+slot.transform.zw);
    let alpha=textureSampleLevel(atlas,atlas_sampler,slot.rect.xy+tiled*slot.rect.zw,0.0).r;
    return select(0.0,1.0,alpha>=slot.cutoff.x);
}

fn visible(origin:vec3<f32>,direction:vec3<f32>)->f32 {
    var t_min=0.0001;
    for (var layer=0u; layer<MAX_LAYERS; layer++) {
        var query:ray_query;
        rayQueryInitialize(&query,scene,RayDesc(5u,255u,t_min,1e20,origin,direction));
        let done=rayQueryProceed(&query);
        let hit=rayQueryGetCommittedIntersection(&query);
        if hit.kind!=1u {return 1.0;}
        if coverage(hit)>0.5 {return 0.0;}
        t_min=hit.t*(1.0+1e-5)+1e-4;
    }
    // A stack deeper than MAX_LAYERS transparent layers counts as occluded.
    return 0.0;
}

// PCG hash; per pixel, frame and sample decorrelated sequences.
fn pcg(v:u32)->u32 {
    let state=v*747796405u+2891336453u;
    let word=((state>>((state>>28u)+4u))^state)*277803737u;
    return (word>>22u)^word;
}
fn random2(pixel:vec2<u32>,frame:u32,k:u32)->vec2<f32> {
    let a=pcg(pixel.x+pcg(pixel.y+pcg(frame*16u+k)));
    let b=pcg(a);
    return vec2<f32>(f32(a)/4294967296.0,f32(b)/4294967296.0);
}
// Uniform direction inside the cone around `axis`.
fn cone(axis:vec3<f32>,cos_max:f32,u:vec2<f32>)->vec3<f32> {
    let cos_t=1.0-u.x*(1.0-cos_max);
    let sin_t=sqrt(max(1.0-cos_t*cos_t,0.0));
    let phi=6.28318530718*u.y;
    let helper=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(axis.x)>0.9);
    let t=normalize(cross(helper,axis));
    let b=cross(axis,t);
    return normalize(t*(cos(phi)*sin_t)+b*(sin(phi)*sin_t)+axis*cos_t);
}

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
        let origin=world+normal*0.002+light*0.001;
        if params.soft.x>=1.0 {
            visibility=visible(origin,light);
        } else {
            let samples=max(u32(params.soft.y),1u);
            var sum=0.0;
            for (var k=0u; k<samples; k++) {
                sum+=visible(origin,cone(light,params.soft.x,random2(id.xy,u32(params.soft.z),k)));
            }
            visibility=sum/f32(samples);
        }
    }
    textureStore(output,pixel,vec4<f32>(textureLoad(ao,pixel,0).r,visibility,0.0,1.0));
}

// No casters: keep ambient occlusion, set visibility to one (no dummy AS).
@compute @workgroup_size(8,8)
fn clear(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=textureDimensions(output)){return;}
    let pixel=vec2<i32>(id.xy);
    textureStore(output,pixel,vec4<f32>(textureLoad(ao,pixel,0).r,1.0,0.0,1.0));
}
