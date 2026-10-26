// SPDX-License-Identifier: MPL-2.0
// Denoising for stochastic ray-traced visibility (ADR 0056): temporal
// accumulation with depth-validated reprojection, then an edge-aware
// spatial filter whose radius shrinks as history accumulates.
struct Params {
    inverse_view_projection:mat4x4<f32>,
    view:mat4x4<f32>,
    prev_view_projection:mat4x4<f32>,
    prev_view:mat4x4<f32>,
    // x,y: size; z: history valid (0/1); w: max history length
    size:vec4<u32>,
}
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var depth:texture_depth_2d;
@group(0) @binding(2) var noisy:texture_2d<f32>;
@group(0) @binding(3) var history:texture_2d<f32>;
@group(0) @binding(4) var history_out:texture_storage_2d<rgba16float,write>;
@group(0) @binding(5) var normals:texture_2d<f32>;
@group(0) @binding(6) var ao:texture_2d<f32>;
@group(0) @binding(7) var output:texture_storage_2d<rgba16float,write>;

fn world_at(pixel:vec2<i32>,d:f32)->vec3<f32> {
    let uv=(vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(params.size.xy);
    let ndc=vec2<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0);
    let w=params.inverse_view_projection*vec4<f32>(ndc,d,1.0);
    return w.xyz/w.w;
}
fn view_depth(m:mat4x4<f32>,world:vec3<f32>)->f32 { return -(m*vec4<f32>(world,1.0)).z; }

// history_out: x visibility, y view depth, z accumulated samples.
@compute @workgroup_size(8,8)
fn temporal(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=params.size.xy){return;}
    let pixel=vec2<i32>(id.xy);
    let d=textureLoad(depth,pixel,0);
    let sample=textureLoad(noisy,pixel,0).y;
    if d>=1.0 {
        textureStore(history_out,pixel,vec4<f32>(1.0,0.0,0.0,1.0));
        return;
    }
    let world=world_at(pixel,d);
    let current_depth=view_depth(params.view,world);
    var visibility=sample;
    var count=1.0;
    if params.size.z==1u {
        let clip=params.prev_view_projection*vec4<f32>(world,1.0);
        let ndc=clip.xy/clip.w;
        let uv=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5);
        if clip.w>0.0 && all(uv>=vec2<f32>(0.0)) && all(uv<vec2<f32>(1.0)) {
            let prev_pixel=vec2<i32>(uv*vec2<f32>(params.size.xy));
            let h=textureLoad(history,prev_pixel,0);
            let expected=view_depth(params.prev_view,world);
            if h.z>0.0 && abs(h.y-expected)<=0.03*max(expected,0.001) {
                count=min(h.z+1.0,f32(params.size.w));
                visibility=mix(h.x,sample,1.0/count);
            }
        }
    }
    textureStore(history_out,pixel,vec4<f32>(visibility,current_depth,count,1.0));
}

@compute @workgroup_size(8,8)
fn spatial(@builtin(global_invocation_id) id:vec3<u32>){
    if any(id.xy>=params.size.xy){return;}
    let pixel=vec2<i32>(id.xy);
    let center=textureLoad(history,pixel,0);
    let ambient=textureLoad(ao,pixel,0).r;
    if center.z<=0.0 {
        textureStore(output,pixel,vec4<f32>(ambient,1.0,0.0,1.0));
        return;
    }
    let n0=textureLoad(normals,pixel,0).xyz;
    // Young history is noisier: widen the footprint until it converges.
    let step=select(1,2,center.z<4.0);
    var sum=0.0;
    var weight=0.0;
    for (var y=-2; y<=2; y++) {
        for (var x=-2; x<=2; x++) {
            let q=clamp(pixel+vec2<i32>(x,y)*step,vec2<i32>(0),vec2<i32>(params.size.xy)-vec2<i32>(1));
            let h=textureLoad(history,q,0);
            if h.z<=0.0 {continue;}
            let n=textureLoad(normals,q,0).xyz;
            let spatial_w=exp(-f32(x*x+y*y)/4.5);
            let depth_w=exp(-abs(h.y-center.y)/(0.02*max(center.y,0.001)));
            let normal_w=pow(max(dot(n,n0),0.0),16.0);
            let w=spatial_w*depth_w*normal_w;
            sum+=h.x*w;
            weight+=w;
        }
    }
    let visibility=select(center.x,sum/weight,weight>1e-4);
    textureStore(output,pixel,vec4<f32>(ambient,visibility,0.0,1.0));
}
