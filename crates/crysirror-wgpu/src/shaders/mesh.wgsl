struct Camera { view_projection: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
}
@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>) -> Out {
    var out: Out;
    out.position = camera.view_projection * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.color = color;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(0.35, 0.8, 0.5));
    let intensity = 0.25 + 0.75 * max(dot(normalize(in.normal), light), 0.0);
    return vec4<f32>(in.color.rgb * intensity, in.color.a);
}
