// SPDX-License-Identifier: MPL-2.0
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
fn white() -> [u8; 4] {
    [255; 4]
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene3d {
    pub camera: Camera,
    pub meshes: Vec<Mesh>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    pub name: String,
    pub geometry: Geometry,
    #[serde(default)]
    pub translation: [f32; 3],
    #[serde(default)]
    pub rotation_degrees: [f32; 3],
    #[serde(default = "one3")]
    pub scale: [f32; 3],
    #[serde(default = "white")]
    pub color: [u8; 4],
}
fn one3() -> [f32; 3] {
    [1.; 3]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Cube,
    /// A glTF mesh (all primitives, with their PBR materials), referenced by
    /// client-scoped asset path and mesh index. Rendered by the PBR path;
    /// device resources are rebuilt from this logical reference (ADR 0010).
    Model {
        asset: String,
        mesh: u32,
    },
    /// A whole glTF scene (node hierarchy, skins, animations) as one entity.
    Scene {
        asset: String,
    },
    /// Heightfield terrain centred on the entity origin (ADR 0014).
    Terrain(crate::terrain::TerrainSpec),
    Triangles {
        vertices: Vec<[f32; 3]>,
        indices: Vec<u32>,
    },
}
fn finite(v: &[f32]) -> bool {
    v.iter().all(|n| n.is_finite() && n.abs() <= 1_000_000.)
}
impl Scene3d {
    pub fn validate(&self) -> Result<()> {
        let c = &self.camera;
        ensure!(
            finite(&c.eye) && finite(&c.target) && finite(&c.up),
            "non-finite camera"
        );
        ensure!(
            c.fov_y_degrees.is_finite() && (1.0..179.0).contains(&c.fov_y_degrees),
            "camera FOV"
        );
        ensure!(
            c.near.is_finite()
                && c.far.is_finite()
                && c.near > 0.
                && c.far > c.near
                && c.far <= 1_000_000.,
            "camera clip planes"
        );
        let d = [
            c.target[0] - c.eye[0],
            c.target[1] - c.eye[1],
            c.target[2] - c.eye[2],
        ];
        let cross = [
            d[1] * c.up[2] - d[2] * c.up[1],
            d[2] * c.up[0] - d[0] * c.up[2],
            d[0] * c.up[1] - d[1] * c.up[0],
        ];
        ensure!(
            cross.iter().map(|x| x * x).sum::<f32>() > 1e-12,
            "camera target/up are degenerate"
        );
        ensure!(self.meshes.len() <= 4096, "too many meshes");
        let mut names = BTreeSet::new();
        let mut total = 0usize;
        for m in &self.meshes {
            ensure!(
                !m.name.is_empty() && names.insert(&m.name),
                "duplicate/empty mesh name"
            );
            ensure!(
                finite(&m.translation)
                    && finite(&m.rotation_degrees)
                    && finite(&m.scale)
                    && m.scale.iter().all(|s| s.abs() > 1e-6),
                "invalid mesh transform"
            );
            ensure!(
                m.color[3] == 255,
                "prototype 3D supports opaque meshes only"
            );
            let count = match &m.geometry {
                Geometry::Cube => 36,
                Geometry::Terrain(t) => {
                    t.validate()?;
                    0
                }
                Geometry::Scene { asset } => {
                    ensure!(
                        !asset.is_empty()
                            && asset.len() <= 4096
                            && !asset.starts_with('/')
                            && !asset.contains(".."),
                        "invalid scene reference"
                    );
                    0
                }
                Geometry::Model { asset, mesh } => {
                    ensure!(
                        !asset.is_empty()
                            && asset.len() <= 4096
                            && !asset.starts_with('/')
                            && !asset.contains("..")
                            && !asset.contains('\\')
                            && *mesh < 65536,
                        "invalid model reference"
                    );
                    0
                }
                Geometry::Triangles { vertices, indices } => {
                    ensure!(
                        !vertices.is_empty() && !indices.is_empty() && indices.len() % 3 == 0,
                        "invalid triangles"
                    );
                    ensure!(vertices.iter().all(|v| finite(v)), "non-finite vertex");
                    ensure!(
                        indices.iter().all(|i| (*i as usize) < vertices.len()),
                        "mesh index out of range"
                    );
                    indices.len()
                }
            };
            total = total.checked_add(count).context("vertex count overflow")?;
            ensure!(total <= 1_000_000, "too many expanded vertices");
        }
        Ok(())
    }
}
