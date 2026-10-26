// SPDX-License-Identifier: MPL-2.0
// Shared hit-lighting API over the renderer's original light buffer. No new
// engine-private light representation or camera-cluster culling at bounce hits.
struct BounceLight {
    position_range:vec4<f32>, direction_kind:vec4<f32>, intensity:vec4<f32>,
    cone:vec4<f32>, right:vec4<f32>, up:vec4<f32>,
}
@group(0) @binding(12) var<storage,read> bounce_lights:array<BounceLight>;
fn bounce_local(point:vec3<f32>,n:vec3<f32>,count:u32)->vec3<f32> {
    var sum=vec3<f32>(0.0);
    for(var k=0u;k<count;k++) {
        let li=bounce_lights[k];
        let kind=li.direction_kind.w;
        let range=li.position_range.w;
        var p=li.position_range.xyz;
        let delta=point-p;
        let center2=dot(delta,delta);
        if center2>=range*range {continue;}
        let exponent=li.cone.z;
        let q=center2/(range*range);
        if kind>2.5 {
            var shape=pow(clamp(1.0-q*q,0.0,1.0),2.0);
            if exponent>0.0 {shape=pow(clamp(1.0-q,0.0,1.0),exponent);}
            let cosine=select(max(dot(n,-li.direction_kind.xyz),0.0),1.0,kind>3.5);
            sum+=li.intensity.rgb*(shape*cosine);
            continue;
        }
        if kind>1.5 {
            let hx=max(length(li.right.xyz),1e-5); let hy=max(length(li.up.xyz),1e-5);
            let ax=li.right.xyz/hx; let ay=li.up.xyz/hy;
            p+=ax*clamp(dot(delta,ax),-hx,hx)+ay*clamp(dot(delta,ay),-hy,hy);
        }
        let to=p-point;
        let d2=max(dot(to,to),1e-4);
        let distance=sqrt(d2);
        let l=to/distance;
        var emit=1.0;
        if kind>1.5 {emit=max(dot(li.direction_kind.xyz,-l),0.0);}
        else if kind>0.5 {let c=clamp((dot(li.direction_kind.xyz,-l)-li.cone.x)*li.cone.y,0.0,1.0);emit=c*c;}
        let cosine=max(dot(n,l),0.0);
        if cosine<=0.0 || emit<=0.0 {continue;}
        let r=d2/(range*range);
        var attenuation=pow(clamp(1.0-r*r,0.0,1.0),2.0)/d2;
        if exponent>0.0 {attenuation=pow(clamp(1.0-r,0.0,1.0),exponent);}
        let visibility=visible(point+n*0.002,l,max(distance-0.004,0.0001));
        sum+=li.intensity.rgb*(emit*cosine*attenuation*visibility);
    }
    return sum;
}
