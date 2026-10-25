// SPDX-License-Identifier: MPL-2.0
// Environment background, drawn at the far plane after opaque geometry.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - vec2<f32>(1.0);
    var o: SkyOut;
    o.clip = vec4<f32>(p, 1.0, 1.0);
    o.ndc = p;
    return o;
}
@fragment
fn fs_main(i: SkyOut) -> @location(0) vec4<f32> {
    // inv_view_proj is unjittered: the sky does not shimmer under TAA.
    let far = frame.inv_view_proj * vec4<f32>(i.ndc, 1.0, 1.0);
    let dir = far.xyz / far.w - frame.camera_pos.xyz;
    let c = textureSampleLevel(env_source, env_sampler, equirect_uv(dir), 0.0).rgb;
    // Fog toward the far plane (capped by max opacity, like UE's sky fog).
    let fog = height_fog(frame.camera_pos.xyz + normalize(dir) * frame.clusters.y);
    // Irradiance captures read alpha as hit distance; 0 marks a miss.
    return vec4<f32>(mix(c * frame.env.x, fog.rgb, fog.a), select(1.0, 0.0, frame.misc.w > 0.5));
}
