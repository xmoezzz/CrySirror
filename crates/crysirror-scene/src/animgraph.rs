// SPDX-License-Identifier: MPL-2.0
//! Animation state machines, blend spaces and montages (ADR 0014).
//! Definitions live in the world; instances are part of the entity's
//! AnimationState and resolve every fixed step to weighted clip layers, which
//! the renderer already skins. Notifies become events for gameplay.
use crate::components::AnimationLayer;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn one() -> f32 {
    1.
}
fn yes() -> bool {
    true
}
fn fade() -> f32 {
    0.2
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample1 {
    pub at: f32,
    pub clip: String,
    /// Playback rate of this sample (e.g. walk 1.0, run 1.2).
    #[serde(default = "one")]
    pub speed: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample2 {
    pub at: [f32; 2],
    pub clip: String,
    #[serde(default = "one")]
    pub speed: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Motion {
    Clip {
        clip: String,
        #[serde(default = "one")]
        speed: f32,
        #[serde(default = "yes")]
        looping: bool,
    },
    /// 1D blend space: linear blend between the two samples around `param`.
    Blend1d {
        param: String,
        samples: Vec<Sample1>,
    },
    /// 2D blend space: inverse-distance weighting over samples (gradient-band
    /// style falloff; exact on samples).
    Blend2d {
        params: [String; 2],
        samples: Vec<Sample2>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CondOp {
    Greater,
    Less,
    Equal,
    True,
    False,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub param: String,
    pub op: CondOp,
    #[serde(default)]
    pub value: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    /// Source state, or "*" for any state.
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Only after the source state has run this long (seconds).
    #[serde(default)]
    pub min_time: f32,
    #[serde(default = "fade")]
    pub fade: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateMachineDef {
    pub entry: String,
    pub states: BTreeMap<String, Motion>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
    /// Parameter defaults (bools are 0/1).
    #[serde(default)]
    pub parameters: BTreeMap<String, f32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notify {
    pub time: f32,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MontageDef {
    pub clip: String,
    /// Clip length in seconds (from modelInfo).
    pub length: f32,
    #[serde(default = "one")]
    pub rate: f32,
    #[serde(default = "fade")]
    pub blend_in: f32,
    #[serde(default = "fade")]
    pub blend_out: f32,
    /// Named start times (combo windows: "Attack1", "Attack2", ...).
    #[serde(default)]
    pub sections: BTreeMap<String, f32>,
    #[serde(default)]
    pub notifies: Vec<Notify>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimDefs {
    #[serde(default)]
    pub machines: BTreeMap<String, StateMachineDef>,
    #[serde(default)]
    pub montages: BTreeMap<String, MontageDef>,
}
impl AnimDefs {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.machines.len() <= 1024 && self.montages.len() <= 4096,
            "animation definition limit"
        );
        for (n, m) in &self.machines {
            ensure!(
                m.states.contains_key(&m.entry),
                "machine '{n}': entry state missing"
            );
            ensure!(
                m.states.len() <= 256 && m.transitions.len() <= 1024,
                "machine '{n}' size"
            );
            for t in &m.transitions {
                ensure!(
                    (t.from == "*" || m.states.contains_key(&t.from))
                        && m.states.contains_key(&t.to),
                    "machine '{n}': transition {}→{}",
                    t.from,
                    t.to
                );
                ensure!(
                    t.fade.is_finite() && t.fade >= 0. && t.min_time.is_finite(),
                    "machine '{n}' transition timing"
                );
            }
            for (s, motion) in &m.states {
                match motion {
                    Motion::Blend1d { samples, .. } => ensure!(
                        !samples.is_empty() && samples.len() <= 32,
                        "machine '{n}' state '{s}' samples"
                    ),
                    Motion::Blend2d { samples, .. } => ensure!(
                        !samples.is_empty() && samples.len() <= 32,
                        "machine '{n}' state '{s}' samples"
                    ),
                    Motion::Clip { .. } => {}
                }
            }
        }
        for (n, m) in &self.montages {
            ensure!(
                m.length.is_finite() && m.length > 0. && m.rate.is_finite() && m.rate > 0.,
                "montage '{n}' timing"
            );
            ensure!(
                m.sections
                    .values()
                    .chain(m.notifies.iter().map(|x| &x.time))
                    .all(|t| (0.0..=m.length).contains(t)),
                "montage '{n}' times"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineInstance {
    pub machine: String,
    pub state: String,
    pub state_time: f32,
    /// Phase (seconds) shared by blend-space samples of the current state.
    pub phase: f32,
    /// Previous state being faded out: (state, its time, its phase, fade progress 0..1, fade seconds).
    #[serde(default)]
    pub previous: Option<(String, f32, f32, f32, f32)>,
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MontageInstance {
    pub montage: String,
    pub time: f32,
    pub weight: f32,
    /// Blending out (stopped or reached the end).
    pub stopping: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimEvent {
    /// "notify", "state_entered", "montage_ended".
    pub kind: String,
    pub name: String,
}

fn motion_layers(
    m: &Motion,
    params: &BTreeMap<String, f32>,
    time: f32,
    phase: f32,
    weight: f32,
    out: &mut Vec<AnimationLayer>,
) {
    let mut push = |clip: &str, speed: f32, w: f32, looping: bool, t: f32| {
        if w > 1e-4 {
            out.push(AnimationLayer {
                clip: clip.into(),
                time: t * speed,
                speed: 0.,
                weight: w * weight,
                target_weight: w * weight,
                fade: 0.,
                looping,
            });
        }
    };
    match m {
        Motion::Clip {
            clip,
            speed,
            looping,
        } => push(clip, *speed, 1., *looping, time),
        Motion::Blend1d { param, samples } => {
            let x = params.get(param).copied().unwrap_or(0.);
            let mut s: Vec<&Sample1> = samples.iter().collect();
            s.sort_by(|a, b| a.at.total_cmp(&b.at));
            if x <= s[0].at {
                push(&s[0].clip, s[0].speed, 1., true, phase);
            } else if x >= s[s.len() - 1].at {
                let l = s[s.len() - 1];
                push(&l.clip, l.speed, 1., true, phase);
            } else {
                let i = s
                    .windows(2)
                    .position(|w| x >= w[0].at && x <= w[1].at)
                    .unwrap();
                let (a, b) = (s[i], s[i + 1]);
                let f = (x - a.at) / (b.at - a.at).max(1e-6);
                push(&a.clip, a.speed, 1. - f, true, phase);
                push(&b.clip, b.speed, f, true, phase);
            }
        }
        Motion::Blend2d { params: p, samples } => {
            let q = [
                params.get(&p[0]).copied().unwrap_or(0.),
                params.get(&p[1]).copied().unwrap_or(0.),
            ];
            if let Some(exact) = samples
                .iter()
                .find(|s| (s.at[0] - q[0]).abs() < 1e-5 && (s.at[1] - q[1]).abs() < 1e-5)
            {
                push(&exact.clip, exact.speed, 1., true, phase);
                return;
            }
            let w: Vec<f32> = samples
                .iter()
                .map(|s| 1. / ((s.at[0] - q[0]).powi(2) + (s.at[1] - q[1]).powi(2)).powi(2))
                .collect();
            let total: f32 = w.iter().sum();
            for (s, wi) in samples.iter().zip(w) {
                push(&s.clip, s.speed, wi / total, true, phase);
            }
        }
    }
}

impl MachineInstance {
    pub fn new(name: &str, def: &StateMachineDef) -> Self {
        Self {
            machine: name.into(),
            state: def.entry.clone(),
            state_time: 0.,
            phase: 0.,
            previous: None,
            params: def.parameters.clone(),
        }
    }
    fn holds(&self, c: &Condition) -> bool {
        let v = self.params.get(&c.param).copied().unwrap_or(0.);
        match c.op {
            CondOp::Greater => v > c.value,
            CondOp::Less => v < c.value,
            CondOp::Equal => (v - c.value).abs() < 1e-5,
            CondOp::True => v != 0.,
            CondOp::False => v == 0.,
        }
    }
    /// Advance and transition; returns entered state, if any.
    pub fn advance(&mut self, def: &StateMachineDef, dt: f32) -> Option<String> {
        self.state_time += dt;
        self.phase += dt;
        if let Some(p) = &mut self.previous {
            p.1 += dt;
            p.2 += dt;
            p.3 = if p.4 > 0. {
                (p.3 + dt / p.4).min(1.)
            } else {
                1.
            };
            if p.3 >= 1. {
                self.previous = None;
            }
        }
        let t = def.transitions.iter().find(|t| {
            (t.from == "*" || t.from == self.state)
                && t.to != self.state
                && self.state_time >= t.min_time
                && t.conditions.iter().all(|c| self.holds(c))
        })?;
        let to = t.to.clone();
        // Locomotion blend spaces keep phase across sibling states (foot sync).
        let keep_phase = matches!(
            def.states.get(&to),
            Some(Motion::Blend1d { .. } | Motion::Blend2d { .. })
        ) && matches!(
            def.states.get(&self.state),
            Some(Motion::Blend1d { .. } | Motion::Blend2d { .. })
        );
        self.previous = Some((
            std::mem::replace(&mut self.state, to.clone()),
            self.state_time,
            self.phase,
            0.,
            t.fade,
        ));
        self.state_time = 0.;
        if !keep_phase {
            self.phase = 0.;
        }
        Some(to)
    }
    pub fn layers(&self, def: &StateMachineDef) -> Vec<AnimationLayer> {
        let mut out = Vec::new();
        let alpha = self.previous.as_ref().map(|p| p.3).unwrap_or(1.);
        if let Some(m) = def.states.get(&self.state) {
            motion_layers(
                m,
                &self.params,
                self.state_time,
                self.phase,
                alpha,
                &mut out,
            );
        }
        if let Some((s, t, ph, a, _)) = &self.previous {
            if let Some(m) = def.states.get(s) {
                motion_layers(m, &self.params, *t, *ph, 1. - a, &mut out);
            }
        }
        out
    }
}

/// Advance a montage; collects crossed notifies. Returns false when finished.
pub fn advance_montage(
    m: &mut MontageInstance,
    def: &MontageDef,
    dt: f32,
    events: &mut Vec<AnimEvent>,
) -> bool {
    let before = m.time;
    m.time += dt * def.rate;
    for n in &def.notifies {
        if n.time > before && n.time <= m.time || (before == 0. && n.time == 0.) {
            events.push(AnimEvent {
                kind: "notify".into(),
                name: n.name.clone(),
            });
        }
    }
    if m.time >= def.length - def.blend_out * def.rate {
        m.stopping = true;
    }
    if m.stopping {
        m.weight -= dt / def.blend_out.max(1e-3);
    } else {
        m.weight = (m.weight + dt / def.blend_in.max(1e-3)).min(1.);
    }
    if m.weight <= 0. || m.time >= def.length {
        events.push(AnimEvent {
            kind: "montage_ended".into(),
            name: m.montage.clone(),
        });
        return false;
    }
    true
}

/// Resolve a machine + montage into final layers: the montage is a full-body
/// slot blended over the machine output.
pub fn compose(
    defs: &AnimDefs,
    machine: Option<&MachineInstance>,
    montage: Option<&MontageInstance>,
) -> Result<Vec<AnimationLayer>> {
    let mut layers = match machine {
        Some(mi) => mi.layers(
            defs.machines
                .get(&mi.machine)
                .with_context(|| format!("unknown state machine '{}'", mi.machine))?,
        ),
        None => Vec::new(),
    };
    if let Some(m) = montage {
        let def = defs
            .montages
            .get(&m.montage)
            .with_context(|| format!("unknown montage '{}'", m.montage))?;
        let w = m.weight.clamp(0., 1.);
        for l in &mut layers {
            l.weight *= 1. - w;
            l.target_weight = l.weight;
        }
        layers.push(AnimationLayer {
            clip: def.clip.clone(),
            time: m.time,
            speed: 0.,
            weight: w.max(1e-3),
            target_weight: w.max(1e-3),
            fade: 0.,
            looping: false,
        });
    }
    layers.retain(|l| l.weight > 1e-4);
    // Keep the heaviest layers within the renderer's layer budget.
    layers.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    layers.truncate(crate::components::MAX_ANIMATION_LAYERS);
    Ok(layers)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn defs() -> AnimDefs {
        serde_json::from_value(serde_json::json!({
            "machines": {"Knight": {
                "entry": "Locomotion",
                "parameters": {"speed": 0, "dead": 0},
                "states": {
                    "Locomotion": {"kind": "blend1d", "param": "speed", "samples": [
                        {"at": 0, "clip": "Idle"}, {"at": 1.6, "clip": "Walk"}, {"at": 5, "clip": "Run", "speed": 1.1}]},
                    "Death": {"kind": "clip", "clip": "Die", "looping": false}
                },
                "transitions": [{"from": "*", "to": "Death", "conditions": [{"param": "dead", "op": "true"}], "fade": 0.1}]
            }},
            "montages": {"Slash": {"clip": "Attack", "length": 1.0, "blend_in": 0.1, "blend_out": 0.2,
                "sections": {"Combo2": 0.5}, "notifies": [{"time": 0.3, "name": "HitWindowOpen"}, {"time": 0.45, "name": "HitWindowClose"}]}}
        })).unwrap()
    }
    #[test]
    fn blendspace_transitions_and_montage_notifies() {
        let d = defs();
        d.validate().unwrap();
        let mdef = &d.machines["Knight"];
        let mut mi = MachineInstance::new("Knight", mdef);
        mi.params.insert("speed".into(), 3.3);
        mi.advance(mdef, 0.5);
        let l = mi.layers(mdef);
        let w: BTreeMap<_, _> = l.iter().map(|x| (x.clip.as_str(), x.weight)).collect();
        assert!(
            (w["Walk"] - 0.5).abs() < 1e-4 && (w["Run"] - 0.5).abs() < 1e-4,
            "{w:?}"
        );
        assert!(
            (l.iter().find(|x| x.clip == "Run").unwrap().time - 0.55).abs() < 1e-5,
            "phase-synced with sample speed"
        );
        mi.params.insert("dead".into(), 1.);
        assert_eq!(mi.advance(mdef, 0.016), Some("Death".into()));
        mi.advance(mdef, 0.05);
        let l = mi.layers(mdef);
        let death = l.iter().find(|x| x.clip == "Die").unwrap().weight;
        assert!((death - 0.5).abs() < 1e-3, "cross-fade halfway: {death}");
        mi.advance(mdef, 0.1);
        assert!(mi.previous.is_none());

        let mut m = MontageInstance {
            montage: "Slash".into(),
            time: 0.,
            weight: 0.,
            stopping: false,
        };
        let mut ev = Vec::new();
        for _ in 0..25 {
            assert!(advance_montage(
                &mut m,
                &d.montages["Slash"],
                0.016,
                &mut ev
            ));
        }
        assert_eq!(
            ev.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["HitWindowOpen"]
        );
        let layers = compose(&d, Some(&mi), Some(&m)).unwrap();
        assert_eq!(layers[0].clip, "Attack", "montage dominates at full weight");
        let mut alive = true;
        for _ in 0..80 {
            alive = advance_montage(&mut m, &d.montages["Slash"], 0.016, &mut ev);
            if !alive {
                break;
            }
        }
        assert!(
            !alive
                && ev.iter().any(|e| e.kind == "montage_ended")
                && ev.iter().any(|e| e.name == "HitWindowClose")
        );
    }
}
