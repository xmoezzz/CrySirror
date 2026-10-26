// SPDX-License-Identifier: MPL-2.0
// Ray-traced irradiance probe update (ADR 0059), appended to rt_common.wgsl.
// It writes the same data as the raster capture + gi_project.wgsl path
// (ADR 0028): per probe L1 SH of irradiance (r, g, b, state) and 8×8
// octahedral distance moments for Chebyshev visibility. One workgroup per
// probe, one invocation per octahedral texel, RAYS jittered rays each.
// Hits are lit by the sun (ray-traced shadow), their emission and the
// volume's current irradiance (multi-bounce); misses see the environment.
struct Params {
    // xyz: direction light travels, w: illuminance
    sun_dir:vec4<f32>,
    // rgb: linear colour, w: 1 with shadows
    sun_color:vec4<f32>,
    // x: IBL intensity, y: environment rotation, z: hysteresis, w: frame
    env:vec4<f32>,
    // Volume: centre + intensity, half extent, probe counts.
    gi_center:vec4<f32>,
    gi_extent:vec4<f32>,
    gi_counts:vec4<f32>,
    // x: first vec4 of the GI section, y: of the visibility section, z: jobs
    offsets:vec4<u32>,
    // x distance clamp (misses count as this far), y back-face fraction
    // above which a probe is disabled
    limits:vec4<f32>,
    // Probe index per job, four per vec4 (at most MAX_JOBS).
    jobs:array<vec4<u32>,16>,
}
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(2) var environment:texture_2d<f32>;
@group(0) @binding(3) var environment_sampler:sampler;
@group(0) @binding(5) var<storage,read_write> data:array<vec4<f32>>;

const PI:f32=3.14159265359;
const OCT:u32=8u;
const RAYS:u32=4u;
var<workgroup> acc:array<array<vec4<f32>,3>,64>;
var<workgroup> wacc:array<vec2<f32>,64>;
var<workgroup> moments:array<vec2<f32>,64>;

fn rotate(d:vec3<f32>,a:f32)->vec3<f32> {
    let c=cos(a); let s=sin(a);
    return vec3<f32>(c*d.x+s*d.z,d.y,-s*d.x+c*d.z);
}
fn sky(d:vec3<f32>)->vec3<f32> {
    let r=rotate(normalize(d),-params.env.y);
    let uv=vec2<f32>(atan2(r.z,r.x)/(2.0*PI)+0.5,acos(clamp(r.y,-1.0,1.0))/PI);
    return textureSampleLevel(environment,environment_sampler,uv,0.0).rgb*params.env.x;
}
fn probe_position(i:u32)->vec3<f32> {
    let c=vec3<u32>(params.gi_counts.xyz);
    let idx=vec3<u32>(i%c.x,(i/c.x)%c.y,i/(c.x*c.y));
    let cells=vec3<f32>(c-vec3<u32>(1u));
    return params.gi_center.xyz-params.gi_extent.xyz+vec3<f32>(idx)/cells*2.0*params.gi_extent.xyz;
}
// Volume irradiance at a point (trilinear, without visibility).
fn volume_irradiance(p:vec3<f32>,n:vec3<f32>)->vec3<f32> {
    let ext=params.gi_extent.xyz;
    let cnt=vec3<u32>(params.gi_counts.xyz);
    let cells=vec3<f32>(cnt-vec3<u32>(1u));
    let t=(clamp(p-params.gi_center.xyz,-ext,ext)+ext)/(2.0*ext)*cells;
    let b=vec3<u32>(clamp(floor(t),vec3<f32>(0.0),cells-vec3<f32>(1.0)));
    let f=t-vec3<f32>(b);
    var sum=vec3<f32>(0.0);
    var wsum=0.0;
    for (var c=0u; c<8u; c++) {
        let o=vec3<u32>(c&1u,(c>>1u)&1u,(c>>2u)&1u);
        let idx=b+o;
        let i=idx.x+cnt.x*(idx.y+cnt.y*idx.z);
        let base=params.offsets.x+i*4u;
        if data[base+3u].x<0.5 {continue;}
        let tri=select(1.0-f,f,o==vec3<u32>(1u));
        let w=tri.x*tri.y*tri.z;
        let nn=vec4<f32>(1.0,n);
        sum+=max(vec3<f32>(dot(data[base],nn),dot(data[base+1u],nn),dot(data[base+2u],nn)),vec3<f32>(0.0))*w;
        wsum+=w;
    }
    if wsum<=1e-6 {return vec3<f32>(0.0);}
    return sum/wsum*params.gi_center.w;
}
fn oct_vector(e:vec2<f32>)->vec3<f32> {
    var v=vec3<f32>(e,1.0-abs(e.x)-abs(e.y));
    if v.z<0.0 {
        v=vec3<f32>((1.0-abs(v.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),v.xy>=vec2<f32>(0.0)),v.z);
    }
    return v;
}

@compute @workgroup_size(64)
fn update(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32){
    if group.x>=params.offsets.z {return;}
    let probe=params.jobs[group.x/4u][group.x%4u];
    let origin=probe_position(probe);
    let tx=li%OCT;
    let ty=li/OCT;
    var r=vec4<f32>(0.0); var g=vec4<f32>(0.0); var b=vec4<f32>(0.0);
    var w=0.0; var back=0.0;
    var m=vec2<f32>(0.0);
    for (var k=0u; k<RAYS; k++) {
        let u=random2(vec2<u32>(probe,li),u32(params.env.w),k);
        let e=(vec2<f32>(f32(tx),f32(ty))+u)/f32(OCT)*2.0-1.0;
        let v=oct_vector(e);
        let len2=dot(v,v);
        let dir=v*inverseSqrt(len2);
        // Solid angle of an octahedral sample ∝ 1/|v|³ (as for cube texels).
        let dw=1.0/(len2*sqrt(len2));
        var radiance=vec3<f32>(0.0);
        var distance=params.limits.x;
        let hit=closest(origin,dir,1e20);
        if hit.kind==1u {
            distance=min(hit.t,params.limits.x);
            let inst=instances[hit.instance_custom_data].data;
            let n=normalize(hit.object_to_world*vec4<f32>(corner_normal(inst.x+hit.primitive_index*3u,hit.barycentrics),0.0));
            // Back faces by the shading normal: independent of each backend's
            // triangle winding convention for `front_face`.
            if dot(n,dir)<0.0 {
                let point=origin+dir*hit.t;
                let albedo=hit_material(hit).rgb;
                var emissive=vec3<f32>(0.0);
                if inst.y>0u {emissive=slots[inst.y-1u].emissive.rgb;}
                let l=-params.sun_dir.xyz;
                let nl=max(dot(n,l),0.0);
                var sun=0.0;
                if nl>0.0 {
                    sun=nl;
                    if params.sun_color.w>0.5 {sun*=visible(point+n*0.002,l,1e20);}
                }
                let irradiance=params.sun_color.rgb*params.sun_dir.w*sun+volume_irradiance(point+n*0.05,n);
                radiance=albedo/PI*irradiance+emissive;
            } else {
                back+=dw;
            }
        } else {
            radiance=sky(dir);
        }
        let basis=vec4<f32>(0.282095,0.488603*dir);
        r+=basis*radiance.r*dw; g+=basis*radiance.g*dw; b+=basis*radiance.b*dw;
        w+=dw;
        m+=vec2<f32>(distance,distance*distance);
    }
    acc[li]=array<vec4<f32>,3>(r,g,b);
    wacc[li]=vec2<f32>(w,back);
    moments[li]=m/f32(RAYS);
    workgroupBarrier();
    let base=params.offsets.x+probe*4u;
    let fresh=data[base+3u].z<0.5;
    let keep=select(params.env.z,0.0,fresh);
    if li<32u {
        let at=params.offsets.y+probe*32u+li;
        let next=vec4<f32>(moments[2u*li],moments[2u*li+1u]);
        data[at]=mix(next,data[at],keep);
    }
    if li==0u {
        var sum=array<vec4<f32>,3>(vec4<f32>(0.0),vec4<f32>(0.0),vec4<f32>(0.0));
        var ws=vec2<f32>(0.0);
        for (var t=0u; t<64u; t++) {
            sum[0]+=acc[t][0]; sum[1]+=acc[t][1]; sum[2]+=acc[t][2];
            ws+=wacc[t];
        }
        // Normalise the weights to 4π, then convolve: A0 = π, A1 = 2π/3.
        let norm=4.0*PI/ws.x;
        let conv=vec4<f32>(PI*0.282095,vec3<f32>(2.0*PI/3.0*0.488603));
        let back_fraction=ws.y/ws.x;
        for (var c=0u; c<3u; c++) {
            data[base+c]=mix(sum[c]*norm*conv,data[base+c],keep);
        }
        // x: usable, y: back-face fraction, z: refreshed at least once.
        data[base+3u]=vec4<f32>(select(1.0,0.0,back_fraction>params.limits.y),back_fraction,1.0,0.0);
    }
}
