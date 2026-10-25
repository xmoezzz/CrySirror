// SPDX-License-Identifier: MPL-2.0
use crate::{Pose, Shape};
use anyhow::{ensure, Result};
use glam::{Mat4, Vec3};
use rapier3d::{na, prelude::*};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub(crate) fn vector(v: Vec3) -> Vector<Real> {
    Vector::new(v.x, v.y, v.z)
}
pub(crate) fn vec3(v: Vector<Real>) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}
pub(crate) fn point(v: Vec3) -> Point<Real> {
    Point::new(v.x, v.y, v.z)
}
pub(crate) fn iso(p: Pose) -> Isometry<Real> {
    Isometry::from_parts(
        Translation::from(vector(p.translation)),
        na::UnitQuaternion::new_normalize(na::Quaternion::new(
            p.rotation.w,
            p.rotation.x,
            p.rotation.y,
            p.rotation.z,
        )),
    )
}
pub(crate) fn pose(p: &Isometry<Real>) -> Pose {
    let q = p.rotation.quaternion();
    Pose {
        translation: vec3(p.translation.vector),
        rotation: glam::Quat::from_xyzw(q.i, q.j, q.k, q.w),
    }
}
pub(crate) fn validate_pose(p: Pose) -> Result<()> {
    ensure!(
        p.translation.is_finite()
            && p.rotation.is_finite()
            && (p.rotation.length_squared() - 1.).abs() < 0.001,
        "invalid physics pose/quaternion"
    );
    Ok(())
}
impl Shape {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Cuboid { half_extents: h } => {
                ensure!(h.is_finite() && h.min_element() > 0., "invalid cuboid")
            }
            Self::Ball { radius } => ensure!(radius.is_finite() && *radius > 0., "invalid ball"),
            Self::Capsule {
                radius,
                half_height,
            } => ensure!(
                radius.is_finite() && *radius > 0. && half_height.is_finite() && *half_height >= 0.,
                "invalid capsule"
            ),
            Self::TriMesh { vertices, indices } => {
                ensure!(
                    !vertices.is_empty()
                        && vertices.len() <= 4_000_000
                        && !indices.is_empty()
                        && indices.len() <= 4_000_000,
                    "mesh shape budget/empty geometry"
                );
                ensure!(
                    vertices.iter().all(|v| v.is_finite()),
                    "nonfinite mesh vertex"
                );
                for t in indices.iter() {
                    ensure!(
                        t.iter().all(|i| (*i as usize) < vertices.len())
                            && t[0] != t[1]
                            && t[1] != t[2]
                            && t[0] != t[2],
                        "invalid mesh index"
                    );
                    ensure!(
                        (vertices[t[1] as usize] - vertices[t[0] as usize])
                            .cross(vertices[t[2] as usize] - vertices[t[0] as usize])
                            .length_squared()
                            > 1e-20,
                        "degenerate mesh triangle"
                    );
                }
            }
            Self::ConvexHull { points } => ensure!(
                (4..=1_000_000).contains(&points.len()) && points.iter().all(|p| p.is_finite()),
                "invalid convex hull points"
            ),
            Self::HeightField {
                heights,
                rows,
                cols,
                scale,
            } => ensure!(
                *rows >= 2
                    && *cols >= 2
                    && rows.checked_mul(*cols) == Some(heights.len())
                    && heights.len() <= 4_000_000
                    && heights.iter().all(|h| h.is_finite())
                    && scale.is_finite()
                    && scale.min_element() > 0.,
                "invalid height field"
            ),
        }
        Ok(())
    }
    /// Exact primitive scaling only. A nonuniformly scaled sphere/capsule is
    /// not silently substituted with a different volume; callers can provide
    /// an authored convex hull instead.
    pub fn scaled(&self, scale: Vec3) -> Result<Self> {
        ensure!(
            scale.is_finite() && scale.min_element() > 0.,
            "physics scale must be positive and finite"
        );
        let uniform = (scale.max_element() - scale.min_element()) <= 1e-5 * scale.max_element();
        let shape = match self {
            Self::Cuboid { half_extents } => Self::Cuboid {
                half_extents: *half_extents * scale,
            },
            Self::Ball { radius } => {
                ensure!(
                    uniform,
                    "nonuniform sphere scale requires an explicit convex shape"
                );
                Self::Ball {
                    radius: radius * scale.x,
                }
            }
            Self::Capsule {
                radius,
                half_height,
            } => {
                ensure!(
                    uniform,
                    "nonuniform capsule scale requires an explicit convex shape"
                );
                Self::Capsule {
                    radius: radius * scale.x,
                    half_height: half_height * scale.x,
                }
            }
            Self::TriMesh { vertices, indices } => Self::TriMesh {
                vertices: Arc::new(vertices.iter().map(|p| *p * scale).collect()),
                indices: indices.clone(),
            },
            Self::ConvexHull { points } => Self::ConvexHull {
                points: Arc::new(points.iter().map(|p| *p * scale).collect()),
            },
            Self::HeightField {
                heights,
                rows,
                cols,
                scale: s,
            } => Self::HeightField {
                heights: heights.clone(),
                rows: *rows,
                cols: *cols,
                scale: *s * scale,
            },
        };
        shape.validate()?;
        Ok(shape)
    }
}
/// A rigid pose plus positive scale can be represented exactly. A rotated
/// nonuniform parent/child product may contain shear and is rejected here.
pub fn decompose_transform(matrix: Mat4) -> Result<(Pose, Vec3)> {
    ensure!(matrix.is_finite(), "nonfinite physics transform");
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    ensure!(
        scale.is_finite() && scale.min_element() > 0.,
        "reflected/zero physics scale unsupported"
    );
    let rebuilt = Mat4::from_scale_rotation_translation(scale, rotation, translation);
    let a = matrix.to_cols_array();
    let b = rebuilt.to_cols_array();
    let magnitude = scale.max_element().max(1.);
    ensure!(
        a[3].abs() < 1e-6 && a[7].abs() < 1e-6 && a[11].abs() < 1e-6 && (a[15] - 1.).abs() < 1e-6,
        "physics transform must be affine"
    );
    ensure!(
        a.iter()
            .zip(b)
            .all(|(a, b)| (*a - b).abs() <= magnitude * 1e-5),
        "sheared physics transform must be baked into explicit geometry"
    );
    let pose = Pose {
        translation,
        rotation,
    };
    validate_pose(pose)?;
    Ok((pose, scale))
}
fn fingerprint(shape: &Shape) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut f = |value: f32| value.to_bits().hash(&mut hasher);
    match shape {
        Shape::Cuboid { half_extents } => {
            for x in half_extents.to_array() {
                f(x);
            }
        }
        Shape::Ball { radius } => f(*radius),
        Shape::Capsule {
            radius,
            half_height,
        } => {
            f(*radius);
            f(*half_height);
        }
        Shape::TriMesh { vertices, indices } => {
            for v in vertices.iter() {
                for x in v.to_array() {
                    f(x);
                }
            }
            for t in indices.iter() {
                t.hash(&mut hasher);
            }
        }
        Shape::ConvexHull { points } => {
            for v in points.iter() {
                for x in v.to_array() {
                    f(x);
                }
            }
        }
        Shape::HeightField {
            heights,
            rows,
            cols,
            scale,
        } => {
            for h in heights.iter() {
                f(*h);
            }
            for x in scale.to_array() {
                f(x);
            }
            rows.hash(&mut hasher);
            cols.hash(&mut hasher);
        }
    };
    std::mem::discriminant(shape).hash(&mut hasher);
    hasher.finish()
}
struct Cached {
    desc: Shape,
    shape: SharedShape,
    last: u64,
}
pub(crate) struct ShapeCache {
    entries: HashMap<u64, Vec<Cached>>,
    capacity: usize,
    clock: u64,
    pub builds: u64,
    pub hits: u64,
}
impl ShapeCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            clock: 0,
            builds: 0,
            hits: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }
    pub fn get(&mut self, desc: &Shape) -> Result<SharedShape> {
        desc.validate()?;
        self.clock += 1;
        let hash = fingerprint(desc);
        if let Some(bucket) = self.entries.get_mut(&hash) {
            if let Some(entry) = bucket.iter_mut().find(|e| e.desc == *desc) {
                self.hits += 1;
                entry.last = self.clock;
                return Ok(entry.shape.clone());
            }
        }
        let shape = match desc {
            Shape::Cuboid { half_extents: h } => SharedShape::cuboid(h.x, h.y, h.z),
            Shape::Ball { radius } => SharedShape::ball(*radius),
            Shape::Capsule {
                radius,
                half_height,
            } => SharedShape::capsule_y(*half_height, *radius),
            Shape::TriMesh { vertices, indices } => SharedShape::trimesh(
                vertices.iter().copied().map(point).collect(),
                indices.as_ref().clone(),
            ),
            Shape::ConvexHull { points } => {
                SharedShape::convex_hull(&points.iter().copied().map(point).collect::<Vec<_>>())
                    .ok_or_else(|| anyhow::anyhow!("convex hull is degenerate"))?
            }
            Shape::HeightField {
                heights,
                rows,
                cols,
                scale,
            } => SharedShape::heightfield(
                na::DMatrix::from_fn(*rows, *cols, |r, c| heights[r * cols + c]),
                vector(*scale),
            ),
        };
        self.builds += 1;
        if self.len() >= self.capacity {
            if let Some((hash, index)) = self
                .entries
                .iter()
                .flat_map(|(h, v)| v.iter().enumerate().map(move |(i, e)| (*h, i, e.last)))
                .min_by_key(|v| v.2)
                .map(|(h, i, _)| (h, i))
            {
                let bucket = self.entries.get_mut(&hash).unwrap();
                bucket.swap_remove(index);
                if bucket.is_empty() {
                    self.entries.remove(&hash);
                }
            }
        }
        self.entries.entry(hash).or_default().push(Cached {
            desc: desc.clone(),
            shape: shape.clone(),
            last: self.clock,
        });
        Ok(shape)
    }
}
