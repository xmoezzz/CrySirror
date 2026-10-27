// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Immutable local mesh BVHs and library-built world instance bounds.
use crate::schema::*;
use anyhow::{Context, Result, ensure};
use glam::{Mat4, Vec3};
use parry3d::{
    bounding_volume::{Aabb, BoundingVolume},
    na::{Point3, Vector3},
    partitioning::Qbvh,
    query::{Ray, RayCast, visitors::RayIntersectionsVisitor},
    shape::TriMesh,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
type Surface = ([Vec3; 3], u32, usize);
enum Index {
    Empty,
    Parry(TriMesh),
    #[cfg(feature = "obvhs")]
    Obvhs {
        bvh: obvhs::bvh2::Bvh2,
        triangles: Vec<parry3d::shape::Triangle>,
    },
}
struct Geometry {
    index: Index,
    triangles: Vec<Surface>,
}
#[derive(Clone)]
struct Instance {
    geometry: Arc<Geometry>,
    transform: Mat4,
    inverse: Mat4,
    world: Arc<Vec<Surface>>,
    bounds: Option<Aabb>,
    offset: u32,
    id: u32,
    enabled: bool,
}
/// Reusable immutable geometry; publication never mutates a captured query scene.
pub struct AcousticScene {
    top: Qbvh<u32>,
    instances: Vec<Instance>,
    lookup: Arc<BTreeMap<u32, usize>>,
    materials: Arc<Vec<AcousticMaterial>>,
    pub backend: AcousticBackend,
    reused_bvhs: usize,
    changed_meshes: usize,
}
/// Snapshot work counters, not allocator or elapsed-time measurements.
#[derive(Clone, Debug, Serialize)]
pub struct AcousticGeometryStats {
    /// Stored meshes, including disabled and empty meshes.
    pub meshes: usize,
    /// Enabled nonempty mesh instances in the top-level BVH.
    pub active_meshes: usize,
    /// Stored triangles; disabling geometry does not bypass scene budgets.
    pub triangles: usize,
    /// Local mesh BVHs shared with the preceding immutable snapshot.
    pub reused_mesh_bvhs: usize,
    /// Mesh poses/enabled flags that actually changed in this snapshot.
    pub changed_meshes: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct SurfaceHit {
    pub distance: f32,
    pub mesh: u32,
    pub material: usize,
    pub normal: [f32; 3],
}
fn transform_valid(matrix: Mat4) -> Result<Mat4> {
    let determinant = matrix.determinant();
    ensure!(
        matrix.is_finite()
            && determinant.is_finite()
            && determinant.abs() > 1e-12
            && matrix.row(3) == glam::Vec4::W,
        "acoustic transform must be finite, affine and invertible"
    );
    let inverse = matrix.inverse();
    ensure!(
        inverse.is_finite(),
        "acoustic inverse transform is not finite"
    );
    Ok(inverse)
}
impl Instance {
    fn positioned(
        geometry: Arc<Geometry>,
        matrix: Mat4,
        offset: u32,
        id: u32,
        enabled: bool,
    ) -> Result<Self> {
        let inverse = transform_valid(matrix)?;
        let mut bounds: Option<Aabb> = None;
        let world = geometry
            .triangles
            .iter()
            .map(|(tri, mesh, material)| {
                let vertices = tri.map(|p| matrix.transform_point3(p));
                ensure!(
                    vertices
                        .iter()
                        .all(|p| p.is_finite() && p.abs().max_element() <= 1e6),
                    "invalid acoustic world vertex"
                );
                let area = (vertices[1] - vertices[0])
                    .cross(vertices[2] - vertices[0])
                    .length_squared();
                ensure!(
                    area.is_finite() && area > 1e-14,
                    "degenerate acoustic world triangle"
                );
                for p in vertices {
                    let p = Point3::from(p.to_array());
                    if let Some(a) = &mut bounds {
                        a.mins = a.mins.inf(&p);
                        a.maxs = a.maxs.sup(&p);
                    } else {
                        bounds = Some(Aabb::new(p, p));
                    }
                }
                Ok((vertices, *mesh, *material))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            geometry,
            transform: matrix,
            inverse,
            world: Arc::new(world),
            bounds,
            offset,
            id,
            enabled,
        })
    }
    fn closest(
        &self,
        origin: Vec3,
        direction: Vec3,
        min: f32,
        max: f32,
    ) -> Result<Option<(usize, f32)>> {
        let origin = self.inverse.transform_point3(origin);
        let direction = self.inverse.transform_vector3(direction);
        let scale = direction.length();
        ensure!(
            origin.is_finite() && scale.is_finite() && scale > 0. && (max * scale).is_finite(),
            "acoustic transformed ray exceeds numeric range"
        );
        let direction = direction / scale;
        let (min, max) = (min * scale, max * scale);
        let raw = match &self.geometry.index {
            Index::Empty => None,
            Index::Parry(mesh) => {
                let p = origin + direction * min;
                let ray = Ray::new(
                    Point3::from(p.to_array()),
                    Vector3::from(direction.to_array()),
                );
                mesh.cast_local_ray_and_get_normal(&ray, max - min, false)
                    .map(|h| {
                        (
                            h.feature.unwrap_face() as usize % self.geometry.triangles.len(),
                            min + h.time_of_impact,
                        )
                    })
            }
            #[cfg(feature = "obvhs")]
            Index::Obvhs { bvh, triangles } => {
                let ray = obvhs::ray::Ray::new(
                    glam_obvhs::Vec3A::from_array(origin.to_array()),
                    glam_obvhs::Vec3A::from_array(direction.to_array()),
                    min,
                    max,
                );
                let mut hit = obvhs::ray::RayHit::none();
                bvh.ray_traverse(ray, &mut hit, |ray, id| {
                    let p = ray.origin;
                    let d = ray.direction;
                    let r = Ray::new(Point3::new(p.x, p.y, p.z), Vector3::new(d.x, d.y, d.z));
                    triangles[bvh.primitive_indices[id] as usize]
                        .cast_local_ray(&r, ray.tmax, false)
                        .filter(|t| *t >= ray.tmin)
                        .unwrap_or(f32::INFINITY)
                })
                .then(|| {
                    (
                        bvh.primitive_indices[hit.primitive_id as usize] as usize,
                        hit.t,
                    )
                })
            }
        };
        Ok(raw.map(|(face, t)| (face, t / scale)))
    }
}
fn top(instances: &[Instance]) -> Qbvh<u32> {
    let mut bvh = Qbvh::new();
    let boxes = instances
        .iter()
        .enumerate()
        .filter(|(_, m)| m.enabled)
        .filter_map(|(i, m)| {
            m.bounds.map(|b| {
                let extent = b.mins.coords.abs().max().max(b.maxs.coords.abs().max());
                (i as u32, b.loosened(epsilon(extent)))
            })
        })
        .collect::<Vec<_>>();
    if !boxes.is_empty() {
        bvh.clear_and_rebuild(boxes.into_iter(), 0.);
    }
    bvh
}
impl AcousticScene {
    pub fn triangle_count(&self) -> usize {
        self.instances
            .iter()
            .map(|m| m.geometry.triangles.len())
            .sum()
    }
    pub fn material(&self, index: usize) -> &AcousticMaterial {
        &self.materials[index]
    }
    pub fn geometry_stats(&self) -> AcousticGeometryStats {
        AcousticGeometryStats {
            meshes: self.instances.len(),
            active_meshes: self
                .instances
                .iter()
                .filter(|m| m.enabled && m.bounds.is_some())
                .count(),
            triangles: self.triangle_count(),
            reused_mesh_bvhs: self.reused_bvhs,
            changed_meshes: self.changed_meshes,
        }
    }
    pub fn mesh_pose(&self, id: u32) -> Option<AcousticMeshUpdate> {
        self.lookup
            .get(&id)
            .map(|index| &self.instances[*index])
            .map(|m| AcousticMeshUpdate {
                id,
                transform: Some(m.transform.to_cols_array()),
                enabled: Some(m.enabled),
            })
    }
    pub fn build(spec: AcousticSceneSpec) -> Result<Self> {
        ensure!(
            spec.meshes.len() <= 4096 && spec.materials.len() <= 256,
            "acoustic mesh/material count"
        );
        #[cfg(not(feature = "obvhs"))]
        ensure!(
            !matches!(spec.backend, AcousticBackend::Obvhs),
            "obvhs acoustic backend is not enabled"
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
        let mut ids = BTreeSet::new();
        let (mut vertices_count, mut triangles_count) = (0usize, 0usize);
        let mut instances = Vec::new();
        for mesh in spec.meshes {
            ensure!(ids.insert(mesh.id), "duplicate acoustic mesh ID");
            vertices_count = vertices_count
                .checked_add(mesh.vertices.len())
                .context("acoustic vertex count")?;
            ensure!(
                vertices_count <= 1_000_000 && triangles_count + mesh.triangles.len() <= 262144,
                "acoustic geometry limit"
            );
            ensure!(
                mesh.materials.is_empty() || mesh.materials.len() == mesh.triangles.len(),
                "acoustic material index count"
            );
            let vertices = mesh
                .vertices
                .iter()
                .map(|p| Vec3::from(*p))
                .collect::<Vec<_>>();
            ensure!(
                vertices
                    .iter()
                    .all(|p| p.is_finite() && p.abs().max_element() <= 1e6),
                "invalid acoustic local vertex"
            );
            let triangles = mesh
                .triangles
                .iter()
                .enumerate()
                .map(|(i, index)| {
                    let get = |id: u32| {
                        vertices
                            .get(id as usize)
                            .copied()
                            .context("acoustic triangle index out of bounds")
                    };
                    let tri = [get(index[0])?, get(index[1])?, get(index[2])?];
                    let area = (tri[1] - tri[0]).cross(tri[2] - tri[0]).length_squared();
                    ensure!(
                        area.is_finite() && area > 0.,
                        "degenerate acoustic local triangle"
                    );
                    let material = mesh.materials.get(i).copied().unwrap_or(0) as usize;
                    ensure!(material < spec.materials.len(), "acoustic material missing");
                    Ok((tri, mesh.id, material))
                })
                .collect::<Result<Vec<_>>>()?;
            let geometry = Arc::new(Geometry {
                index: Index::Empty,
                triangles,
            });
            let mut instance = Instance::positioned(
                geometry,
                Mat4::from_cols_array(&mesh.transform),
                triangles_count as u32,
                mesh.id,
                true,
            )?;
            let local = Arc::get_mut(&mut instance.geometry)
                .context("new acoustic geometry unexpectedly shared")?;
            local.index = build_index(&local.triangles, spec.backend)?;
            instances.push(instance);
            triangles_count += mesh.triangles.len();
        }
        Ok(Self {
            top: top(&instances),
            lookup: Arc::new(
                instances
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (m.id, i))
                    .collect(),
            ),
            instances,
            materials: Arc::new(spec.materials),
            backend: spec.backend,
            reused_bvhs: 0,
            changed_meshes: 0,
        })
    }
    /// Worker-side immutable instance commit; all local mesh BVHs are retained.
    pub fn updated(&self, updates: &[AcousticMeshUpdate]) -> Result<Self> {
        self.updated_cancellable(updates, || Ok(()))
    }
    pub fn updated_cancellable(
        &self,
        updates: &[AcousticMeshUpdate],
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        ensure!(
            !updates.is_empty() && updates.len() <= 4096,
            "acoustic instance update count"
        );
        let mut pending = BTreeMap::new();
        for u in updates {
            u.validate()?;
            ensure!(
                pending.insert(u.id, u).is_none(),
                "duplicate acoustic mesh update"
            );
        }
        let mut instances = self.instances.clone();
        let mut changed = 0;
        for instance in &mut instances {
            check()?;
            let Some(update) = pending.remove(&instance.id) else {
                continue;
            };
            let matrix = update
                .transform
                .map(|v| Mat4::from_cols_array(&v))
                .unwrap_or(instance.transform);
            let enabled = update.enabled.unwrap_or(instance.enabled);
            if matrix != instance.transform {
                *instance = Instance::positioned(
                    instance.geometry.clone(),
                    matrix,
                    instance.offset,
                    instance.id,
                    enabled,
                )?;
                changed += 1;
            } else if instance.enabled != enabled {
                instance.enabled = enabled;
                changed += 1;
            }
        }
        ensure!(pending.is_empty(), "unknown acoustic mesh update ID");
        check()?;
        let bvh = top(&instances);
        check()?;
        Ok(Self {
            top: bvh,
            lookup: self.lookup.clone(),
            reused_bvhs: instances
                .iter()
                .filter(|m| !m.geometry.triangles.is_empty())
                .count(),
            instances,
            materials: self.materials.clone(),
            backend: self.backend,
            changed_meshes: changed,
        })
    }
    /// Segment interval [min,max) in normalized world-distance units.
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
        let shifted = origin + direction * min;
        ensure!(
            shifted.is_finite(),
            "acoustic ray start exceeds numeric range"
        );
        let ray = Ray::new(
            Point3::from(shifted.to_array()),
            Vector3::from(direction.to_array()),
        );
        let mut best: Option<(usize, usize, f32)> = None;
        let mut error = None;
        let mut visit = |id: &u32| {
            let instance = &self.instances[*id as usize];
            if !instance.enabled || ignore == Some(instance.id) {
                return true;
            }
            let bound = best.as_ref().map_or(max, |h| h.2);
            match instance.closest(origin, direction, min, bound) {
                Ok(Some((face, t))) if t < max => {
                    let stable = instance.offset as usize + face;
                    if best.is_none_or(|(m, f, d)| {
                        t < d || (t == d && stable < self.instances[m].offset as usize + f)
                    }) {
                        best = Some((*id as usize, face, t));
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    error = Some(e);
                    return false;
                }
            }
            true
        };
        self.top
            .traverse_depth_first(&mut RayIntersectionsVisitor::new(
                &ray,
                max - min,
                &mut visit,
            ));
        if let Some(e) = error {
            return Err(e);
        }
        Ok(best.map(|(id, face, distance)| {
            let (tri, mesh, material) = &self.instances[id].world[face];
            SurfaceHit {
                distance,
                mesh: *mesh,
                material: *material,
                normal: (tri[1] - tri[0])
                    .cross(tri[2] - tri[0])
                    .normalize()
                    .to_array(),
            }
        }))
    }
    pub(crate) fn surfaces(&self) -> impl Iterator<Item = (u32, &Surface)> {
        self.instances.iter().filter(|m| m.enabled).flat_map(|m| {
            m.world
                .iter()
                .enumerate()
                .map(move |(id, tri)| (m.offset + id as u32, tri))
        })
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

#[cfg(test)]
mod instance_tests {
    use super::*;
    #[test]
    fn immutable_pose_commit_reuses_blas_and_unchanged_world_pages() {
        let mesh = |id| AcousticMesh {
            id,
            vertices: vec![[-1., -1., 2.], [1., -1., 2.], [0., 1., 2.]],
            triangles: vec![[0, 1, 2]],
            materials: vec![],
            transform: Mat4::IDENTITY.to_cols_array(),
        };
        let before = AcousticScene::build(AcousticSceneSpec {
            materials: vec![Default::default()],
            meshes: vec![mesh(7), mesh(8)],
            ..Default::default()
        })
        .unwrap();
        let after = before
            .updated(&[AcousticMeshUpdate {
                id: 7,
                transform: Some(Mat4::from_translation(Vec3::X * 4.).to_cols_array()),
                enabled: None,
            }])
            .unwrap();
        for i in 0..2 {
            assert!(Arc::ptr_eq(
                &before.instances[i].geometry,
                &after.instances[i].geometry
            ));
        }
        assert!(!Arc::ptr_eq(
            &before.instances[0].world,
            &after.instances[0].world
        ));
        assert!(Arc::ptr_eq(
            &before.instances[1].world,
            &after.instances[1].world
        ));
        assert_eq!(after.geometry_stats().reused_mesh_bvhs, 2);
        assert_eq!(
            before.mesh_pose(7).unwrap().transform.unwrap(),
            Mat4::IDENTITY.to_cols_array()
        );
        let canceled = before.updated_cancellable(
            &[AcousticMeshUpdate {
                id: 7,
                transform: None,
                enabled: Some(false),
            }],
            || anyhow::bail!("canceled"),
        );
        assert!(canceled.is_err());
        assert_eq!(before.geometry_stats().active_meshes, 2);
    }
}

fn build_index(triangles: &[Surface], backend: AcousticBackend) -> Result<Index> {
    let index = if triangles.is_empty() {
        Index::Empty
    } else {
        match backend {
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
    Ok(index)
}
