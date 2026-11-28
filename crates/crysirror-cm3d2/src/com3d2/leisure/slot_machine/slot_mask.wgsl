// SPDX-License-Identifier: MPL-2.0
// AdditiveMaskUV's DXBC compares V with the two inclusive endpoints of
// each band and discards the fragment only when all three bands exclude it.
fn crysirror_surface(i: SurfaceInput) -> SurfaceOutput {
    var o = surface_default(i);
    let v = 1.0 - i.uv.y;
    let a = i.parameters[0];
    let b = i.parameters[1];
    let inside = (v >= a.x && v <= a.y)
        || (v >= a.z && v <= a.w)
        || (v >= b.x && v <= b.y);
    if inside {
        o.color = o.color * i.parameters[2];
    } else {
        o.color = vec4<f32>(0.0);
    }
    return o;
}
