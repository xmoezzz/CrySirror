// Bounded, separable box filter for engine-owned transition targets.
// The 64-pixel tile and its halo are loaded once from the input texture.
@group(0) @binding(0) var source: texture_2d<f32>;
struct Params { config:vec4<u32>, rect:vec4<f32>, extra:vec4<u32> }
@group(0) @binding(1) var<uniform> params:Params;
@group(0) @binding(2) var output:texture_storage_2d<__OUTPUT_FORMAT__,write>;
var<workgroup> tile:array<vec4<f32>,320>;
fn gamma(v:vec3<f32>)->vec3<f32> {
    let x=clamp(v,vec3<f32>(0.),vec3<f32>(1.));
    return select(1.055*pow(x,vec3<f32>(1./2.4))-0.055,x*12.92,x<=vec3<f32>(0.0031308));
}
fn input_value(pixel:vec2<i32>)->vec4<f32> {
    let size=vec2<i32>(textureDimensions(source));
    let mode=params.config.w;
    if (mode==2u) {
        let uv=(vec2<f32>(pixel)-params.rect.xy)/params.rect.zw;
        let p=vec2<i32>(clamp(floor(uv*vec2<f32>(size)),vec2<f32>(0.),vec2<f32>(size)-1.));
        let rgb=floor(gamma(textureLoad(source,p,0).rgb)*255.+0.5);
        let gray=floor(dot(rgb,vec3<f32>(54.,183.,19.))/256.);
        return vec4<f32>(vec3<f32>(gray),255.);
    }
    let lo=vec2<i32>(clamp(floor(params.rect.xy),vec2<f32>(0.),vec2<f32>(size)-1.));
    let hi=max(lo,vec2<i32>(clamp(ceil(params.rect.xy+params.rect.zw)-1.,vec2<f32>(0.),vec2<f32>(size)-1.)));
    let value=textureLoad(source,clamp(pixel,lo,hi),0);
    if (mode==1u) {
        return floor(vec4<f32>(gamma(value.rgb/max(value.a,0.000001))*value.a,value.a)*255.+0.5);
    }
    return value;
}
@compute @workgroup_size(64)
fn cs_main(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let axis=params.config.x;
    let radius=params.config.y;
    let size=textureDimensions(output);
    let begin=select(group.x,group.y,axis==1u)*64u;
    let fixed=select(group.y,group.x,axis==1u);
    for (var n=lane;n<64u+2u*radius;n+=64u) {
        let moving=i32(begin+n)-i32(radius);
        let pixel=select(vec2<i32>(moving,i32(fixed)),vec2<i32>(i32(fixed),moving),axis==1u);
        tile[n]=input_value(pixel);
    }
    workgroupBarrier();
    let moving=begin+lane;
    let pixel=select(vec2<u32>(moving,fixed),vec2<u32>(fixed,moving),axis==1u);
    if (any(pixel>=size)) { return; }
    let center=vec2<f32>(pixel)+0.5;
    if (any(center<params.rect.xy)||any(center>=params.rect.xy+params.rect.zw)) {
        textureStore(output,vec2<i32>(pixel),vec4<f32>(0.));return;
    }
    if(axis==0u) {
        var sum=vec4<f32>(0.);
        for(var n=0u;n<2u*radius+1u;n+=1u) {sum+=tile[lane+n];}
        textureStore(output,vec2<i32>(pixel),sum);return;
    }
    // At radius 128 the full 2D sum exceeds f32's exact integer range.
    // Horizontal sums are exact <=65535; finish in bounded u32 arithmetic.
    var total=vec4<u32>(0u);
    for(var n=0u;n<2u*radius+1u;n+=1u) {total+=vec4<u32>(tile[lane+n]);}
    let area=(2u*radius+1u)*(2u*params.config.z+1u);
    var average=vec4<f32>(total/vec4<u32>(area))/255.;
    if(params.extra.x!=0u) {average=vec4<f32>(vec3<f32>(1.-average.r),1.);}
    textureStore(output,vec2<i32>(pixel),average);
}
