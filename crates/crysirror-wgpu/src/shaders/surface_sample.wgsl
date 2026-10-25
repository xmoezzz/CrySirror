// SPDX-License-Identifier: MPL-2.0
// Same coverage calculation in color, depth and shadow passes.
fn sample_surface(uv: vec2<f32>, world: vec3<f32>, normal: vec3<f32>, camera: vec3<f32>, time: f32) -> SurfaceOutput {
    let base = textureSample(t_base,s_material,uv)*material.base_color;
    let opacity = opacity_channel(textureSample(t_opacity,s_material,uv));
    let mr = textureSample(t_mr,s_material,uv);
    let emission = textureSample(t_emissive,s_material,uv).rgb*material.emissive.rgb*material.emissive.a;
    let direction = camera-world;
    let view_dir = direction/max(length(direction),0.00001);
    let n = normal/max(length(normal),0.00001);
    var i: SurfaceInput;
    i.uv=uv; i.world_position=world; i.world_normal=n; i.view_direction=view_dir;
    i.time=time; i.color=vec4<f32>(base.rgb,base.a*opacity); i.emission=emission;
    i.roughness=mr.g*material.params.y; i.metallic=mr.b*material.params.x;
    i.parameters=material.effect_params;
    var o=crysirror_surface(i);
    o.color=vec4<f32>(clamp(o.color.rgb,vec3<f32>(0.0),vec3<f32>(65504.0)),clamp(o.color.a,0.0,1.0));
    o.emission=clamp(o.emission,vec3<f32>(0.0),vec3<f32>(65504.0));
    o.roughness=clamp(o.roughness,0.045,1.0); o.metallic=clamp(o.metallic,0.0,1.0);
    return o;
}
