// SPDX-License-Identifier: MPL-2.0
// GPU-only bitonic permutation. No particle-state readback or CPU counts.
struct Particle { a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,d:vec4<f32>,e:vec4<f32>,metadata:vec4<u32> }
struct Params {model:mat4x4<f32>,camera:vec4<f32>,stage:vec4<u32>}
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var<storage,read> particles:array<Particle>;
@group(0) @binding(2) var<storage,read_write> indices:array<u32>;
@group(0) @binding(3) var<storage,read> indirect:array<u32>;
@group(0) @binding(4) var<storage,read> alive:array<u32>;
const DEAD:u32=0xffffffffu;
fn farther(a:u32,b:u32)->bool {
    if a==DEAD {return false;}
    if b==DEAD {return true;}
    let x=(params.model*vec4<f32>(particles[a].a.xyz,1.0)).xyz-params.camera.xyz;
    let y=(params.model*vec4<f32>(particles[b].a.xyz,1.0)).xyz-params.camera.xyz;
    let da=dot(x,x);let db=dot(y,y);
    return da>db || (da==db && a<b);
}
@compute @workgroup_size(256)
fn initialize(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=params.stage.z {return;}
    var value=DEAD;
    if id.x<indirect[1] {value=alive[id.x];}
    indices[id.x]=value;
}
@compute @workgroup_size(256)
fn sort(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;
    if i>=params.stage.z {return;}
    let j=i^params.stage.x;
    if j<=i || j>=params.stage.z {return;}
    let a=indices[i];let b=indices[j];
    let descending=(i&params.stage.y)==0u;
    if (descending && farther(b,a)) || (!descending && farther(a,b)) {
        indices[i]=b;indices[j]=a;
    }
}
