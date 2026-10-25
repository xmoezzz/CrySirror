// SPDX-License-Identifier: MPL-2.0
//! Immutable CPU geometry snapshots consumed by the native AS service.
use crate::{geometry, hardware_rt::RtTriangle};
use anyhow::{Context, Result, ensure};
use crysirror_asset::pbr::{PbrModel, PbrPrimitive};
use glam::{Mat4, Vec3, Vec4};
use std::sync::Arc;
#[derive(Clone)]
pub(crate) enum Source {
    Model {
        model: Arc<PbrModel>,
        mesh: usize,
        primitive: usize,
    },
    Owned(Arc<PbrPrimitive>),
}
#[derive(Clone)]
pub(crate) struct Primitive {
    pub source: Source,
    pub matrix: Mat4,
    pub palette: Option<Vec<Mat4>>,
}
impl Primitive {
    fn source(&self) -> Result<&PbrPrimitive> {
        match &self.source {
            Source::Model {
                model,
                mesh,
                primitive,
            } => model
                .meshes
                .get(*mesh)
                .and_then(|p| p.get(*primitive))
                .context("RT primitive missing"),
            Source::Owned(p) => Ok(p),
        }
    }
    fn triangles(&self) -> Result<Vec<RtTriangle>> {
        let p = self.source()?;
        ensure!(p.indices.len() % 3 == 0, "RT index count");
        let mut out = Vec::with_capacity(p.indices.len() / 3);
        for indices in p.indices.chunks_exact(3) {
            let mut v = [Vec3::ZERO; 3];
            for (i, index) in indices.iter().enumerate() {
                let index = *index as usize;
                let vertex = p.vertices.get(index).context("RT vertex index")?;
                let mut point = Vec3::from_array(vertex.position);
                if let Some(palette) = &self.palette {
                    let (joints, weights) = p
                        .skin_attrs
                        .as_ref()
                        .and_then(|a| a.get(index))
                        .context("RT skin attributes")?;
                    let mut posed = Vec4::ZERO;
                    for (joint, weight) in joints.iter().zip(weights) {
                        if *weight != 0. {
                            posed += *palette.get(*joint as usize).context("RT joint index")?
                                * point.extend(1.)
                                * *weight;
                        }
                    }
                    point = posed.truncate();
                }
                v[i] = self.matrix.transform_point3(point);
                ensure!(v[i].is_finite(), "RT transformed vertex not finite");
            }
            let n = (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero();
            if n.length_squared() == 0. {
                continue;
            }
            out.push(RtTriangle {
                a: v[0].to_array(),
                b: v[1].to_array(),
                c: v[2].to_array(),
                normal: n.to_array(),
                color: [1.; 4],
            });
        }
        Ok(out)
    }
}
#[cfg(test)]
pub(crate) fn prepare(
    pool: &crysirror_tasks::TaskPool,
    requests: &[Primitive],
) -> Result<Vec<RtTriangle>> {
    let total = requests
        .iter()
        .map(|p| p.source().map(|p| p.indices.len() / 3))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum::<usize>();
    ensure!(
        total <= crate::rt_capabilities::MAX_RT_PRIMITIVES as usize,
        "hardware RT triangle preparation budget exceeded"
    );
    let mut out = Vec::with_capacity(total);
    for batch in requests.chunks(pool.stats().workers.max(1) * 2) {
        let jobs = batch
            .iter()
            .cloned()
            .map(|p| {
                pool.spawn(move |cancel| {
                    cancel.check()?;
                    p.triangles()
                })
            })
            .collect::<Result<Vec<_>>>()?;
        for job in jobs {
            out.extend(job.wait()?);
        }
    }
    Ok(out)
}
type GeometryKey = (usize, usize, usize, Vec<u32>);
#[derive(Default)]
pub(crate) struct InstanceGeometryCache {
    entries: std::collections::HashMap<GeometryKey, (Source, Arc<Vec<RtTriangle>>)>,
    pub hits: u64,
    pub misses: u64,
}
impl InstanceGeometryCache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    /// Keep local geometry immutable and shared across rigid transforms/frames.
    /// Source Arcs are retained with cache entries, preventing pointer-ID reuse.
    pub fn prepare(
        &mut self,
        pool: &crysirror_tasks::TaskPool,
        requests: &[Primitive],
    ) -> Result<Vec<crate::hardware_rt::RtInstance>> {
        use std::collections::{HashMap, HashSet};
        let key = |r: &Primitive| {
            let (ptr, mesh, primitive) = match &r.source {
                Source::Model {
                    model,
                    mesh,
                    primitive,
                } => (Arc::as_ptr(model) as usize, *mesh, *primitive),
                Source::Owned(p) => (Arc::as_ptr(p) as usize, usize::MAX, 0),
            };
            let pose: Vec<u32> = r
                .palette
                .as_ref()
                .map(|p| {
                    p.iter()
                        .flat_map(|m| m.to_cols_array().map(f32::to_bits))
                        .collect()
                })
                .unwrap_or_default();
            (ptr, mesh, primitive, pose)
        };
        let keys: Vec<_> = requests.iter().map(key).collect();
        let used: HashSet<_> = keys.iter().cloned().collect();
        self.entries.retain(|k, _| used.contains(k));
        let mut unique = HashMap::new();
        for (r, k) in requests.iter().zip(&keys) {
            unique.entry(k.clone()).or_insert(r);
        }
        let total = unique
            .values()
            .map(|r| r.source().map(|p| p.indices.len() / 3))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .sum::<usize>();
        ensure!(
            total <= crate::rt_capabilities::MAX_RT_PRIMITIVES as usize && unique.len() <= 1024,
            "RT unique geometry preparation budget exceeded"
        );
        let missing: Vec<_> = unique
            .into_iter()
            .filter_map(|(k, r)| {
                if self.entries.contains_key(&k) {
                    self.hits += 1;
                    None
                } else {
                    self.misses += 1;
                    let mut r = r.clone();
                    r.matrix = Mat4::IDENTITY;
                    Some((k, r))
                }
            })
            .collect();
        for batch in missing.chunks(pool.stats().workers.max(1) * 2) {
            let jobs = batch
                .iter()
                .map(|(k, r)| {
                    let source = r.source.clone();
                    let p = r.clone();
                    Ok((
                        k.clone(),
                        source,
                        pool.spawn(move |c| {
                            c.check()?;
                            p.triangles().map(Arc::new)
                        })?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            for (k, source, job) in jobs {
                self.entries.insert(k, (source, job.wait()?));
            }
        }
        Ok(requests
            .iter()
            .zip(keys)
            .map(|(r, k)| crate::hardware_rt::RtInstance {
                triangles: self.entries[&k].1.clone(),
                transform: r.matrix.to_cols_array_2d(),
            })
            .collect())
    }
}
pub(crate) fn simple(scene: &crysirror_runtime::model::Scene3d) -> Result<Vec<RtTriangle>> {
    let vertices = geometry::triangles(scene)?;
    let mut out = Vec::with_capacity(vertices.len() / 3);
    for v in vertices.chunks_exact(3) {
        ensure!(
            v[0].color[3] >= 1.,
            "hardware primary path supports opaque geometry only"
        );
        out.push(RtTriangle {
            a: v[0].position,
            b: v[1].position,
            c: v[2].position,
            normal: v[0].normal,
            color: v[0].color,
        });
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transformed_and_skinned_ray_vertices_follow_the_shared_render_pose() {
        let p = PbrPrimitive {
            vertices: vec![
                crysirror_asset::pbr::PbrVertex {
                    position: [0., 0., 0.],
                    ..Default::default()
                },
                crysirror_asset::pbr::PbrVertex {
                    position: [1., 0., 0.],
                    ..Default::default()
                },
                crysirror_asset::pbr::PbrVertex {
                    position: [0., 1., 0.],
                    ..Default::default()
                },
            ],
            indices: vec![0, 1, 2],
            skin_attrs: Some(vec![([0; 4], [1., 0., 0., 0.]); 3]),
            material: 0,
            bounds: ([0.; 3], [1.; 3]),
        };
        let request = Primitive {
            source: Source::Owned(Arc::new(p)),
            matrix: Mat4::from_translation(Vec3::new(10., 0., 0.)),
            palette: Some(vec![Mat4::from_translation(Vec3::new(0., 2., 0.))]),
        };
        let pool = crysirror_tasks::TaskPool::new("rt", 2, 8).unwrap();
        let triangles = prepare(&pool, &[request]).unwrap();
        assert_eq!(triangles[0].a, [10., 2., 0.]);
        assert_eq!(triangles[0].b, [11., 2., 0.]);
        assert_eq!(triangles[0].c, [10., 3., 0.]);
        let source = Source::Owned(Arc::new(PbrPrimitive {
            vertices: vec![
                crysirror_asset::pbr::PbrVertex {
                    position: [0., 0., 0.],
                    ..Default::default()
                },
                crysirror_asset::pbr::PbrVertex {
                    position: [1., 0., 0.],
                    ..Default::default()
                },
                crysirror_asset::pbr::PbrVertex {
                    position: [0., 1., 0.],
                    ..Default::default()
                },
            ],
            indices: vec![0, 1, 2],
            skin_attrs: None,
            material: 0,
            bounds: ([0.; 3], [1.; 3]),
        }));
        let one = Primitive {
            source,
            matrix: Mat4::IDENTITY,
            palette: None,
        };
        let mut two = one.clone();
        two.matrix = Mat4::from_translation(Vec3::X);
        let mut cache = InstanceGeometryCache::default();
        let frame = cache.prepare(&pool, &[one.clone(), two.clone()]).unwrap();
        assert!(Arc::ptr_eq(&frame[0].triangles, &frame[1].triangles));
        assert_eq!(cache.misses, 1);
        two.matrix = Mat4::from_translation(Vec3::Y);
        let next = cache.prepare(&pool, &[one, two]).unwrap();
        assert!(Arc::ptr_eq(&frame[0].triangles, &next[0].triangles));
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.hits, 1);
    }
}
