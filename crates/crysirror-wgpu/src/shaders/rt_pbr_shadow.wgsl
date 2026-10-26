// SPDX-License-Identifier: MPL-2.0
// PBR sun visibility from native ray queries over the renderer's instanced
// scene (ADR 0051, 0055). Alpha-tested casters are resolved by re-tracing
// past transparent hits: every query still uses one rayQueryProceed on
// force-opaque geometry (see hardware_rt.wgsl for the Naga 29.0.3 Metal
// constraint), so no candidate callbacks are needed on any backend.
struct Params {
    inverse_view_projection:mat4x4<f32>, inverse_view:mat4x4<f32>, towards_sun:vec4<f32>,
    size:vec4<u32>, soft:vec4<f32>, local:array<vec4<f32>,2>, local_range:array<vec4<f32>,2>,
}
@group(0) @binding(0) var<uniform> params:Params;
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
    var local=array<f32,2>(1.0,1.0);
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
        let samples=max(u32(params.soft.y),1u);
        if params.soft.x>=1.0 {
            visibility=visible(origin,light,1e20);
        } else {
            var sum=0.0;
            for (var k=0u; k<samples; k++) {
                sum+=visible(origin,cone(light,params.soft.x,random2(id.xy,u32(params.soft.z),k)),1e20);
            }
            visibility=sum/f32(samples);
        }
        let surface=world+normal*0.002;
        for (var i=0u; i<2u; i++) {
            let l=params.local[i];
            let info=params.local_range[i];
            if info.y<0.5 {continue;}
            let to=l.xyz-surface;
            let dist=length(to);
            // Out of range or at the emitter: unlit anyway / undefined.
            if dist>=info.x || dist<=l.w+1e-3 {continue;}
            var sum=0.0;
            let count=select(1u,samples,l.w>0.0);
            for (var k=0u; k<count; k++) {
                var target_point=l.xyz;
                if l.w>0.0 {
                    // Uniform point on the emitter disc facing the surface.
                    let u=random2(id.xy,u32(params.soft.z)+7919u*(i+1u),k);
                    let axis=to/dist;
                    let helper=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(axis.x)>0.9);
                    let t=normalize(cross(helper,axis));
                    let b=cross(axis,t);
                    let r=l.w*sqrt(u.x);
                    let phi=6.28318530718*u.y;
                    target_point=l.xyz+t*(r*cos(phi))+b*(r*sin(phi));
                }
                let d=target_point-surface;
                let len=length(d);
                // Stop short of the emitter so its own proxy geometry never shadows it.
                sum+=visible(surface,d/len,max(len-l.w-0.01,0.0002));
            }
            local[i]=sum/f32(count);
        }
    }
    textureStore(output,pixel,vec4<f32>(textureLoad(ao,pixel,0).r,visibility,local[0],local[1]));
}

// No casters: keep ambient occlusion, set visibility to one (no dummy AS).
@compute @workgroup_size(8,8)
fn clear(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=textureDimensions(output)){return;}
    let pixel=vec2<i32>(id.xy);
    textureStore(output,pixel,vec4<f32>(textureLoad(ao,pixel,0).r,1.0,1.0,1.0));
}
