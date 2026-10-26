// SPDX-License-Identifier: MPL-2.0
// RT geometry gathers from the renderer's own vertex streams (ADR 0055):
// skinned/morphed positions into a dynamic BLAS input, and triangle UVs for
// alpha-tested hits. The 48-byte PbrVertex stream is position(3) normal(3)
// uv(2) tangent(4) floats; skin streams are joints u16x4 + weights f32x4.
struct Params { count:u32, palette_offset:u32, skinned:u32, out_base:u32 }
// Two layouts share bindings 0-1; each stays within 4 storage buffers.
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var<storage,read> source:array<f32>;
// skin_positions
@group(0) @binding(2) var<storage,read> skin:array<u32>;
@group(0) @binding(3) var<storage,read> palette:array<mat4x4<f32>>;
@group(0) @binding(4) var<storage,read_write> positions:array<f32>;
// gather_uvs
@group(0) @binding(5) var<storage,read> indices:array<u32>;
@group(0) @binding(6) var<storage,read_write> uvs:array<vec2<f32>>;

@compute @workgroup_size(64)
fn skin_positions(@builtin(global_invocation_id) id:vec3<u32>){
    let i=id.x;
    if i>=params.count {return;}
    var p=vec3<f32>(source[i*12u],source[i*12u+1u],source[i*12u+2u]);
    if params.skinned==1u {
        let packed0=skin[i*6u];
        let packed1=skin[i*6u+1u];
        let joints=vec4<u32>(packed0&0xffffu,packed0>>16u,packed1&0xffffu,packed1>>16u);
        let weights=vec4<f32>(bitcast<f32>(skin[i*6u+2u]),bitcast<f32>(skin[i*6u+3u]),bitcast<f32>(skin[i*6u+4u]),bitcast<f32>(skin[i*6u+5u]));
        let b=params.palette_offset;
        let k=palette[b+joints.x]*weights.x+palette[b+joints.y]*weights.y+palette[b+joints.z]*weights.z+palette[b+joints.w]*weights.w;
        p=(k*vec4<f32>(p,1.0)).xyz;
    }
    positions[i*3u]=p.x; positions[i*3u+1u]=p.y; positions[i*3u+2u]=p.z;
}

@compute @workgroup_size(64)
fn gather_uvs(@builtin(global_invocation_id) id:vec3<u32>){
    let corner=id.x;
    if corner>=params.count {return;}
    let v=indices[corner];
    uvs[params.out_base+corner]=vec2<f32>(source[v*12u+6u],source[v*12u+7u]);
}
