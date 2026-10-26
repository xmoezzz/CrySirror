// SPDX-License-Identifier: MPL-2.0
// Shared ray-query scene access (ADR 0055-0058): instance table, packed
// corner attributes, baked material atlas, alpha-tested re-tracing and
// random sequences. Every query proceeds once on force-opaque geometry
// (see hardware_rt.wgsl for the Naga 29.0.3 Metal constraint).
enable wgpu_ray_query;
// x: first attribute corner of the instance's geometry, y: material slot + 1 (0 = none).
struct Instance { data:vec4<u32> }
// rect: atlas UV origin/size; transform: material UV scale/offset;
// cutoff: x alpha cutoff, y 1 when alpha-tested; emissive: linear rgb.
struct Slot { rect:vec4<f32>, transform:vec4<f32>, cutoff:vec4<f32>, emissive:vec4<f32> }
@group(0) @binding(1) var scene:acceleration_structure;
@group(0) @binding(6) var<storage,read> instances:array<Instance>;
@group(0) @binding(7) var<storage,read> attributes:array<vec2<u32>>;
@group(0) @binding(8) var<storage,read> slots:array<Slot>;
@group(0) @binding(9) var atlas:texture_2d<f32>;
@group(0) @binding(10) var atlas_sampler:sampler;

fn corner_uv(base:u32,b:vec2<f32>)->vec2<f32> {
    return unpack2x16float(attributes[base].x)*(1.0-b.x-b.y)+unpack2x16float(attributes[base+1u].x)*b.x+unpack2x16float(attributes[base+2u].x)*b.y;
}
fn decode_octahedral(e:vec2<f32>)->vec3<f32> {
    let f=e*2.0-vec2<f32>(1.0);
    var n=vec3<f32>(f.x,f.y,1.0-abs(f.x)-abs(f.y));
    let t=max(-n.z,0.0);
    n=vec3<f32>(n.x+select(t,-t,n.x>=0.0),n.y+select(t,-t,n.y>=0.0),n.z);
    return normalize(n);
}
fn corner_normal(base:u32,b:vec2<f32>)->vec3<f32> {
    let n=decode_octahedral(unpack2x16unorm(attributes[base].y))*(1.0-b.x-b.y)
        +decode_octahedral(unpack2x16unorm(attributes[base+1u].y))*b.x
        +decode_octahedral(unpack2x16unorm(attributes[base+2u].y))*b.y;
    return normalize(n);
}
// Atlas texel of the hit: linear base colour and coverage alpha.
fn hit_material(hit:RayIntersection)->vec4<f32> {
    let inst=instances[hit.instance_custom_data].data;
    if inst.y==0u {return vec4<f32>(0.5,0.5,0.5,1.0);}
    let slot=slots[inst.y-1u];
    let uv=corner_uv(inst.x+hit.primitive_index*3u,hit.barycentrics);
    let tiled=fract(uv*slot.transform.xy+slot.transform.zw);
    return textureSampleLevel(atlas,atlas_sampler,slot.rect.xy+tiled*slot.rect.zw,0.0);
}
const MAX_LAYERS:u32=8u;

// Coverage of the committed hit; 1 for opaque casters.
fn coverage(hit:RayIntersection)->f32 {
    let inst=instances[hit.instance_custom_data].data;
    if inst.y==0u {return 1.0;}
    let slot=slots[inst.y-1u];
    if slot.cutoff.y<0.5 {return 1.0;}
    let base=inst.x+hit.primitive_index*3u;
    let b=hit.barycentrics;
    let uv=unpack2x16float(attributes[base].x)*(1.0-b.x-b.y)+unpack2x16float(attributes[base+1u].x)*b.x+unpack2x16float(attributes[base+2u].x)*b.y;
    let tiled=fract(uv*slot.transform.xy+slot.transform.zw);
    let alpha=textureSampleLevel(atlas,atlas_sampler,slot.rect.xy+tiled*slot.rect.zw,0.0).a;
    return select(0.0,1.0,alpha>=slot.cutoff.x);
}

fn visible(origin:vec3<f32>,direction:vec3<f32>,t_max:f32)->f32 {
    var t_min=0.0001;
    for (var layer=0u; layer<MAX_LAYERS; layer++) {
        if t_min>=t_max {return 1.0;}
        var query:ray_query;
        rayQueryInitialize(&query,scene,RayDesc(1u,255u,t_min,t_max,origin,direction));
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


// Closest hit whose material covers the ray (alpha-tested layers skipped);
// kind 0 when nothing covered is hit before t_max.
fn closest(origin:vec3<f32>,direction:vec3<f32>,t_max:f32)->RayIntersection {
    var none:RayIntersection;
    var t_min=0.0001;
    for (var layer=0u; layer<MAX_LAYERS; layer++) {
        if t_min>=t_max {return none;}
        var query:ray_query;
        rayQueryInitialize(&query,scene,RayDesc(1u,255u,t_min,t_max,origin,direction));
        let done=rayQueryProceed(&query);
        let hit=rayQueryGetCommittedIntersection(&query);
        if hit.kind!=1u || coverage(hit)>0.5 {return hit;}
        t_min=hit.t*(1.0+1e-5)+1e-4;
    }
    return none;
}
