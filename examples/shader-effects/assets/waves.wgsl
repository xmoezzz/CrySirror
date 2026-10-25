// SPDX-License-Identifier: MPL-2.0
// Developer-authored surface hook, ABI v1. Inputs are linear colors.
fn crysirror_surface(i: SurfaceInput) -> SurfaceOutput {
    var o = surface_default(i);
    let wave = 0.5 + 0.5*sin(i.uv.y*24.0 + i.uv.x*7.0 - i.time*2.0);
    let tint = mix(i.color.rgb, i.parameters[0].rgb, wave);
    o.color = vec4<f32>(tint, i.color.a);
    o.emission = tint*wave*i.parameters[0].w;
    return o;
}
