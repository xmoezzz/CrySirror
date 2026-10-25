// SPDX-License-Identifier: MPL-2.0
// Shared ordered Bayer fade. Threshold/soft-width profile agrees with
// siglus_rs crates/reallive/src/effects.rs @
// 5f358ef1360b0439e92aed2b749e8f25d8721954, styles 4/5. No VM or native
// framebuffer is used: inputs are the engine renderer's retained GPU scenes.
struct WipeUniform {
    kind_progress: vec4<f32>,
    option0: vec4<f32>,
    option1: vec4<f32>,
    option2: vec4<f32>,
    option3: vec4<f32>,
};
@group(0) @binding(2) var current_tex: texture_2d<f32>;
@group(0) @binding(4) var old_tex: texture_2d<f32>;
@group(0) @binding(8) var<uniform> wipe: WipeUniform;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p=vec2<f32>(f32((index << 1u) & 2u),f32(index & 2u));
    return vec4<f32>(p*2.0-1.0,0.0,1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at=vec2<i32>(position.xy);
    let current=textureLoad(current_tex,at,0);
    let progress=clamp(wipe.kind_progress.y,0.0,1.0);
    if progress>=1.0 {return current;}
    let old_size=vec2<i32>(textureDimensions(old_tex));
    let old=textureLoad(old_tex,clamp(at,vec2<i32>(0),old_size-vec2<i32>(1)),0);
    let render_size=vec2<f32>(textureDimensions(current_tex));
    let canvas=max(wipe.kind_progress.zw,vec2<f32>(1.0));
    let scale=min(render_size.x/canvas.x,render_size.y/canvas.y);
    let margin=(render_size-canvas*scale)*0.5;
    let pixel=vec2<u32>(max(floor((position.xy-margin)/scale),vec2<f32>(0.0)));
    let cell=max(vec2<u32>(wipe.option0.xy),vec2<u32>(1));
    let base=pixel/cell;
    var i=base.x;
    var j=base.y;
    if wipe.option0.z>0.5 {
        i=base.x+base.y;
        j=base.x+3u*base.y;
    }
    var bayer=array<u32,16>(0u,8u,2u,10u,12u,4u,14u,6u,3u,11u,1u,9u,15u,7u,13u,5u);
    let threshold=(f32(bayer[(j%4u)*4u+i%4u])/16.0)*(15.0/16.0)+0.5/16.0;
    let width=max(wipe.option0.w,1.0)/16.0;
    let amount=clamp((progress-threshold)/width+0.5,0.0,1.0);
    // Byte-space nearest rounding is explicit, independently of target UNORM
    // rounding. Sampling raw views preserves gamma-encoded scene values.
    return floor(mix(old,current,amount)*255.0+vec4<f32>(0.5))/255.0;
}
