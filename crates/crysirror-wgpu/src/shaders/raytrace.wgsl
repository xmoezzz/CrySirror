// SPDX-License-Identifier: MPL-2.0
struct Params { inverse_view_projection: mat4x4<f32>, background: vec4<f32>, size_count: vec4<u32> }
struct Triangle { a:vec4<f32>, b:vec4<f32>, c:vec4<f32>, normal:vec4<f32>, color:vec4<f32> }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage,read> triangles: array<Triangle>;
@group(0) @binding(2) var output: texture_storage_2d<rgba16float,write>;

fn distance_to_triangle(origin:vec3<f32>,direction:vec3<f32>,triangle:Triangle)->f32 {
    let edge1=triangle.b.xyz-triangle.a.xyz;
    let edge2=triangle.c.xyz-triangle.a.xyz;
    let p=cross(direction,edge2);
    let determinant=dot(edge1,p);
    if(abs(determinant)<0.0000001){return 1e30;}
    let inverse=1.0/determinant;
    let t=origin-triangle.a.xyz;
    let u=dot(t,p)*inverse;
    if(u<0.0 || u>1.0){return 1e30;}
    let q=cross(t,edge1);
    let v=dot(direction,q)*inverse;
    if(v<0.0 || u+v>1.0){return 1e30;}
    let distance=dot(edge2,q)*inverse;
    if(distance<0.0001){return 1e30;}
    return distance;
}

@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x>=params.size_count.x || id.y>=params.size_count.y){return;}
    let uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(params.size_count.xy);
    let ndc=vec2<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0);
    let a=params.inverse_view_projection*vec4<f32>(ndc,0.0,1.0);
    let b=params.inverse_view_projection*vec4<f32>(ndc,1.0,1.0);
    let origin=a.xyz/a.w;
    let ray=b.xyz/b.w-origin;
    let direction=normalize(ray);
    var nearest=length(ray);
    var index=params.size_count.z;
    for(var i=0u;i<params.size_count.z;i++){
        let distance=distance_to_triangle(origin,direction,triangles[i]);
        if(distance<nearest){nearest=distance;index=i;}
    }
    var color=params.background;
    if(index<params.size_count.z){
        let triangle=triangles[index];
        let normal=normalize(triangle.normal.xyz);
        let light=normalize(vec3<f32>(0.45,0.85,0.3));
        let point=origin+direction*nearest;
        var visibility=1.0;
        for(var i=0u;i<params.size_count.z;i++){
            if(i!=index && distance_to_triangle(point+light*0.002,light,triangles[i])<1e20){visibility=0.0;break;}
        }
        let diffuse=max(dot(normal,light),0.0);
        color=vec4<f32>(triangle.color.rgb*(0.18+0.82*diffuse*visibility),1.0);
    }
    textureStore(output,vec2<i32>(id.xy),color);
}
