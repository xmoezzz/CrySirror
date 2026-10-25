// SPDX-License-Identifier: MPL-2.0
//! Rig evaluation: parameters ← defaults, motions, scripted tweens and
//! physics; parts ← keyform offsets, deformer chains and opacity.
use crate::{
    Curve, FramePart, Interpolation, MotionBlend, Part, PartKey, PuppetFrame, PuppetRig,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// Physics sub-step (s); larger frame deltas are split.
const PHYSICS_STEP: f32 = 1. / 120.;
/// Longest single advance; a stalled host does not explode the physics.
const MAX_ADVANCE_MS: f32 = 1000.;

/// A scripted parameter value, eased from `from` to `to`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub from: f32,
    pub to: f32,
    pub start_ms: f64,
    pub duration_ms: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayingMotion {
    pub id: String,
    pub start_ms: f64,
    pub looping: bool,
    pub weight: f32,
    /// When a stop was requested (fade-out start).
    pub stop_ms: Option<f64>,
}
/// Everything that changes while a puppet runs; saved with the game.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PuppetState {
    pub time_ms: f64,
    /// Parameter values of the last evaluation.
    pub values: BTreeMap<String, f32>,
    pub overrides: BTreeMap<String, Override>,
    pub motions: Vec<PlayingMotion>,
    /// Pendulum (angle degrees, angular velocity) by pendulum index.
    pub physics: Vec<[f32; 2]>,
}

/// A rig plus its running state.
#[derive(Clone, Debug)]
pub struct Puppet {
    rig: Arc<PuppetRig>,
    state: PuppetState,
}

fn smooth(t: f32) -> f32 {
    t * t * (3. - 2. * t)
}
pub(crate) fn sample_curve(c: &Curve, t: f32) -> f32 {
    let k = &c.keys;
    if t <= k[0][0] {
        return k[0][1];
    }
    let last = k[k.len() - 1];
    if t >= last[0] {
        return last[1];
    }
    let i = k.partition_point(|key| key[0] <= t) - 1;
    let (a, b) = (k[i], k[i + 1]);
    let f = (t - a[0]) / (b[0] - a[0]);
    let f = match c.interpolation {
        Interpolation::Linear => f,
        Interpolation::Step => 0.,
        Interpolation::Smooth => smooth(f),
    };
    a[1] + (b[1] - a[1]) * f
}
/// Segment and fraction of `v` among increasing key values (clamped).
fn segment(values: impl ExactSizeIterator<Item = f32> + Clone, v: f32) -> (usize, usize, f32) {
    let n = values.len();
    let vals: Vec<f32> = values.collect();
    if n == 1 || v <= vals[0] {
        return (0, 0, 0.);
    }
    if v >= vals[n - 1] {
        return (n - 1, n - 1, 0.);
    }
    let i = vals.partition_point(|x| *x <= v) - 1;
    (i, i + 1, (v - vals[i]) / (vals[i + 1] - vals[i]))
}

/// 2D affine [a, b, c, d, tx, ty]: x' = a·x + c·y + tx, y' = b·x + d·y + ty.
type Affine = [f32; 6];
const IDENTITY: Affine = [1., 0., 0., 1., 0., 0.];
fn mul(p: &Affine, q: &Affine) -> Affine {
    [
        p[0] * q[0] + p[2] * q[1],
        p[1] * q[0] + p[3] * q[1],
        p[0] * q[2] + p[2] * q[3],
        p[1] * q[2] + p[3] * q[3],
        p[0] * q[4] + p[2] * q[5] + p[4],
        p[1] * q[4] + p[3] * q[5] + p[5],
    ]
}
fn apply(m: &Affine, v: [f32; 2]) -> [f32; 2] {
    [m[0] * v[0] + m[2] * v[1] + m[4], m[1] * v[0] + m[3] * v[1] + m[5]]
}

impl Puppet {
    pub fn new(rig: Arc<PuppetRig>) -> Result<Self> {
        rig.validate()?;
        let mut p = Self { state: PuppetState::default(), rig };
        p.state.physics = vec![[0., 0.]; p.rig.physics.len()];
        p.state.values = p.defaults();
        Ok(p)
    }
    /// Restores a saved state; it must belong to this rig.
    pub fn with_state(rig: Arc<PuppetRig>, state: PuppetState) -> Result<Self> {
        let mut p = Self::new(rig)?;
        ensure!(state.physics.len() == p.rig.physics.len(), "puppet state does not match the rig's physics");
        for id in state.values.keys().chain(state.overrides.keys()) {
            ensure!(p.rig.parameter(id).is_some(), "puppet state names unknown parameter '{id}'");
        }
        for m in &state.motions {
            ensure!(p.rig.motions.iter().any(|x| x.id == m.id), "puppet state names unknown motion '{}'", m.id);
        }
        ensure!(state.time_ms.is_finite() && state.time_ms >= 0., "puppet state time");
        p.state = state;
        Ok(p)
    }
    pub fn rig(&self) -> &Arc<PuppetRig> {
        &self.rig
    }
    pub fn state(&self) -> &PuppetState {
        &self.state
    }
    fn defaults(&self) -> BTreeMap<String, f32> {
        self.rig.parameters.iter().map(|p| (p.id.clone(), p.default)).collect()
    }
    pub fn value(&self, id: &str) -> Option<f32> {
        self.state.values.get(id).copied()
    }
    /// Scripted value: eases from the current value over `duration_ms` and
    /// then holds, overriding motions until released.
    pub fn set_parameter(&mut self, id: &str, value: f32, duration_ms: f32) -> Result<()> {
        let p = self.rig.parameter(id).with_context(|| format!("unknown puppet parameter '{id}'"))?;
        ensure!(value.is_finite() && duration_ms.is_finite() && duration_ms >= 0., "invalid parameter value or duration");
        let to = value.clamp(p.min, p.max);
        let from = self.state.values.get(id).copied().unwrap_or(p.default);
        self.state.overrides.insert(id.into(), Override { from, to, start_ms: self.state.time_ms, duration_ms });
        if duration_ms == 0. {
            self.state.values.insert(id.into(), to);
        }
        Ok(())
    }
    pub fn release_parameter(&mut self, id: &str) -> Result<()> {
        ensure!(self.rig.parameter(id).is_some(), "unknown puppet parameter '{id}'");
        self.state.overrides.remove(id);
        Ok(())
    }
    pub fn play(&mut self, id: &str, looping: Option<bool>, weight: f32) -> Result<()> {
        let m = self.rig.motions.iter().find(|m| m.id == id).with_context(|| format!("unknown puppet motion '{id}'"))?;
        ensure!(weight.is_finite() && (0.0..=1.).contains(&weight), "motion weight must be 0..1");
        self.state.motions.retain(|p| p.id != id);
        self.state.motions.push(PlayingMotion {
            id: id.into(),
            start_ms: self.state.time_ms,
            looping: looping.unwrap_or(m.looping),
            weight,
            stop_ms: None,
        });
        Ok(())
    }
    /// Fades the motion out; an empty id stops all motions.
    pub fn stop(&mut self, id: &str) -> Result<()> {
        if !id.is_empty() {
            ensure!(self.rig.motions.iter().any(|m| m.id == id), "unknown puppet motion '{id}'");
        }
        let now = self.state.time_ms;
        for m in self.state.motions.iter_mut().filter(|m| id.is_empty() || m.id == id) {
            m.stop_ms.get_or_insert(now);
        }
        Ok(())
    }
    pub fn playing(&self) -> Vec<&str> {
        self.state.motions.iter().map(|m| m.id.as_str()).collect()
    }

    /// Advances time, re-evaluates parameters and steps physics.
    pub fn advance(&mut self, dt_ms: f32) -> Result<()> {
        if !dt_ms.is_finite() || dt_ms < 0. {
            bail!("invalid puppet time step {dt_ms}");
        }
        let dt_ms = dt_ms.min(MAX_ADVANCE_MS);
        self.state.time_ms += dt_ms as f64;
        let now = self.state.time_ms;
        let mut values = self.defaults();
        // Motions in start order; finished ones are dropped.
        let rig = self.rig.clone();
        self.state.motions.retain(|pm| {
            let m = rig.motions.iter().find(|m| m.id == pm.id).expect("validated motion");
            let local = (now - pm.start_ms) as f32;
            let out_start = match (pm.stop_ms, pm.looping) {
                (Some(s), _) => Some((s - pm.start_ms) as f32),
                (None, false) => Some((m.duration_ms - m.fade_out_ms).max(0.)),
                (None, true) => None,
            };
            let out_len = if pm.stop_ms.is_none() { m.duration_ms - out_start.unwrap_or(0.) } else { m.fade_out_ms };
            if let Some(s) = out_start {
                if local >= s + out_len.max(0.) {
                    return false;
                }
            }
            true
        });
        for pm in &self.state.motions {
            let m = rig.motions.iter().find(|m| m.id == pm.id).unwrap();
            let local = (now - pm.start_ms) as f32;
            let t = if pm.looping { local.rem_euclid(m.duration_ms) } else { local.min(m.duration_ms) };
            let fade_in = if m.fade_in_ms > 0. { (local / m.fade_in_ms).min(1.) } else { 1. };
            let fade_out = match (pm.stop_ms, pm.looping) {
                (Some(s), _) => {
                    let since = (now - s) as f32;
                    if m.fade_out_ms > 0. { 1. - (since / m.fade_out_ms).min(1.) } else { 0. }
                }
                (None, false) if m.fade_out_ms > 0. => ((m.duration_ms - local) / m.fade_out_ms).clamp(0., 1.),
                _ => 1.,
            };
            let w = (pm.weight * fade_in * fade_out).clamp(0., 1.);
            for c in &m.curves {
                let p = rig.parameter(&c.parameter).unwrap();
                let v = sample_curve(c, t);
                let cur = values.get_mut(&c.parameter).unwrap();
                *cur = match m.blend {
                    MotionBlend::Override => *cur + (v - *cur) * w,
                    MotionBlend::Additive => *cur + (v - p.default) * w,
                };
            }
        }
        // Scripted overrides win over motions and physics.
        let scripted: Vec<(String, f32)> = self
            .state
            .overrides
            .iter()
            .map(|(id, o)| {
                let f = if o.duration_ms > 0. { ((now - o.start_ms) as f32 / o.duration_ms).clamp(0., 1.) } else { 1. };
                (id.clone(), o.from + (o.to - o.from) * smooth(f))
            })
            .collect();
        values.extend(scripted.iter().cloned());
        // Physics: inputs from the motion/script values, outputs replace them.
        let mut remaining = dt_ms / 1000.;
        let norm = |values: &BTreeMap<String, f32>, id: &str| {
            let p = rig.parameter(id).unwrap();
            let v = values[id];
            if v >= p.default {
                (v - p.default) / (p.max - p.default).max(1e-6)
            } else {
                (v - p.default) / (p.default - p.min).max(1e-6)
            }
        };
        while remaining > 1e-6 {
            let h = remaining.min(PHYSICS_STEP);
            remaining -= h;
            for (i, pd) in rig.physics.iter().enumerate() {
                let input: f32 = pd.inputs.iter().map(|x| x.weight * norm(&values, &x.parameter)).sum();
                let target = input.clamp(-1., 1.) * pd.max_angle;
                let [a, v] = self.state.physics[i];
                let acc = pd.stiffness * (target - a) - pd.damping * v;
                let v = v + acc * h;
                self.state.physics[i] = [a + v * h, v];
            }
        }
        for (i, pd) in rig.physics.iter().enumerate() {
            let p = rig.parameter(&pd.output).unwrap();
            let s = (self.state.physics[i][0] / pd.max_angle).clamp(-1., 1.);
            let v = if s >= 0. { p.default + s * (p.max - p.default) } else { p.default + s * (p.default - p.min) };
            values.insert(pd.output.clone(), v);
        }
        values.extend(scripted);
        for p in &rig.parameters {
            let v = values.get_mut(&p.id).unwrap();
            *v = v.clamp(p.min, p.max);
        }
        self.state.values = values;
        Ok(())
    }

    fn deformer_matrices(&self) -> BTreeMap<&str, Affine> {
        let mut local: BTreeMap<&str, Affine> = BTreeMap::new();
        for d in &self.rig.deformers {
            let (mut rot, mut tr, mut sc) = (0f32, [0f32; 2], [1f32; 2]);
            for k in &d.keyforms {
                let v = self.state.values[&k.parameter];
                let (i, j, f) = segment(k.keys.iter().map(|k| k.value), v);
                let (a, b) = (&k.keys[i], &k.keys[j]);
                rot += a.rotation + (b.rotation - a.rotation) * f;
                for c in 0..2 {
                    tr[c] += a.translation[c] + (b.translation[c] - a.translation[c]) * f;
                    sc[c] *= a.scale[c] + (b.scale[c] - a.scale[c]) * f;
                }
            }
            let (s, c) = rot.to_radians().sin_cos();
            let p = d.pivot;
            // T(p + tr) · R · S · T(-p)
            let rs: Affine = [c * sc[0], s * sc[0], -s * sc[1], c * sc[1], 0., 0.];
            let m = mul(&[1., 0., 0., 1., p[0] + tr[0], p[1] + tr[1]], &mul(&rs, &[1., 0., 0., 1., -p[0], -p[1]]));
            local.insert(d.id.as_str(), m);
        }
        let mut world = BTreeMap::new();
        for d in &self.rig.deformers {
            let mut m = local[d.id.as_str()];
            let mut cur = d.parent.as_deref();
            while let Some(p) = cur {
                m = mul(&local[p], &m);
                cur = self.rig.deformers.iter().find(|x| x.id == p).unwrap().parent.as_deref();
            }
            world.insert(d.id.as_str(), m);
        }
        world
    }

    fn part_key(&self, part: &Part) -> (Vec<[f32; 2]>, f32, i32) {
        let mut pos = part.positions.clone();
        let (mut opacity, mut order) = (part.opacity, part.order);
        for k in &part.keyforms {
            let v = self.state.values[&k.parameter];
            let (i, j, f) = segment(k.keys.iter().map(|k| k.value), v);
            let (a, b): (&PartKey, &PartKey) = (&k.keys[i], &k.keys[j]);
            for (n, p) in pos.iter_mut().enumerate() {
                let oa = a.offsets.get(n).copied().unwrap_or([0.; 2]);
                let ob = b.offsets.get(n).copied().unwrap_or([0.; 2]);
                p[0] += oa[0] + (ob[0] - oa[0]) * f;
                p[1] += oa[1] + (ob[1] - oa[1]) * f;
            }
            opacity *= a.opacity + (b.opacity - a.opacity) * f;
            order += if f < 0.5 { a.order } else { b.order };
        }
        (pos, opacity.clamp(0., 1.), order)
    }

    /// Meshes of the current state, in draw order.
    pub fn frame(&self) -> PuppetFrame {
        let deformers = self.deformer_matrices();
        let used_as_mask: std::collections::BTreeSet<&str> =
            self.rig.parts.iter().flat_map(|p| p.masks.iter().map(String::as_str)).collect();
        let mut out: Vec<(i32, usize, FramePart)> = Vec::new();
        for (index, part) in self.rig.parts.iter().enumerate() {
            let (mut pos, opacity, order) = self.part_key(part);
            let shown = part.visible && opacity > 0.;
            if !shown && !used_as_mask.contains(part.id.as_str()) {
                continue;
            }
            let m = part.deformer.as_deref().map_or(IDENTITY, |d| deformers[d]);
            for p in &mut pos {
                *p = apply(&m, *p);
            }
            let colors = (0..pos.len())
                .map(|i| {
                    let c = part.colors.get(i).copied().unwrap_or([1.; 4]);
                    [c[0], c[1], c[2], c[3] * opacity]
                })
                .collect();
            out.push((order, index, FramePart {
                id: part.id.clone(),
                texture: part.texture,
                positions: pos,
                uvs: part.uvs.clone(),
                indices: part.indices.clone(),
                colors,
                blend: part.blend,
                masks: Vec::new(),
                invert_mask: part.invert_mask,
                visible: shown,
            }));
        }
        out.sort_by_key(|(order, index, _)| (*order, *index));
        let ids: BTreeMap<String, usize> = out.iter().enumerate().map(|(i, (_, _, p))| (p.id.clone(), i)).collect();
        for (_, index, fp) in &mut out {
            fp.masks = self.rig.parts[*index].masks.iter().filter_map(|m| ids.get(m).copied()).collect();
        }
        PuppetFrame { size: self.rig.size, parts: out.into_iter().map(|(_, _, p)| p).collect(), textures: self.rig.textures.clone() }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::*;

    /// A 100×100 face with a head deformer turned by `angle`, an eye whose
    /// shape follows `eye_open`, a mask, a breathing motion and hair physics.
    pub(crate) fn test_rig() -> PuppetRig {
        let quad = |id: &str, x: f32, y: f32, w: f32, h: f32| Part {
            id: id.into(),
            texture: 0,
            positions: vec![[x, y], [x + w, y], [x, y + h], [x + w, y + h]],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
            indices: vec![0, 1, 2, 2, 1, 3],
            colors: vec![],
            opacity: 1.,
            order: 0,
            blend: PartBlend::Normal,
            deformer: Some("head".into()),
            masks: vec![],
            invert_mask: false,
            visible: true,
            keyforms: vec![],
        };
        let mut eye = quad("eye", 40., 40., 20., 10.);
        eye.order = 2;
        eye.masks = vec!["eye_white".into()];
        eye.keyforms = vec![PartKeyform {
            parameter: "eye_open".into(),
            keys: vec![
                PartKey { value: 0., offsets: vec![[0., 5.], [0., 5.], [0., -5.], [0., -5.]], opacity: 1., order: 0 },
                PartKey { value: 1., offsets: vec![], opacity: 1., order: 0 },
            ],
        }];
        let mut white = quad("eye_white", 38., 38., 24., 14.);
        white.order = 1;
        white.visible = false;
        let mut hair = quad("hair", 20., 0., 60., 30.);
        hair.order = 3;
        hair.keyforms = vec![PartKeyform {
            parameter: "hair_sway".into(),
            keys: vec![
                PartKey { value: -1., offsets: vec![[-10., 0.]; 4], opacity: 1., order: 0 },
                PartKey { value: 1., offsets: vec![[10., 0.]; 4], opacity: 1., order: 0 },
            ],
        }];
        PuppetRig {
            format: FORMAT.into(),
            version: VERSION,
            size: [100., 100.],
            textures: vec!["face.png".into()],
            parameters: vec![
                Parameter { id: "angle".into(), min: -30., max: 30., default: 0. },
                Parameter { id: "eye_open".into(), min: 0., max: 1., default: 1. },
                Parameter { id: "breath".into(), min: 0., max: 1., default: 0. },
                Parameter { id: "hair_sway".into(), min: -1., max: 1., default: 0. },
            ],
            deformers: vec![Deformer {
                id: "head".into(),
                parent: None,
                pivot: [50., 90.],
                keyforms: vec![
                    DeformerKeyform {
                        parameter: "angle".into(),
                        keys: vec![
                            DeformerKey { value: -30., rotation: -15., translation: [0., 0.], scale: [1., 1.] },
                            DeformerKey { value: 30., rotation: 15., translation: [0., 0.], scale: [1., 1.] },
                        ],
                    },
                    DeformerKeyform {
                        parameter: "breath".into(),
                        keys: vec![
                            DeformerKey { value: 0., rotation: 0., translation: [0., 0.], scale: [1., 1.] },
                            DeformerKey { value: 1., rotation: 0., translation: [0., -2.], scale: [1., 1.] },
                        ],
                    },
                ],
            }],
            parts: vec![quad("face", 10., 10., 80., 80.), white, eye, hair],
            motions: vec![
                Motion {
                    id: "breathe".into(),
                    duration_ms: 2000.,
                    looping: true,
                    blend: MotionBlend::Override,
                    fade_in_ms: 0.,
                    fade_out_ms: 0.,
                    curves: vec![Curve { parameter: "breath".into(), keys: vec![[0., 0.], [1000., 1.], [2000., 0.]], interpolation: Interpolation::Linear }],
                },
                Motion {
                    id: "blink".into(),
                    duration_ms: 300.,
                    looping: false,
                    blend: MotionBlend::Override,
                    fade_in_ms: 0.,
                    fade_out_ms: 0.,
                    curves: vec![Curve { parameter: "eye_open".into(), keys: vec![[0., 1.], [150., 0.], [300., 1.]], interpolation: Interpolation::Linear }],
                },
            ],
            physics: vec![Pendulum {
                id: "hair".into(),
                inputs: vec![PendulumInput { parameter: "angle".into(), weight: 1. }],
                output: "hair_sway".into(),
                max_angle: 30.,
                stiffness: 60.,
                damping: 6.,
            }],
            provenance: None,
        }
    }

    fn part<'a>(f: &'a PuppetFrame, id: &str) -> &'a FramePart {
        f.parts.iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn rig_round_trips_and_rejects_broken_references() {
        let rig = test_rig();
        let json = serde_json::to_vec(&rig).unwrap();
        assert_eq!(PuppetRig::from_json(&json).unwrap(), rig);
        let mut bad = rig.clone();
        bad.parts[2].masks = vec!["nope".into()];
        assert!(bad.validate().is_err());
        let mut bad = rig.clone();
        bad.deformers.push(Deformer { id: "a".into(), parent: Some("b".into()), pivot: [0.; 2], keyforms: vec![] });
        bad.deformers.push(Deformer { id: "b".into(), parent: Some("a".into()), pivot: [0.; 2], keyforms: vec![] });
        assert!(bad.validate().unwrap_err().to_string().contains("cycle"));
        let mut bad = rig.clone();
        bad.textures = vec!["model.psb".into()];
        assert!(bad.validate().is_err());
        let mut bad = rig;
        bad.parts[0].keyforms.push(PartKeyform { parameter: "angle".into(), keys: vec![PartKey { value: 0., offsets: vec![[0.; 2]; 3], opacity: 1., order: 0 }] });
        assert!(bad.validate().is_err(), "offset count must match vertices");
    }

    #[test]
    fn deformers_keyforms_masks_and_order_evaluate() {
        let mut p = Puppet::new(Arc::new(test_rig())).unwrap();
        p.advance(0.).unwrap();
        let f = p.frame();
        let order: Vec<&str> = f.parts.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(order, ["face", "eye_white", "eye", "hair"]);
        assert!(!part(&f, "eye_white").visible, "mask-only part is drawn as a mask");
        assert_eq!(part(&f, "eye").masks, vec![1]);
        assert_eq!(part(&f, "face").positions[0], [10., 10.]);
        // Head turn: rotate 15° about (50, 90).
        p.set_parameter("angle", 30., 0.).unwrap();
        p.advance(0.).unwrap();
        let q = part(&p.frame(), "face").positions[0];
        let (s, c) = 15f32.to_radians().sin_cos();
        let e = [50. + (-40.) * c - (-80.) * s, 90. + (-40.) * s + (-80.) * c];
        assert!((q[0] - e[0]).abs() < 1e-3 && (q[1] - e[1]).abs() < 1e-3, "{q:?} vs {e:?}");
        // Closing the eye collapses it vertically.
        p.set_parameter("angle", 0., 0.).unwrap();
        p.set_parameter("eye_open", 0., 0.).unwrap();
        p.advance(0.).unwrap();
        let f = p.frame();
        let eye = &part(&f, "eye").positions;
        assert_eq!((eye[0][1], eye[2][1]), (45., 45.));
    }

    #[test]
    fn motions_loop_fade_and_scripted_values_override_them() {
        let mut p = Puppet::new(Arc::new(test_rig())).unwrap();
        p.play("breathe", None, 1.).unwrap();
        p.advance(500.).unwrap();
        assert!((p.value("breath").unwrap() - 0.5).abs() < 1e-4);
        p.advance(2000.).unwrap(); // 2500 ms → loop time 500
        assert!((p.value("breath").unwrap() - 0.5).abs() < 1e-4);
        p.set_parameter("breath", 1., 100.).unwrap();
        p.advance(50.).unwrap();
        let mid = p.value("breath").unwrap();
        assert!(mid > 0.5 && mid < 1., "eased override {mid}");
        p.advance(100.).unwrap();
        assert_eq!(p.value("breath"), Some(1.));
        p.release_parameter("breath").unwrap();
        p.advance(0.).unwrap();
        assert!(p.value("breath").unwrap() < 1.);
        // A non-looping blink ends and returns the eye to its default.
        p.play("blink", None, 1.).unwrap();
        p.advance(150.).unwrap();
        assert!(p.value("eye_open").unwrap() < 0.01);
        p.advance(400.).unwrap();
        assert_eq!(p.playing(), vec!["breathe"]);
        assert_eq!(p.value("eye_open"), Some(1.));
        p.stop("").unwrap();
        p.advance(1.).unwrap();
        assert!(p.playing().is_empty());
        assert!(p.play("missing", None, 1.).is_err() && p.set_parameter("missing", 0., 0.).is_err());
    }

    #[test]
    fn pendulum_swings_after_a_head_turn_and_settles() {
        let mut p = Puppet::new(Arc::new(test_rig())).unwrap();
        p.set_parameter("angle", 30., 0.).unwrap();
        let mut peak = 0f32;
        for _ in 0..30 {
            p.advance(16.).unwrap();
            peak = peak.max(p.value("hair_sway").unwrap());
        }
        assert!(peak > 0.9, "the hair follows the head: {peak}");
        for _ in 0..300 {
            p.advance(16.).unwrap();
        }
        assert!((p.value("hair_sway").unwrap() - 1.).abs() < 0.02, "settles at the input");
        // State round trip.
        let saved = serde_json::to_string(p.state()).unwrap();
        let restored = Puppet::with_state(p.rig().clone(), serde_json::from_str(&saved).unwrap()).unwrap();
        assert_eq!(restored.frame(), p.frame());
    }
}
