// SPDX-License-Identifier: MPL-2.0
use anyhow::{ensure, Result};
use bytemuck::{Pod, Zeroable};
use crysirror_core::state::Rect;
use crysirror_runtime::model::{Geometry, Scene3d};
use glam::{EulerRot, Mat4, Quat, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex2d {
    pub position: [f32; 4],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// Index into the frame's layer-filter buffer (0 = none).
    pub filter: f32,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex3d {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}
pub fn linear(v: u8) -> f32 {
    let s = f32::from(v) / 255.;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}
pub fn tint(c: [u8; 4]) -> [f32; 4] {
    [
        linear(c[0]),
        linear(c[1]),
        linear(c[2]),
        f32::from(c[3]) / 255.,
    ]
}
pub fn quad(rect: Rect, canvas: [u32; 2], color: [u8; 4]) -> [Vertex2d; 6] {
    let x = rect.x as f32;
    let y = rect.y as f32;
    let w = rect.width as f32;
    let h = rect.height as f32;
    let ndc = |px: f32, py: f32| {
        [
            2. * px / canvas[0] as f32 - 1.,
            1. - 2. * py / canvas[1] as f32,
            0.,
            1.,
        ]
    };
    let color = tint(color);
    let v = |px, py, u, v| Vertex2d {
        position: ndc(px, py),
        uv: [u, v],
        color,
        filter: 0.,
    };
    [
        v(x, y, 0., 0.),
        v(x, y + h, 0., 1.),
        v(x + w, y + h, 1., 1.),
        v(x, y, 0., 0.),
        v(x + w, y + h, 1., 1.),
        v(x + w, y, 1., 0.),
    ]
}
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Viewport {
    pub fn fit(canvas: [u32; 2], output: [u32; 2]) -> Self {
        let scale = (output[0] as f32 / canvas[0] as f32).min(output[1] as f32 / canvas[1] as f32);
        let width = canvas[0] as f32 * scale;
        let height = canvas[1] as f32 * scale;
        Self {
            x: (output[0] as f32 - width) * 0.5,
            y: (output[1] as f32 - height) * 0.5,
            width,
            height,
        }
    }
    pub fn pointer(&self, x: f64, y: f64, canvas: [u32; 2]) -> [i32; 2] {
        if x < self.x as f64
            || y < self.y as f64
            || x >= (self.x + self.width) as f64
            || y >= (self.y + self.height) as f64
        {
            return [-1, -1];
        }
        [
            ((x - self.x as f64) / self.width as f64 * canvas[0] as f64) as i32,
            ((y - self.y as f64) / self.height as f64 * canvas[1] as f64) as i32,
        ]
    }
}
pub fn triangles(scene: &Scene3d) -> Result<Vec<Vertex3d>> {
    scene.validate()?;
    let mut out = Vec::new();
    const CUBE: [[f32; 3]; 8] = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    const INDICES: [u32; 36] = [
        4, 5, 6, 4, 6, 7, 0, 3, 2, 0, 2, 1, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 3, 7, 6, 3, 6, 2,
        0, 1, 5, 0, 5, 4,
    ];
    for mesh in &scene.meshes {
        let (vertices, indices): (&[[f32; 3]], &[u32]) = match &mesh.geometry {
            Geometry::Cube => (&CUBE, &INDICES),
            Geometry::Triangles { vertices, indices } => (vertices, indices),
            // glTF models are drawn only by the PBR path (ADR 0010).
            Geometry::Model { .. } | Geometry::Scene { .. } | Geometry::Terrain(_) => continue,
        };
        let r = mesh.rotation_degrees.map(f32::to_radians);
        let model = Mat4::from_scale_rotation_translation(
            Vec3::from_array(mesh.scale),
            Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]),
            Vec3::from_array(mesh.translation),
        );
        let normal_matrix = model.inverse().transpose();
        let color = tint(mesh.color);
        for face in indices.chunks_exact(3) {
            let p = face
                .iter()
                .map(|i| Vec3::from_array(vertices[*i as usize]))
                .collect::<Vec<_>>();
            let cross = (p[1] - p[0]).cross(p[2] - p[0]);
            ensure!(
                cross.length_squared() > 1e-16,
                "degenerate triangle in {}",
                mesh.name
            );
            let normal = normal_matrix
                .transform_vector3(cross.normalize())
                .normalize()
                .to_array();
            for pos in p {
                out.push(Vertex3d {
                    position: model.transform_point3(pos).to_array(),
                    normal,
                    color,
                });
            }
        }
    }
    Ok(out)
}
pub fn camera(scene: &Scene3d, canvas: [u32; 2]) -> [[f32; 4]; 4] {
    let c = &scene.camera;
    (Mat4::perspective_rh(
        c.fov_y_degrees.to_radians(),
        canvas[0] as f32 / canvas[1] as f32,
        c.near,
        c.far,
    ) * Mat4::look_at_rh(
        Vec3::from_array(c.eye),
        Vec3::from_array(c.target),
        Vec3::from_array(c.up),
    ))
    .to_cols_array_2d()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aspect_fit_and_pointer_agree() {
        let v = Viewport::fit([1280, 720], [800, 800]);
        assert_eq!(v.height, 450.);
        assert_eq!(v.pointer(400., 400., [1280, 720]), [640, 360]);
        assert_eq!(v.pointer(0., 0., [1280, 720]), [-1, -1]);
    }
    #[test]
    fn quad_uses_top_left_coordinates() {
        let q = quad(
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            [100, 100],
            [255; 4],
        );
        assert_eq!(q[0].position, [-1., 1., 0., 1.]);
        assert_eq!(q[2].position, [1., -1., 0., 1.]);
    }
    #[test]
    fn linear_white_and_black() {
        assert_eq!(linear(0), 0.);
        assert!((linear(255) - 1.).abs() < 1e-6);
    }
}

pub fn layer_quad(
    p: &crysirror_scene::Primitive,
    canvas: [u32; 2],
    color: [u8; 4],
) -> [Vertex2d; 6] {
    let mut tint = tint(color);
    tint[3] *= f32::from(p.opacity) / 255.;
    let uv = p.uv;
    let corner = |i: usize, u, v| {
        let [x, y, z, w] = p.quad[i];
        Vertex2d {
            position: [
                (2. * x / canvas[0] as f32 - 1.) * w,
                (1. - 2. * y / canvas[1] as f32) * w,
                z * w,
                w,
            ],
            uv: [u, v],
            color: tint,
            filter: 0.,
        }
    };
    [
        corner(0, uv[0], uv[1]),
        corner(3, uv[0], uv[3]),
        corner(2, uv[2], uv[3]),
        corner(0, uv[0], uv[1]),
        corner(2, uv[2], uv[3]),
        corner(1, uv[2], uv[1]),
    ]
}
