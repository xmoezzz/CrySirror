// SPDX-License-Identifier: MPL-2.0
// ---- split-sum BRDF LUT (x: N.V, y: roughness) ----
@fragment
fn fs_brdf(i: FsOut) -> @location(0) vec4<f32> {
    let nv = max(i.uv.x, 1e-3);
    let roughness = i.uv.y;
    let a = roughness * roughness;
    let v = vec3<f32>(sqrt(1.0 - nv * nv), 0.0, nv);
    let n = vec3<f32>(0.0, 0.0, 1.0);
    var sa = 0.0;
    var sb = 0.0;
    let count = 512u;
    for (var k = 0u; k < count; k = k + 1u) {
        let h = importance_ggx(vec2<f32>(f32(k) / f32(count), radical_inverse(k)), n, a);
        let l = normalize(2.0 * dot(v, h) * h - v);
        let nl = max(l.z, 0.0);
        let nh = max(h.z, 0.0);
        let vh = max(dot(v, h), 0.0);
        if nl > 0.0 {
            let k_ibl = a / 2.0;
            let g = (nv / (nv * (1.0 - k_ibl) + k_ibl)) * (nl / (nl * (1.0 - k_ibl) + k_ibl));
            let g_vis = g * vh / max(nh * nv, 1e-5);
            let fc = pow(1.0 - vh, 5.0);
            sa = sa + (1.0 - fc) * g_vis;
            sb = sb + fc * g_vis;
        }
    }
    return vec4<f32>(sa / f32(count), sb / f32(count), 0.0, 1.0);
}
