// SPDX-License-Identifier: MPL-2.0
// Per-view compact visible indices and draw arguments stay on the GPU.
struct Particle {a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,d:vec4<f32>,e:vec4<f32>,metadata:vec4<u32>}
struct Params {view_projection:mat4x4<f32>,model:mat4x4<f32>,options:vec4<f32>}
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var<storage,read> particles:array<Particle>;
@group(0) @binding(2) var<storage,read> alive:array<u32>;
@group(0) @binding(3) var<storage,read> indirect:array<u32>;
@group(0) @binding(4) var<storage,read_write> visible:array<atomic<u32>>;
@compute @workgroup_size(1)
fn initialize() {
    atomicStore(&visible[0],u32(params.options.w));atomicStore(&visible[1],0u);
    atomicStore(&visible[2],0u);atomicStore(&visible[3],0u);
}
fn inside(plane:vec4<f32>,p:vec3<f32>,radius:f32)->bool {return dot(plane.xyz,p)+plane.w>=-radius*length(plane.xyz);}
@compute @workgroup_size(256)
fn compact(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=min(indirect[1],u32(params.options.z)) {return;}
    let slot=alive[id.x];let particle=particles[slot];
    let p=(params.model*vec4<f32>(particle.a.xyz,1.)).xyz;
    let scale=max(max(length(params.model[0].xyz),length(params.model[1].xyz)),length(params.model[2].xyz));
    let radius=params.options.x*max(particle.e.x,particle.e.y)*scale*1.5;
    let rows=transpose(params.view_projection);
    if !inside(rows[3]+rows[0],p,radius)||!inside(rows[3]-rows[0],p,radius)
       ||!inside(rows[3]+rows[1],p,radius)||!inside(rows[3]-rows[1],p,radius)
       ||!inside(rows[2],p,radius)||!inside(rows[3]-rows[2],p,radius) {return;}
    let at=atomicAdd(&visible[1],1u);
    atomicStore(&visible[u32(params.options.y)+at],slot);
}
