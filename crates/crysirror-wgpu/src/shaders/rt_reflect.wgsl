// SPDX-License-Identifier: MPL-2.0
// Ray-traced mirror reflections (ADR 0058), appended to rt_common.wgsl. The
// reflected ray's closest covered hit is resolved in two ways:
// - visible in this frame's depth: the previous frame's HDR colour at the
//   hit's reprojected position (textured, fully lit);
// - otherwise: the baked material colour, sun with a ray-traced shadow, SH
//   environment irradiance and emission.
// Output matches screen-space reflections: rgb radiance, a = 1 on a hit
// (0 keeps the environment/probe reflection in the forward pass).
struct Params {
    inverse_view_projection:mat4x4<f32>,
    inverse_view:mat4x4<f32>,
    view_projection:mat4x4<f32>,
    // World → previous frame clip space.
    reproject:mat4x4<f32>,
    camera:vec4<f32>,
    // xyz: direction light travels, w: illuminance
    sun_dir:vec4<f32>,
    // rgb: linear colour, w: 1 with shadows
    sun_color:vec4<f32>,
    // x: IBL intensity, y: environment rotation (rad)
    env:vec4<f32>,
    sh:array<vec4<f32>,9>,
    // xy size, z previous frame valid
    size:vec4<u32>,
    // x max distance
    reflection:vec4<f32>,
}
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(2) var depth:texture_depth_2d;
@group(0) @binding(3) var normals:texture_2d<f32>;
@group(0) @binding(4) var prev_color:texture_2d<f32>;
@group(0) @binding(5) var prev_sampler:sampler;
@group(0) @binding(11) var output:texture_storage_2d<rgba16float,write>;

fn rotate_y(v:vec3<f32>,a:f32)->vec3<f32> {
    let c=cos(a); let s=sin(a);
    return vec3<f32>(c*v.x+s*v.z,v.y,-s*v.x+c*v.z);
}
fn sh_irradiance(n:vec3<f32>)->vec3<f32> {
    let r=rotate_y(normalize(n),-params.env.y);
    let x=r.x; let y=r.y; let z=r.z;
    let e=params.sh[0].xyz+params.sh[1].xyz*y+params.sh[2].xyz*z+params.sh[3].xyz*x
        +params.sh[4].xyz*(x*y)+params.sh[5].xyz*(y*z)+params.sh[6].xyz*(3.0*z*z-1.0)
        +params.sh[7].xyz*(x*z)+params.sh[8].xyz*(x*x-y*y);
    return max(e,vec3<f32>(0.0));
}

// Radiance leaving the hit towards the ray origin.
fn shade(hit:RayIntersection,origin:vec3<f32>,direction:vec3<f32>)->vec3<f32> {
    let point=origin+direction*hit.t;
    // Visible this frame: reuse the previous frame's lit, textured colour.
    if params.size.z==1u {
        let clip=params.view_projection*vec4<f32>(point,1.0);
        if clip.w>0.0 {
            let ndc=clip.xy/clip.w;
            let uv=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5);
            if all(uv>=vec2<f32>(0.0)) && all(uv<vec2<f32>(1.0)) {
                let pixel=vec2<i32>(uv*vec2<f32>(params.size.xy));
                let d=textureLoad(depth,pixel,0);
                let s=params.inverse_view_projection*vec4<f32>(ndc,d,1.0);
                let seen=s.xyz/s.w;
                let expected=length(point-params.camera.xyz);
                if d<1.0 && abs(length(seen-params.camera.xyz)-expected)<=0.02*expected {
                    let prev=params.reproject*vec4<f32>(point,1.0);
                    if prev.w>0.0 {
                        let pn=prev.xy/prev.w;
                        let prev_uv=vec2<f32>(pn.x*0.5+0.5,0.5-pn.y*0.5);
                        if all(prev_uv>=vec2<f32>(0.0)) && all(prev_uv<vec2<f32>(1.0)) {
                            return textureSampleLevel(prev_color,prev_sampler,prev_uv,0.0).rgb;
                        }
                    }
                }
            }
        }
    }
    let inst=instances[hit.instance_custom_data].data;
    var n=normalize(hit.object_to_world*vec4<f32>(corner_normal(inst.x+hit.primitive_index*3u,hit.barycentrics),0.0));
    if dot(n,direction)>0.0 {n=-n;}
    let albedo=hit_material(hit).rgb;
    var emissive=vec3<f32>(0.0);
    if inst.y>0u {emissive=slots[inst.y-1u].emissive.rgb;}
    let l=-params.sun_dir.xyz;
    let nl=max(dot(n,l),0.0);
    var sun=0.0;
    if nl>0.0 {
        sun=nl;
        if params.sun_color.w>0.5 {sun*=visible(point+n*0.002,l,1e20);}
    }
    let irradiance=params.sun_color.rgb*params.sun_dir.w*sun+sh_irradiance(n)*params.env.x;
    return albedo/3.14159265359*irradiance+emissive;
}

@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=params.size.xy){return;}
    let pixel=vec2<i32>(id.xy);
    let d=textureLoad(depth,pixel,0);
    let nv=textureLoad(normals,pixel,0);
    var result=vec4<f32>(0.0);
    if d<1.0 && nv.w>0.0 {
        let uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(params.size.xy);
        let ndc=vec2<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0);
        let w=params.inverse_view_projection*vec4<f32>(ndc,d,1.0);
        let world=w.xyz/w.w;
        let n=normalize((params.inverse_view*vec4<f32>(nv.xyz,0.0)).xyz);
        let v=normalize(world-params.camera.xyz);
        let r=reflect(v,n);
        let origin=world+n*0.002;
        let hit=closest(origin,r,params.reflection.x);
        if hit.kind==1u {
            result=vec4<f32>(shade(hit,origin,r),1.0);
        }
    }
    textureStore(output,pixel,result);
}
