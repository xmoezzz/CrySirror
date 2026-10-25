// SPDX-License-Identifier: MPL-2.0
fn effect_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}
fn effect_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f*f*(vec2<f32>(3.0)-2.0*f);
    return mix(mix(effect_hash(cell), effect_hash(cell+vec2<f32>(1.0,0.0)),u.x),
               mix(effect_hash(cell+vec2<f32>(0.0,1.0)), effect_hash(cell+vec2<f32>(1.0)),u.x),u.y);
}
fn crysirror_surface(i: SurfaceInput) -> SurfaceOutput {
    var o = surface_default(i);
    let p = i.parameters;
    if material.surface.y == 1.0 {
        let q = i.uv*p[0].y + i.time*p[1].xy;
        let n = 0.65*effect_noise(q)+0.35*effect_noise(q*2.03+vec2<f32>(17.0));
        let boundary = min(min(i.uv.x,1.0-i.uv.x),min(i.uv.y,1.0-i.uv.y));
        let edge = select(1.0,smoothstep(0.0,max(p[0].z,0.00001),boundary),p[0].z>0.0);
        o.color.a *= (1.0-exp(-p[0].x*n))*edge;
    } else if material.surface.y == 2.0 {
        let f = pow(1.0-clamp(abs(dot(i.world_normal,i.view_direction)),0.0,1.0),p[0].x)*p[0].y;
        o.emission += i.color.rgb*f;
    } else if material.surface.y == 3.0 {
        // World-space noise works for both authored UVs and legacy geometry.
        let n = effect_noise(i.world_position.xz*3.1+i.world_position.yy);
        let kept = select(0.0,1.0,n>=p[0].x);
        o.color.a *= kept;
        o.emission += p[1].rgb*(1.0-smoothstep(p[0].x,p[0].x+p[0].y,n))*kept;
    }
    return o;
}
