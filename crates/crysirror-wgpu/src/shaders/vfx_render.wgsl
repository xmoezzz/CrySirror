// SPDX-License-Identifier: MPL-2.0
// One renderer consumes both CPU-reference uploads and GPU-resident particles.
struct View {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    view: mat4x4<f32>,
    model: mat4x4<f32>,
    camera: vec4<f32>,
    // renderer (sprite=0/mesh=1), facing, sheet columns, sheet rows
    mode: vec4<u32>,
    // frames, fps (0 = over life), soft distance, procedural radial coverage
    animation: vec4<f32>,
    axis: vec4<f32>,
    screen: vec4<f32>,
    fog_color: vec4<f32>,
    // density, height falloff, base height, start distance
    fog_params: vec4<f32>,
}
struct Particle {
    position_age: vec4<f32>,
    velocity_lifetime: vec4<f32>,
    previous_position_rotation: vec4<f32>,
    color: vec4<f32>,
    size: vec4<f32>,
    metadata: vec4<u32>,
}
@group(0) @binding(0) var<uniform> frame: View;
@group(0) @binding(1) var<storage, read> particles: array<Particle>;
@group(0) @binding(2) var<storage, read> alive: array<u32>;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
@group(1) @binding(2) var opaque_depth: texture_depth_2d;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) tile: vec4<f32>,
}
fn safe_normal(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    return select(fallback, v * inverseSqrt(max(dot(v,v), 1e-12)), dot(v,v)>1e-10);
}
fn output_at(world:vec3<f32>,uv:vec2<f32>,color:vec4<f32>)->Out {
    var o:Out;
    o.world=world; o.uv=uv; o.color=color;o.tile=vec4<f32>(0.,0.,1.,1.);
    o.clip=frame.view_projection*vec4<f32>(world,1.0);
    return o;
}
@vertex
fn vs_sprite(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->Out {
    let p=particles[alive[instance]];
    let corners=array<vec2<f32>,6>(vec2<f32>(-1.,-1.),vec2<f32>(1.,-1.),vec2<f32>(1.,1.),vec2<f32>(-1.,-1.),vec2<f32>(1.,1.),vec2<f32>(-1.,1.));
    let corner=corners[vertex];
    let center=(frame.model*vec4<f32>(p.position_age.xyz,1.0)).xyz;
    var right=vec3<f32>(frame.view[0][0],frame.view[1][0],frame.view[2][0]);
    var up=vec3<f32>(frame.view[0][1],frame.view[1][1],frame.view[2][1]);
    if frame.mode.y>0u {
        var axis=(frame.model*vec4<f32>(p.velocity_lifetime.xyz,0.0)).xyz;
        if frame.mode.y==2u {axis=(frame.model*vec4<f32>(frame.axis.xyz,0.0)).xyz;}
        up=safe_normal(axis,up);
        right=safe_normal(cross(up,frame.camera.xyz-center),right);
    }
    let angle=p.previous_position_rotation.w;
    let cs=cos(angle);let sn=sin(angle);
    let q=vec2<f32>(cs*corner.x-sn*corner.y,sn*corner.x+cs*corner.y)*p.size.xy*0.5;
    let scale=vec2<f32>(length(frame.model[0].xyz),length(frame.model[1].xyz));
    let world=center+right*q.x*scale.x+up*q.y*scale.y;
    let life=max(p.velocity_lifetime.w,1e-6);
    let frames=max(u32(frame.animation.x),1u);
    var tile=min(u32(clamp(p.position_age.w/life,0.,0.999999)*f32(frames)),frames-1u);
    if frame.animation.y>0.0 {tile=u32(max(p.position_age.w,0.)*frame.animation.y)%frames;}
    let columns=max(frame.mode.z,1u);let rows=max(frame.mode.w,1u);
    let uv=(vec2<f32>(corner.x*0.5+0.5,0.5-corner.y*0.5)+vec2<f32>(f32(tile%columns),f32(tile/columns)))/vec2<f32>(f32(columns),f32(rows));
    var o=output_at(world,uv,p.color);
    let begin=vec2<f32>(f32(tile%columns),f32(tile/columns))/vec2<f32>(f32(columns),f32(rows));
    o.tile=vec4<f32>(begin,begin+vec2<f32>(1.0)/vec2<f32>(f32(columns),f32(rows)));
    return o;
}
struct MeshIn {
    @location(0) position:vec3<f32>,
    @location(1) uv:vec2<f32>,
}
@vertex
fn vs_mesh(v:MeshIn,@builtin(instance_index) instance:u32)->Out {
    let p=particles[alive[instance]];
    let a=p.previous_position_rotation.w;let cs=cos(a);let sn=sin(a);
    let q=v.position*vec3<f32>(p.size.xy,p.size.x);
    let rotated=vec3<f32>(cs*q.x-sn*q.y,sn*q.x+cs*q.y,q.z);
    return output_at((frame.model*vec4<f32>(p.position_age.xyz+rotated,1.0)).xyz,v.uv,p.color);
}
struct RibbonIn {
    @location(0) position:vec3<f32>,
    @location(1) uv:vec2<f32>,
    @location(2) color:vec4<f32>,
}
@vertex
fn vs_ribbon(v:RibbonIn)->Out {return output_at(v.position,v.uv,v.color);}

fn rgba(i:Out)->vec4<f32> {
    var uv=i.uv;
    if frame.mode.x==0u && frame.animation.w<0.5 {
        // Clamp to texel centres within this atlas cell, avoiding adjacent
        // flipbook-frame bleeding under linear filtering.
        let half_texel=vec2<f32>(0.5)/vec2<f32>(textureDimensions(image));
        uv=clamp(uv,i.tile.xy+half_texel,i.tile.zw-half_texel);
    }
    let tex=textureSample(image,image_sampler,uv);
    var color=tex*i.color;
    if frame.animation.w>0.5 {
        let xy=i.uv*2.0-vec2<f32>(1.0);
        let r2=dot(xy,xy);
        color.a*=select(0.0,(1.0-r2)*(1.0-r2),r2<1.0);
    }
    if frame.animation.z>0.0 {
        let size=textureDimensions(opaque_depth);
        let pixel=clamp(vec2<i32>(i.clip.xy),vec2<i32>(0),vec2<i32>(size)-vec2<i32>(1));
        let depth=textureLoad(opaque_depth,pixel,0);
        if depth<1.0 {
            let uv=(vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(size);
            let ndc=vec2<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0);
            let h=frame.inverse_view_projection*vec4<f32>(ndc,depth,1.0);
            let seen=-(frame.view*vec4<f32>(h.xyz/h.w,1.0)).z;
            let point=-(frame.view*vec4<f32>(i.world,1.0)).z;
            color.a*=clamp((seen-point)/frame.animation.z,0.0,1.0);
        }
    }
    if frame.fog_params.x>0.0 {
        let distance=max(length(i.world-frame.camera.xyz)-frame.fog_params.w,0.0);
        let density=frame.fog_params.x*exp(clamp(-frame.fog_params.y*(i.world.y-frame.fog_params.z),-40.,40.));
        let amount=1.0-exp(-density*distance);
        color=vec4<f32>(mix(color.rgb,frame.fog_color.rgb,amount),color.a);
    }
    return color;
}
@fragment fn fs_alpha(i:Out)->@location(0) vec4<f32> {return rgba(i);}
@fragment fn fs_premultiplied(i:Out)->@location(0) vec4<f32> {let c=rgba(i);return vec4<f32>(c.rgb*c.a,c.a);}
@fragment fn fs_additive(i:Out)->@location(0) vec4<f32> {let c=rgba(i);return vec4<f32>(c.rgb*c.a,0.0);}
@fragment fn fs_multiply(i:Out)->@location(0) vec4<f32> {let c=rgba(i);return vec4<f32>(mix(vec3<f32>(1.0),c.rgb,c.a),0.0);}
