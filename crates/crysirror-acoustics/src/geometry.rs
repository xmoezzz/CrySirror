// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Immutable BVH publication. Ray distances always refer to the original origin.
use crate::schema::*;
use anyhow::{Context, Result, ensure};
use glam::{Mat4, Vec3};
use parry3d::{
    na::{Point3, Vector3},
    query::{Ray, RayCast},
    shape::TriMesh,
};
use serde::Serialize;
enum Index {
    Empty,
    Parry(TriMesh),
    #[cfg(feature = "obvhs")]
    Obvhs {
        bvh: obvhs::bvh2::Bvh2,
        triangles: Vec<parry3d::shape::Triangle>,
    },
}
pub struct AcousticScene {
    index: Index,
    triangles: Vec<([Vec3; 3], u32, usize)>,
    materials: Vec<AcousticMaterial>,
    pub backend: AcousticBackend,
}
#[derive(Clone, Debug, Serialize)]
pub struct SurfaceHit {
    pub distance: f32,
    pub mesh: u32,
    pub material: usize,
    pub normal: [f32; 3],
}
impl AcousticScene {
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }
    pub fn material(&self, index: usize) -> &AcousticMaterial {
        &self.materials[index]
    }
    pub fn build(spec: AcousticSceneSpec) -> Result<Self> {
        ensure!(
            spec.meshes.len() <= 4096 && spec.materials.len() <= 256,
            "acoustic mesh/material count"
        );
        for m in &spec.materials {
            ensure!(
                m.absorption
                    .iter()
                    .chain(&m.transmission)
                    .all(|x| x.is_finite() && (0.0..=1.).contains(x))
                    && m.scattering.is_finite()
                    && (0.0..=1.).contains(&m.scattering),
                "invalid acoustic material"
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut triangles = Vec::new();
        let mut total_vertices = 0usize;
        for mesh in spec.meshes {
            ensure!(ids.insert(mesh.id), "duplicate acoustic mesh ID");
            total_vertices = total_vertices
                .checked_add(mesh.vertices.len())
                .context("acoustic vertex count")?;
            ensure!(
                total_vertices <= 1_000_000 && triangles.len() + mesh.triangles.len() <= 262144,
                "acoustic geometry limit"
            );
            ensure!(
                mesh.materials.is_empty() || mesh.materials.len() == mesh.triangles.len(),
                "acoustic material index count"
            );
            let transform = Mat4::from_cols_array(&mesh.transform);
            ensure!(
                transform.is_finite()
                    && transform.determinant().abs() > 1e-12
                    && transform.row(3) == glam::Vec4::W,
                "acoustic transform must be affine and invertible"
            );
            let vertices = mesh
                .vertices
                .iter()
                .map(|p| {
                    let p = transform.transform_point3(Vec3::from(*p));
                    ensure!(
                        p.is_finite() && p.abs().max_element() <= 1e6,
                        "invalid acoustic vertex"
                    );
                    Ok(p)
                })
                .collect::<Result<Vec<_>>>()?;
            for (i, indices) in mesh.triangles.iter().enumerate() {
                let get = |id: u32| {
                    vertices
                        .get(id as usize)
                        .copied()
                        .context("acoustic triangle index out of bounds")
                };
                let tri = [get(indices[0])?, get(indices[1])?, get(indices[2])?];
                ensure!(
                    (tri[1] - tri[0]).cross(tri[2] - tri[0]).length_squared() > 1e-14,
                    "degenerate acoustic triangle"
                );
                let material = mesh.materials.get(i).copied().unwrap_or(0) as usize;
                ensure!(material < spec.materials.len(), "acoustic material missing");
                triangles.push((tri, mesh.id, material));
            }
        }
        #[cfg(not(feature = "obvhs"))]
        ensure!(
            !matches!(spec.backend, AcousticBackend::Obvhs),
            "obvhs acoustic backend is not enabled"
        );
        let index = if triangles.is_empty() {
            Index::Empty
        } else {
            match spec.backend {
                AcousticBackend::Parry => {
                    let vertices = triangles
                        .iter()
                        .flat_map(|(tri, _, _)| tri.map(|p| Point3::new(p.x, p.y, p.z)))
                        .collect();
                    let indices = (0..triangles.len() as u32)
                        .map(|i| [i * 3, i * 3 + 1, i * 3 + 2])
                        .collect();
                    Index::Parry(TriMesh::new(vertices, indices))
                }
                AcousticBackend::Obvhs => {
                    #[cfg(feature = "obvhs")]
                    {
                        let triangles = triangles
                            .iter()
                            .map(|(tri, _, _)| obvhs::triangle::Triangle {
                                v0: glam_obvhs::Vec3A::from_array(tri[0].to_array()),
                                v1: glam_obvhs::Vec3A::from_array(tri[1].to_array()),
                                v2: glam_obvhs::Vec3A::from_array(tri[2].to_array()),
                            })
                            .collect::<Vec<_>>();
                        let mut params = obvhs::BvhBuildParams::medium_build();
                        params.pre_split = false;
                        let bvh = obvhs::bvh2::builder::build_bvh2_from_tris(
                            &triangles,
                            params,
                            &mut std::time::Duration::default(),
                        );
                        let triangles = triangles
                            .into_iter()
                            .map(|t| {
                                parry3d::shape::Triangle::new(
                                    Point3::from(t.v0.to_array()),
                                    Point3::from(t.v1.to_array()),
                                    Point3::from(t.v2.to_array()),
                                )
                            })
                            .collect();
                        Index::Obvhs { bvh, triangles }
                    }
                    #[cfg(not(feature = "obvhs"))]
                    unreachable!()
                }
            }
        };
        Ok(Self {
            index,
            triangles,
            materials: spec.materials,
            backend: spec.backend,
        })
    }
    /// Segment interval is [min,max) in world-distance units, including skipped surfaces.
    pub fn closest_hit(
        &self,
        origin: Vec3,
        direction: Vec3,
        min: f32,
        max: f32,
        ignore: Option<u32>,
    ) -> Result<Option<SurfaceHit>> {
        ensure!(
            origin.is_finite()
                && direction.is_finite()
                && direction.length_squared().is_finite()
                && direction.length_squared() > 1e-12
                && min.is_finite()
                && max.is_finite()
                && min >= 0.
                && max > min,
            "invalid acoustic ray interval"
        );
        let direction = direction.normalize();
        let mut start = min;
        for _ in 0..256 {
            let raw = match &self.index {
                Index::Empty => None,
                Index::Parry(mesh) => {
                    let p = origin + direction * start;
                    let ray = Ray::new(
                        Point3::new(p.x, p.y, p.z),
                        Vector3::new(direction.x, direction.y, direction.z),
                    );
                    mesh.cast_local_ray_and_get_normal(&ray, max - start, false)
                        .map(|hit| {
                            let face = hit.feature.unwrap_face() as usize % self.triangles.len();
                            (face, start + hit.time_of_impact)
                        })
                }
                #[cfg(feature = "obvhs")]
                Index::Obvhs { bvh, triangles } => {
                    let ray = obvhs::ray::Ray::new(
                        glam_obvhs::Vec3A::from_array(origin.to_array()),
                        glam_obvhs::Vec3A::from_array(direction.to_array()),
                        start,
                        max,
                    );
                    let mut hit = obvhs::ray::RayHit::none();
                    bvh.ray_traverse(ray, &mut hit, |ray, id| {
                        {
                            // Use Parry's primitive test: obvhs 0.4.0's bit-sign
                            // barycentric check rejects negative zero on shared edges.
                            let p = ray.origin;
                            let d = ray.direction;
                            let primitive_ray =
                                Ray::new(Point3::new(p.x, p.y, p.z), Vector3::new(d.x, d.y, d.z));
                            triangles[bvh.primitive_indices[id] as usize]
                                .cast_local_ray(&primitive_ray, ray.tmax, false)
                                .filter(|t| *t >= ray.tmin)
                                .unwrap_or(f32::INFINITY)
                        }
                    })
                    .then(|| {
                        (
                            bvh.primitive_indices[hit.primitive_id as usize] as usize,
                            hit.t,
                        )
                    })
                }
            };
            let Some((id, distance)) = raw else {
                return Ok(None);
            };
            if distance >= max {
                return Ok(None);
            }
            let (tri, mesh, material) = &self.triangles[id];
            if ignore != Some(*mesh) {
                return Ok(Some(SurfaceHit {
                    distance,
                    mesh: *mesh,
                    material: *material,
                    normal: (tri[1] - tri[0])
                        .cross(tri[2] - tri[0])
                        .normalize()
                        .to_array(),
                }));
            }
            start = distance + epsilon(distance);
            if start >= max {
                return Ok(None);
            }
        }
        anyhow::bail!("acoustic ignored-surface traversal budget exceeded")
    }
    pub(crate) fn surfaces(&self) -> impl ExactSizeIterator<Item = &([Vec3; 3], u32, usize)> {
        self.triangles.iter()
    }
    pub fn blocked(&self, from: Vec3, to: Vec3, ignore: Option<u32>) -> Result<bool> {
        let delta = to - from;
        let d = delta.length();
        if d <= 0.002 {
            return Ok(false);
        }
        Ok(self
            .closest_hit(from, delta, epsilon(d), d - epsilon(d), ignore)?
            .is_some())
    }
}
pub(crate) fn epsilon(distance: f32) -> f32 {
    0.0001f32.max(distance.abs() * 4. * f32::EPSILON)
}
