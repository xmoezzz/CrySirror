// SPDX-License-Identifier: MPL-2.0
// Independently implemented fixed-tick VFX kernels. No engine/device pointers
// or executable shader data are part of logical project state.
struct Particle {
    position_age:vec4<f32>, velocity_lifetime:vec4<f32>,
    previous_position_rotation:vec4<f32>, color:vec4<f32>, size:vec4<f32>, metadata:vec4<u32>,
}
struct Event {identity:vec4<u32>,position:vec4<f32>,velocity:vec4<f32>}
struct Control {draw:array<atomic<u32>,4>,stats:array<atomic<u32>,8>,events:array<Event,1024>}
struct Params {
    transform:mat4x4<f32>,eye:vec4<f32>,step:vec4<f32>,counts:vec4<u32>,
    flags:vec4<u32>,tick_sort:vec4<u32>,limits:vec4<u32>,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read_write> particles:array<Particle>;
@group(0) @binding(2) var<storage,read> program:array<vec4<u32>>;
@group(0) @binding(3) var<storage,read_write> indices:array<u32>;
@group(0) @binding(4) var<storage,read_write> control:Control;
fn word(i:u32)->vec4<f32>{return bitcast<vec4<f32>>(program[i]);}
fn hash(seed:u32,id:vec2<u32>,dimension:u32)->u32 {
    var x=seed^(id.x*747796405u)^(id.y*2891336453u)^(dimension*277803737u);
    x=((x>>((x>>28u)+4u))^x)*277803737u;
    return (x>>22u)^x;
}
fn random(id:vec2<u32>,dimension:u32)->f32 {return f32(hash(p.flags.x,id,dimension)>>8u)/16777216.0;}
fn emit(kind:u32,q:Particle){
    let i=atomicAdd(&control.stats[0],1u);
    if i<p.limits.y {
        control.events[i]=Event(vec4<u32>(q.metadata.xy,kind,p.tick_sort.x),vec4<f32>(q.position_age.xyz,q.position_age.w),q.velocity_lifetime);
    }else{atomicAdd(&control.stats[1],1u);}
}
fn safe_normal(v:vec3<f32>,fallback:vec3<f32>)->vec3<f32>{let d=dot(v,v);return select(fallback,v*inverseSqrt(max(d,1e-20)),d>1e-16);}
// Shortest-arc rotation from local +Y, including the antiparallel case.
fn emission_axis(v:vec3<f32>,axis:vec3<f32>)->vec3<f32>{
    let n=safe_normal(axis,vec3<f32>(0.0,1.0,0.0));
    if n.y > 0.9999997615814209 {return v;}
    if n.y < -0.9999997615814209 {return vec3<f32>(-v.x,-v.y,v.z);}
    let c=cross(vec3<f32>(0.0,1.0,0.0),n);
    return v+cross(c,v)+cross(c,cross(c,v))/max(1.0+n.y,1e-8);
}
fn initialize(id:vec2<u32>)->Particle {
    let ranges=word(0u);let direction=word(1u);let sizes=word(3u);
    let a=random(id,4u);let b=random(id,5u);let c=random(id,6u);
    let shape=program[4].x;let cfg=word(4u);let ext=word(5u).xyz;
    var pos=vec3<f32>(0.0);
    if shape==1u {
        let y=1.0-2.0*a;let phi=6.28318530718*b;
        let radius=cfg.y*select(pow(c,1.0/3.0),1.0,cfg.w>0.5);
        let r=sqrt(max(1.0-y*y,0.0));pos=vec3<f32>(r*cos(phi),y,r*sin(phi))*radius;
    }else if shape==2u {pos=(vec3<f32>(a,b,c)*2.0-1.0)*ext;}
    else if shape==3u {
        let y=cfg.z*pow(a,1.0/3.0);let scale=select(1.0,y/max(cfg.z,1e-10),cfg.z>0.0);
        let r=cfg.y*scale*sqrt(b);let phi=6.28318530718*c;pos=vec3<f32>(r*cos(phi),y,r*sin(phi));
    }else if shape==4u {pos=ext*a;}
    let cosine=1.0-random(id,2u)*(1.0-cos(direction.w));let sine=sqrt(max(1.0-cosine*cosine,0.0));
    let phi=6.28318530718*random(id,3u);
    var velocity=emission_axis(vec3<f32>(sine*cos(phi),cosine,sine*sin(phi)),direction.xyz);
    if p.flags.y==0u {pos=(p.transform*vec4<f32>(pos,1.0)).xyz;velocity=safe_normal((p.transform*vec4<f32>(velocity,0.0)).xyz,vec3<f32>(0.0,1.0,0.0));}
    velocity*=mix(ranges.z,ranges.w,random(id,1u));
    let life=mix(ranges.x,ranges.y,random(id,0u));let rotation=mix(sizes.z,sizes.w,random(id,7u));
    return Particle(vec4<f32>(pos,0.0),vec4<f32>(velocity,life),vec4<f32>(pos,rotation),word(2u),vec4<f32>(sizes.xy,0.0,0.0),vec4<u32>(id,program[5].w,1u));
}
@compute @workgroup_size(1)
fn clear(){
    atomicStore(&control.draw[0],p.flags.w);atomicStore(&control.draw[1],0u);atomicStore(&control.draw[2],0u);atomicStore(&control.draw[3],0u);
    for(var i=0u;i<8u;i++){atomicStore(&control.stats[i],0u);}
    atomicStore(&control.stats[2],p.counts.y);
}
// Deterministic lowest-free-slot allocation with a two-level scan. Only the
// small block-total scan is serial (<=1563 blocks at the 200k slot budget).
// Scratch follows the draw-index prefix, retaining four storage bindings.
var<workgroup> free_scan:array<u32,128>;
@compute @workgroup_size(128)
fn scan_free(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(local_invocation_index) lane:u32,@builtin(workgroup_id) group:vec3<u32>){
    let i=gid.x;var available=0u;
    if i<p.counts.x {available=select(0u,1u,particles[i].metadata.w==0u);}
    free_scan[lane]=available;workgroupBarrier();
    for(var offset=1u;offset<128u;offset*=2u){
        var previous=0u;if lane>=offset {previous=free_scan[lane-offset];}
        let value=free_scan[lane]+previous;workgroupBarrier();free_scan[lane]=value;workgroupBarrier();
    }
    if i<p.counts.x {indices[p.limits.x+i]=select(0xffffffffu,free_scan[lane]-1u,available==1u);}
    if lane==127u {indices[p.limits.x+p.counts.x+group.x]=free_scan[127u];}
}
@compute @workgroup_size(1)
fn prefix_free(){
    var free=0u;let at=p.limits.x+p.counts.x;
    for(var block=0u;block<p.limits.w;block++){let count=indices[at+block];indices[at+block]=free;free+=count;}
    let births=min(free,p.counts.y);
    atomicStore(&control.stats[0],births);atomicStore(&control.stats[1],births-min(births,p.limits.y));
    atomicStore(&control.stats[3],p.counts.y-births);atomicStore(&control.stats[6],births);
}
@compute @workgroup_size(64)
fn spawn(@builtin(global_invocation_id) gid:vec3<u32>){
    let i=gid.x;if i>=p.counts.x {return;}let local_rank=indices[p.limits.x+i];
    if local_rank==0xffffffffu{return;}
    let birth=indices[p.limits.x+p.counts.x+i/128u]+local_rank;
    if birth>=p.counts.y {return;}
    let lo=p.counts.z+birth;let carry=u32(lo<p.counts.z);let id=vec2<u32>(lo,p.counts.w+carry);
    let q=initialize(id);particles[i]=q;
    if birth<p.limits.y {control.events[birth]=Event(vec4<u32>(id,0u,p.tick_sort.x),vec4<f32>(q.position_age.xyz,q.position_age.w),q.velocity_lifetime);}
}
fn interpolation(t:f32,mode:u32)->f32 {
    if mode==0u {return select(0.0,1.0,t>=1.0);}if mode==2u{return t*t*(3.0-2.0*t);}return t;
}
fn curve_color(offset:u32,count:u32,mode:u32,t:f32)->vec4<f32>{
    var i=0u;for(var j=1u;j<count-1u;j++){if word(offset+j*2u).x<=t{i=j;}}
    let a=word(offset+i*2u);let b=word(offset+(i+1u)*2u);
    let f=interpolation(clamp((t-a.x)/(b.x-a.x),0.0,1.0),mode);
    return mix(vec4<f32>(a.yzw,word(offset+i*2u+1u).x),vec4<f32>(b.yzw,word(offset+(i+1u)*2u+1u).x),f);
}
fn curve_size(offset:u32,count:u32,mode:u32,t:f32)->vec2<f32>{
    var i=0u;for(var j=1u;j<count-1u;j++){if word(offset+j).x<=t{i=j;}}
    let a=word(offset+i);let b=word(offset+i+1u);let f=interpolation(clamp((t-a.x)/(b.x-a.x),0.0,1.0),mode);return mix(a.yz,b.yz,f);
}
@compute @workgroup_size(64)
fn update(@builtin(global_invocation_id) gid:vec3<u32>){
    let i=gid.x;if i>=p.counts.x{return;}var q=particles[i];if q.metadata.w==0u{return;}
    let dt=p.step.x;if dt==0.0{return;}q.previous_position_rotation=vec4<f32>(q.position_age.xyz,q.previous_position_rotation.w);
    var pos=q.position_age.xyz;var velocity=q.velocity_lifetime.xyz;
    for(var j=0u;j<p.flags.z;j++) {
        let at=8u+j*4u;let op=program[at].x;let a=word(at).yzw;let b=word(at+1u);
        if op==1u {velocity+=a*dt;}
        else if op==2u {velocity*=exp(-a.x*dt);}
        else if op==3u {
            // Analytic divergence-free curl, same phase and time as CPU oracle.
            let phase=random(q.metadata.xy,8u)*6.28318530718;
            let curl=-vec3<f32>(cos(pos.z*a.y+p.step.y*1.37+phase),cos(pos.x*a.y+p.step.y*0.73+phase),cos(pos.y*a.y+p.step.y+phase));
            velocity+=curl*(a.y*a.x*dt);
        }else if op==4u {
            let to=a-pos;let distance=length(to);
            if distance>1e-6&&distance<b.y {let fade=1.0-distance/b.y;velocity+=(to/distance)*(b.x*fade*fade*dt);}
        }else if op==5u {
            let axis=safe_normal(a,vec3<f32>(0.0,1.0,0.0));let angle=b.x*dt;let c=cos(angle);let s=sin(angle);
            let origin=select(p.transform[3].xyz,vec3<f32>(0.0),p.flags.y!=0u);
            let delta=pos-origin;
            pos=origin+delta*c+cross(axis,delta)*s+axis*dot(axis,delta)*(1.0-c);
            velocity=velocity*c+cross(axis,velocity)*s+axis*dot(axis,velocity)*(1.0-c);
        }else if op==6u {q.previous_position_rotation.w+=a.x*dt;}
    }
    pos+=velocity*dt;var killed=false;var collided=false;
    for(var j=0u;j<p.flags.z;j++) {
        let at=8u+j*4u;let op=program[at].x;let a=word(at).yzw;let b=word(at+1u);
        var collision=false;
        if op==9u {
            let n=safe_normal(a,vec3<f32>(0.0,1.0,0.0));let distance=dot(pos,n)-b.x;
            if distance<0.0 {
                pos-=n*distance;let vn=dot(velocity,n);
                if vn<0.0 {velocity=(velocity-n*vn)*(1.0-b.z)-n*vn*b.y;}
                collision=true;killed=killed||b.w>0.5;
            }
        }else if op==10u {
            let delta=pos-a;let distance=length(delta);
            if distance<b.x {
                let n=safe_normal(delta,vec3<f32>(0.0,1.0,0.0));pos=a+n*b.x;
                let vn=dot(velocity,n);if vn<0.0 {velocity-=n*vn*(1.0+b.y);}
                collision=true;killed=killed||b.z>0.5;
            }
        }
        collided=collided||collision;
    }
    q.position_age=vec4<f32>(pos,q.position_age.w+dt);q.velocity_lifetime=vec4<f32>(velocity,q.velocity_lifetime.w);
    let normalized_age=clamp(q.position_age.w/q.velocity_lifetime.w,0.0,1.0);
    for(var j=0u;j<p.flags.z;j++){
        let instruction=program[8u+j*4u];
        if instruction.x==7u{q.color=curve_color(instruction.y,instruction.z,instruction.w,normalized_age);}
        else if instruction.x==8u{q.size=vec4<f32>(curve_size(instruction.y,instruction.z,instruction.w,normalized_age),0.0,0.0);}
    }
    if collided {emit(2u,q);atomicAdd(&control.stats[5],1u);}
    if killed||q.position_age.w>=q.velocity_lifetime.w {q.metadata.w=0u;emit(1u,q);atomicAdd(&control.stats[4],1u);}
    particles[i]=q;
}
@compute @workgroup_size(64)
fn compact(@builtin(global_invocation_id) gid:vec3<u32>){
    let i=gid.x;if i>=p.limits.x{return;}
    // Indices use a stable slot layout when sorting, with dead sentinels; the
    // unsorted draw needs a densely packed prefix and uses atomic compaction.
    if p.limits.z!=0u {
        indices[i]=0xffffffffu;
        if i<p.counts.x&&particles[i].metadata.w!=0u {indices[i]=i;atomicAdd(&control.draw[1],1u);}
    }else if i<p.counts.x&&particles[i].metadata.w!=0u {let j=atomicAdd(&control.draw[1],1u);indices[j]=i;}
}
fn farther(a:u32,b:u32)->bool {
    if a==0xffffffffu{return false;}if b==0xffffffffu{return true;}
    var pa=particles[a].position_age.xyz;var pb=particles[b].position_age.xyz;
    if p.flags.y!=0u {pa=(p.transform*vec4<f32>(pa,1.0)).xyz;pb=(p.transform*vec4<f32>(pb,1.0)).xyz;}
    let da=dot(pa-p.eye.xyz,pa-p.eye.xyz);let db=dot(pb-p.eye.xyz,pb-p.eye.xyz);
    return da>db||(da==db&&a<b);
}
@compute @workgroup_size(64)
fn sort_indices(@builtin(global_invocation_id) gid:vec3<u32>){
    let i=gid.x;if i>=p.limits.x{return;}let j=i^p.tick_sort.z;if j<=i||j>=p.limits.x{return;}
    let a=indices[i];let b=indices[j];let descending=(i&p.tick_sort.w)==0u;
    if farther(b,a)==descending {indices[i]=b;indices[j]=a;}
}
