// SPDX-License-Identifier: MPL-2.0
//! Skeletal animation: keyframe sampling, multi-clip blending, hierarchy
//! evaluation and joint palettes (glTF 2.0 semantics). Device-free.
use crate::pbr::{ChannelPath, Clip, Interpolation, PbrModel, Trs};
use glam::{Mat4, Quat, Vec3, Vec4};

pub const MAX_JOINTS: usize = 256;

fn sample(times: &[f32], t: f32) -> (usize, usize, f32) {
    if t <= times[0] {
        return (0, 0, 0.);
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return (last, last, 0.);
    }
    let i = times.partition_point(|x| *x <= t) - 1;
    let span = (times[i + 1] - times[i]).max(1e-9);
    (i, i + 1, (t - times[i]) / span)
}
fn cubic(v0: Vec4, b0: Vec4, a1: Vec4, v1: Vec4, s: f32, dt: f32) -> Vec4 {
    let (s2, s3) = (s * s, s * s * s);
    v0 * (2. * s3 - 3. * s2 + 1.)
        + b0 * dt * (s3 - 2. * s2 + s)
        + v1 * (-2. * s3 + 3. * s2)
        + a1 * dt * (s3 - s2)
}

/// Local node transforms for one clip at time `t` (seconds, already wrapped).
pub fn sample_clip(model: &PbrModel, clip: &Clip, t: f32, out: &mut [Trs]) {
    for c in &clip.channels {
        let Some(node) = out.get_mut(c.node) else {
            continue;
        };
        let (i, j, f) = sample(&c.times, t);
        let v = match c.interpolation {
            Interpolation::Step => Vec4::from_array(c.values[i]),
            Interpolation::Linear => {
                let (a, b) = (Vec4::from_array(c.values[i]), Vec4::from_array(c.values[j]));
                if c.path == ChannelPath::Rotation {
                    let q = Quat::from_vec4(a).slerp(Quat::from_vec4(b), f);
                    Vec4::from(q)
                } else {
                    a.lerp(b, f)
                }
            }
            Interpolation::CubicSpline => {
                let k = |n: usize, m: usize| Vec4::from_array(c.values[n * 3 + m]);
                let dt = c.times[j] - c.times[i];
                let r = cubic(k(i, 1), k(i, 2), k(j, 0), k(j, 1), f, dt);
                if c.path == ChannelPath::Rotation {
                    r.normalize()
                } else {
                    r
                }
            }
        };
        match c.path {
            ChannelPath::Translation => node.translation = v.truncate(),
            ChannelPath::Scale => node.scale = v.truncate(),
            ChannelPath::Rotation => node.rotation = Quat::from_vec4(v).normalize(),
        }
    }
    let _ = model;
}

/// One weighted clip contribution.
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub clip: usize,
    pub time: f32,
    pub weight: f32,
}

/// Blend clips into local transforms (weights normalized; rest pose when
/// no layer is given) and evaluate global node matrices.
pub fn pose(model: &PbrModel, layers: &[Layer]) -> Vec<Mat4> {
    let rest: Vec<Trs> = model.nodes.iter().map(|n| n.rest).collect();
    let total: f32 = layers.iter().map(|l| l.weight.max(0.)).sum();
    let local: Vec<Trs> = if layers.is_empty() || total <= 1e-6 {
        rest
    } else {
        let mut acc_t = vec![Vec3::ZERO; rest.len()];
        let mut acc_s = vec![Vec3::ZERO; rest.len()];
        let mut acc_r = vec![Vec4::ZERO; rest.len()];
        let mut first_r: Vec<Option<Quat>> = vec![None; rest.len()];
        for l in layers {
            let Some(clip) = model.animations.get(l.clip) else {
                continue;
            };
            let w = l.weight.max(0.) / total;
            let mut p = rest.clone();
            let t = if clip.duration > 0. {
                l.time.rem_euclid(clip.duration)
            } else {
                0.
            };
            sample_clip(model, clip, t, &mut p);
            for (i, x) in p.iter().enumerate() {
                acc_t[i] += x.translation * w;
                acc_s[i] += x.scale * w;
                // Keep quaternions in one hemisphere before averaging.
                let r = match first_r[i] {
                    Some(f) if f.dot(x.rotation) < 0. => -x.rotation,
                    Some(_) => x.rotation,
                    None => {
                        first_r[i] = Some(x.rotation);
                        x.rotation
                    }
                };
                acc_r[i] += Vec4::from(r) * w;
            }
        }
        (0..rest.len())
            .map(|i| Trs {
                translation: acc_t[i],
                rotation: Quat::from_vec4(acc_r[i]).normalize(),
                scale: acc_s[i],
            })
            .collect()
    };
    let mut global = vec![Mat4::IDENTITY; local.len()];
    let mut stack: Vec<(usize, Mat4)> = model.roots.iter().map(|r| (*r, Mat4::IDENTITY)).collect();
    let mut guard = 0;
    while let Some((n, parent)) = stack.pop() {
        guard += 1;
        if guard > 100_000 || n >= local.len() {
            break;
        }
        let g = parent * local[n].matrix();
        global[n] = g;
        for c in &model.nodes[n].children {
            stack.push((*c, g));
        }
    }
    global
}

/// Joint matrices for `skin`, in the space of the skinned mesh node.
pub fn palette(model: &PbrModel, skin: usize, mesh_node: usize, global: &[Mat4]) -> Vec<Mat4> {
    let s = &model.skins[skin];
    let inv_mesh = global[mesh_node].inverse();
    s.joints
        .iter()
        .zip(&s.inverse_bind)
        .map(|(j, ibm)| inv_mesh * global[*j] * *ibm)
        .collect()
}

pub fn clip_index(model: &PbrModel, name: &str) -> Option<usize> {
    model.animations.iter().position(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbr::{Channel, SceneNode, Skin};
    fn two_bone() -> PbrModel {
        let rest = |y: f32| Trs {
            translation: Vec3::new(0., y, 0.),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        PbrModel {
            nodes: vec![
                SceneNode {
                    name: "root".into(),
                    children: vec![1],
                    mesh: None,
                    skin: None,
                    rest: rest(0.),
                },
                SceneNode {
                    name: "tip".into(),
                    children: vec![],
                    mesh: None,
                    skin: None,
                    rest: rest(1.),
                },
            ],
            roots: vec![0],
            skins: vec![Skin {
                joints: vec![0, 1],
                inverse_bind: vec![
                    Mat4::IDENTITY,
                    Mat4::from_translation(Vec3::new(0., -1., 0.)),
                ],
            }],
            animations: vec![Clip {
                name: "bend".into(),
                duration: 1.,
                channels: vec![Channel {
                    node: 0,
                    path: ChannelPath::Rotation,
                    interpolation: Interpolation::Linear,
                    times: vec![0., 1.],
                    values: vec![
                        Quat::IDENTITY.to_array(),
                        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array(),
                    ],
                }],
            }],
            ..Default::default()
        }
    }
    #[test]
    fn hierarchy_sampling_blending_and_palette() {
        let m = two_bone();
        // Rest pose: tip at y=1, palette identity.
        let g = pose(&m, &[]);
        assert!((g[1].w_axis.truncate() - Vec3::Y).length() < 1e-5);
        let p = palette(&m, 0, 0, &g);
        assert!(p.iter().all(|x| x.abs_diff_eq(Mat4::IDENTITY, 1e-5)));
        // Half-way: 45 degrees about Z moves the tip to (-sin45, cos45).
        let g = pose(
            &m,
            &[Layer {
                clip: 0,
                time: 0.5,
                weight: 1.,
            }],
        );
        let tip = g[1].w_axis.truncate();
        assert!(
            (tip - Vec3::new(-0.70710677, 0.70710677, 0.)).length() < 1e-4,
            "{tip}"
        );
        // 50/50 blend of t=0 and t=1 equals the half-way pose.
        let b = pose(
            &m,
            &[
                Layer {
                    clip: 0,
                    time: 0.,
                    weight: 1.,
                },
                Layer {
                    clip: 0,
                    time: 0.999_999,
                    weight: 1.,
                },
            ],
        );
        assert!((b[1].w_axis.truncate() - tip).length() < 1e-3);
        assert_eq!(clip_index(&m, "bend"), Some(0));
    }
}

/// Evaluate independent character poses on engine CPU workers. Results preserve
/// request order; hierarchy/blend evaluation within a character is unchanged.
pub fn poses_parallel(
    pool: &crysirror_tasks::TaskPool,
    requests: &[(std::sync::Arc<PbrModel>, Vec<Layer>)],
) -> anyhow::Result<Vec<Vec<Mat4>>> {
    let mut out = Vec::with_capacity(requests.len());
    let batch = pool.stats().workers.max(1) * 2;
    for chunk in requests.chunks(batch) {
        let jobs = chunk
            .iter()
            .map(|(model, layers)| {
                let model = model.clone();
                let layers = layers.clone();
                pool.spawn(move |cancel| {
                    cancel.check()?;
                    Ok(pose(&model, &layers))
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        for job in jobs {
            out.push(job.wait()?);
        }
    }
    Ok(out)
}
