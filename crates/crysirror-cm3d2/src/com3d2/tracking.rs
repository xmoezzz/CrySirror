// SPDX-License-Identifier: MPL-2.0
//! Managed CameraChase and TBody head/eye control in the original Unity axes.
use super::*;
use crate::chara::{Maid, unity_euler};
use crate::kag::EyeLook;
use crysirror_asset::{
    anim::{self, PoseLayer},
    interchange::{
        CM3D2_CLIP,
        cm3d2::{Mesh, convert_animation},
    },
    pbr::{Clip, PbrModel, SceneNode, Trs, load_geometry},
};
use glam::{Mat4, Quat, Vec3};
use std::collections::{BTreeMap, BTreeSet};

fn mirror(v: Vec3) -> Vec3 {
    Vec3::new(-v.x, v.y, v.z)
}
fn mirror_q(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

/// The camera samples the same rest hierarchy, named clip channels, blend
/// weights and loop clocks as the renderer, without decoding material images.
struct Skeleton {
    file: String,
    model: PbrModel,
    clips: BTreeMap<String, Clip>,
}
impl Skeleton {
    fn load(file: &str, files: &GameFiles, op: &Opener) -> Result<Self> {
        let mesh = Mesh::parse(&files.read(op.storage.as_ref(), file)?)?;
        let mut model = PbrModel::default();
        for b in &mesh.bones {
            model.nodes.push(SceneNode {
                name: b.name.clone(),
                children: Vec::new(),
                mesh: None,
                skin: None,
                rest: Trs {
                    translation: mirror(Vec3::from_array(b.translation)),
                    rotation: mirror_q(Quat::from_array(b.rotation)),
                    scale: Vec3::from_array(b.scale),
                },
            });
        }
        for (i, b) in mesh.bones.iter().enumerate() {
            if let Some(parent) = b.parent {
                model
                    .nodes
                    .get_mut(parent)
                    .context("bone parent")?
                    .children
                    .push(i);
            } else {
                model.roots.push(i);
            }
            if b.scale_node {
                let child = model.nodes.len();
                model.nodes[i].children.push(child);
                model.nodes.push(SceneNode {
                    name: format!("{}_SCL_", b.name),
                    children: vec![],
                    mesh: None,
                    skin: None,
                    rest: Trs {
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                });
            }
        }
        Ok(Self {
            file: file.into(),
            model,
            clips: BTreeMap::new(),
        })
    }
    fn sample(
        &mut self,
        actor: &Maid,
        files: &GameFiles,
        op: &Opener,
        now: u64,
        transform: Mat4,
    ) -> Result<Pose> {
        self.clips
            .retain(|name, _| actor.anims.iter().any(|a| &a.file == name));
        for a in actor.anims.iter().filter(|a| a.queued.is_none()) {
            if !self.clips.contains_key(&a.file) {
                let bytes = files.read(op.storage.as_ref(), &a.file)?;
                let glb = convert_animation(&bytes, CM3D2_CLIP)?.to_glb()?;
                let source = load_geometry(&glb, |_| bail!("external file in embedded motion"))?;
                let clip = source.animations.first().context("motion has no clip")?;
                self.clips
                    .insert(a.file.clone(), anim::retarget(&source, clip, &self.model));
            }
        }
        let layers: Vec<_> = actor
            .anims
            .iter()
            .filter(|a| a.queued.is_none())
            .map(|a| PoseLayer {
                clip: &self.clips[&a.file],
                time: now.saturating_sub(a.start) as f32 / 1000.,
                weight: a.weight(now),
                looping: a.looping,
                layer: a.layer,
            })
            .collect();
        let local = anim::local_pose_layered(&self.model, &layers);
        let global = anim::global_pose(&self.model, &local);
        Ok(Pose {
            local,
            global,
            transform,
        })
    }
    fn index(&self, name: &str) -> Option<usize> {
        self.model.nodes.iter().position(|n| n.name == name)
    }
}
struct Pose {
    local: Vec<Trs>,
    global: Vec<Mat4>,
    transform: Mat4,
}
impl Pose {
    fn world(&self, index: usize) -> Mat4 {
        let reflect = Mat4::from_scale(Vec3::new(-1., 1., 1.));
        self.transform * reflect * self.global[index] * reflect
    }
}

struct Chase {
    actor: usize,
    bone: String,
    offset: Vec3,
    until: Option<u64>,
}
#[derive(Default)]
struct Gaze {
    weight: f32,
    tracking: bool,
    angles: Vec3,
    velocity: Vec3,
    eye: Vec3,
    away_offset: Option<Vec3>,
    away_frames: u16,
    away_side: bool,
    eye_file: Option<String>,
    eye_rest: Option<[Quat; 2]>,
}
fn rotation(m: Mat4) -> Quat {
    m.to_scale_rotation_translation().1.normalize()
}
fn angles(direction: Vec3) -> Vec3 {
    if direction.length_squared() < 1e-12 {
        return Vec3::ZERO;
    }
    let q = Quat::from_rotation_arc(Vec3::Y, direction.normalize());
    let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}
impl Gaze {
    fn step(
        &mut self,
        actor: &mut Maid,
        rig: &Skeleton,
        pose: &Pose,
        camera: Vec3,
        target: Option<Vec3>,
        dt: f32,
        files: &GameFiles,
        op: &Opener,
    ) -> Result<()> {
        let prefix = if actor.man { "ManBip" } else { "Bip01" };
        let head_name = format!("{prefix} Head");
        let Some(head) = rig.index(&head_name) else {
            return Ok(());
        };
        let Some(neck) = rig.index(&format!("{prefix} Neck")) else {
            return Ok(());
        };
        let h = pose.world(head);
        let n = pose.world(neck);
        let mut eyes = true;
        let mut away = false;
        let (head_on, point) = match &actor.eye.0 {
            EyeLook::Camera(mode) => {
                let head = match mode.as_str() {
                    "顔を向ける" | "目と顔を向ける" => true,
                    "顔だけ動かす" => {
                        eyes = false;
                        true
                    }
                    "顔をそらす" => {
                        away = true;
                        true
                    }
                    "目だけそらす" => {
                        away = true;
                        false
                    }
                    "無し" | "無視する" => {
                        eyes = false;
                        false
                    }
                    "目だけ向ける" => false,
                    _ => bail!("unknown EyeMoveType {mode}"),
                };
                (
                    head,
                    if away {
                        h.transform_point3(self.away_offset.unwrap_or(Vec3::Y))
                    } else {
                        camera
                    },
                )
            }
            EyeLook::Reset | EyeLook::None => {
                eyes = false;
                (false, camera)
            }
            EyeLook::Position(p) => {
                let offset = Vec3::from_array(*p);
                self.away_offset = Some(offset);
                (true, h.transform_point3(offset))
            }
            EyeLook::Chara { .. } => (
                true,
                target.unwrap_or_else(|| {
                    self.away_offset = Some(Vec3::Y);
                    h.transform_point3(Vec3::Y)
                }),
            ),
        };
        if away {
            let a = (point - h.w_axis.truncate()).normalize_or_zero();
            let b = (camera - h.w_axis.truncate()).normalize_or_zero();
            if self.away_frames > 0 {
                self.away_frames += 1;
                if self.away_frames > 200 {
                    self.away_frames = 0;
                }
            }
            if a.dot(b) > 0.9 && self.away_frames == 0 {
                self.away_offset = Some(if self.away_side {
                    Vec3::new(-0.5, 1., -0.7)
                } else {
                    Vec3::new(-0.6, 1., 0.6)
                });
                self.away_side = !self.away_side;
                self.away_frames = 1;
            }
        }
        let delta = dt
            / if actor.eye.1 <= 0. {
                0.001
            } else {
                actor.eye.1
            };
        self.weight = (self.weight + if head_on { delta } else { -delta }).clamp(0., 1.);
        let desired = angles(rotation(n).inverse() * (point - n.w_axis.truncate()));
        self.tracking = if self.tracking {
            desired.x > -80. && desired.x < 80. && desired.z > -50. && desired.z < 60.
        } else {
            desired.x > -60. && desired.x < 60. && desired.z > -40. && desired.z < 50.
        };
        for i in [0, 2] {
            if self.tracking {
                let diff = desired[i] - self.angles[i];
                if diff.abs() > 10. {
                    self.velocity[i] += diff.signum() * 0.3;
                } else {
                    self.velocity[i] *= 0.95;
                }
            } else if self.angles[i].abs() > 10. {
                self.velocity[i] -= self.angles[i].signum() * 0.1;
            }
        }
        self.velocity *= 0.95;
        self.angles += self.velocity;
        let base = mirror_q(pose.local[head].rotation);
        let rest = mirror_q(rig.model.nodes[head].rest.rotation);
        let next = base.slerp(
            rest * unity_euler((self.angles * 0.4).to_array()),
            crate::face::coss(self.weight),
        );
        actor
            .tracking_rotations
            .insert(head_name, mirror_q(next).to_array());
        if actor.man {
            return Ok(());
        }
        let face = actor.slots.get("head").and_then(|s| s.model.as_deref());
        if self.eye_file.as_deref() != face {
            self.eye_file = face.map(str::to_owned);
            self.eye_rest = None;
            if let Some(file) = face {
                let mesh = Mesh::parse(&files.read(op.storage.as_ref(), file)?)?;
                let bone = |name: &str| {
                    mesh.bones
                        .iter()
                        .find(|b| b.name == name)
                        .map(|b| Quat::from_array(b.rotation))
                };
                self.eye_rest = bone("Eye_L").zip(bone("Eye_R")).map(|(l, r)| [l, r]);
            }
        }
        if let Some([l, r]) = self.eye_rest {
            let head_world = rotation(h) * base.inverse() * next;
            if eyes && self.tracking {
                let want = angles(head_world.inverse() * (point - h.w_axis.truncate()));
                self.eye = self.eye.lerp(want, if away { 0.05 } else { 0.5 });
            } else {
                self.eye *= 0.95;
            }
            let e = [0., self.eye.x * 0.2, self.eye.z * 0.1];
            actor.tracking_rotations.insert(
                "Eye_L".into(),
                mirror_q(l * unity_euler(e.map(|v| -v))).to_array(),
            );
            actor
                .tracking_rotations
                .insert("Eye_R".into(), mirror_q(r * unity_euler(e)).to_array());
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Tracking {
    chase: Option<Chase>,
    gazes: BTreeMap<usize, Gaze>,
    rigs: BTreeMap<usize, Skeleton>,
}
impl Tracking {
    pub fn remove(&mut self, i: usize) {
        if self.chase.as_ref().is_some_and(|c| c.actor == i) {
            self.chase = None;
        }
        self.rigs.remove(&i);
        self.gazes.remove(&i);
    }
    pub fn follow(
        &mut self,
        i: Option<usize>,
        bone: String,
        offset: [f32; 3],
        once: bool,
        seconds: f32,
        now: u64,
    ) {
        self.chase = i.map(|actor| Chase {
            actor,
            bone,
            offset: offset.into(),
            until: once.then(|| now.saturating_add((seconds.max(0.) * 1000.).round() as u64)),
        });
    }
    pub fn stop(&mut self) {
        self.chase = None;
    }
    pub fn gaze(&mut self, i: usize) {
        self.gazes.entry(i).or_default();
    }
    pub fn update(
        &mut self,
        characters: &mut [Maid],
        slots: &BTreeMap<(bool, usize), usize>,
        position: Vec3,
        rot: Quat,
        orbit: &mut Orbit,
        files: &GameFiles,
        op: &Opener,
        now: u64,
        dt: f32,
    ) -> Result<bool> {
        let mut needed: BTreeSet<_> = self.gazes.keys().copied().collect();
        if let Some(c) = &self.chase {
            needed.insert(c.actor);
        }
        for &i in self.gazes.keys() {
            if let EyeLook::Chara { who, .. } = &characters[i].eye.0
                && let Some(&j) = slots.get(who)
            {
                needed.insert(j);
            }
        }
        let mut poses = BTreeMap::new();
        for i in needed {
            let m = &mut characters[i];
            if !m.visible {
                continue;
            }
            let Some(file) = m.slots.get("body").and_then(|s| s.model.as_ref()) else {
                continue;
            };
            if !self.rigs.get(&i).is_some_and(|r| &r.file == file) {
                self.rigs.insert(i, Skeleton::load(file, files, op)?);
            }
            m.advance_anims(now);
            let transform = Mat4::from_rotation_translation(
                rot * unity_euler(m.rotation),
                position + rot * Vec3::from_array(m.position),
            );
            poses.insert(
                i,
                self.rigs
                    .get_mut(&i)
                    .unwrap()
                    .sample(m, files, op, now, transform)?,
            );
        }
        let mut changed = false;
        if let Some(c) = &self.chase {
            let p = poses.get(&c.actor).and_then(|p| {
                Some(
                    p.world(self.rigs[&c.actor].index(&c.bone)?)
                        .w_axis
                        .truncate(),
                )
            });
            if let Some(p) = p {
                orbit.0 = (p + c.offset).to_array();
                changed = true;
            }
            if p.is_none() || c.until.is_some_and(|end| now >= end) {
                self.chase = None;
            }
        }
        let camera = Vec3::from_array(orbit.0)
            + Quat::from_rotation_y(orbit.2[0].to_radians())
                * Quat::from_rotation_x(orbit.2[1].to_radians())
                * Vec3::new(0., 0., -orbit.1);
        for (&i, g) in &mut self.gazes {
            let Some(pose) = poses.get(&i) else { continue };
            let m = &mut characters[i];
            let target = if let EyeLook::Chara { who, bone } = &m.eye.0 {
                slots.get(who).and_then(|j| {
                    Some(
                        poses
                            .get(j)?
                            .world(
                                self.rigs
                                    .get(j)?
                                    .index(bone.as_deref().unwrap_or("Bip01 Head"))?,
                            )
                            .w_axis
                            .truncate(),
                    )
                })
            } else {
                None
            };
            g.step(m, &self.rigs[&i], pose, camera, target, dt, files, op)?;
        }
        Ok(changed)
    }
}
