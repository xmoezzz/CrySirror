// SPDX-License-Identifier: MPL-2.0
//! Level sequences (ADR 0040): keyed timelines that move the camera,
//! entity transforms and light properties, with procedural flicker noise
//! and named events. The UE "Level Sequence" / Unity "Timeline" role.
//!
//! Playback is world state: it advances on the fixed world step, pauses
//! with its client and is saved, so a restored game resumes mid-sequence.
use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};

pub const MAX_TRACKS: usize = 512;
pub const MAX_KEYS: usize = 4096;
pub const MAX_SEQUENCES_PER_CLIENT: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyInterpolation {
    /// Piecewise linear.
    Linear,
    Step,
    /// Smoothstep between keys (ease in/out at every key).
    Smooth,
    /// Catmull-Rom spline through the keys (non-uniform key times).
    #[default]
    Spline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackProperty {
    /// [x, y, z] world units (local to the parent).
    Translation,
    /// [x, y, z] Euler degrees (XYZ order).
    Rotation,
    /// [x, y, z].
    Scale,
    /// [value] in the light's units.
    LightIntensity,
    /// [r, g, b] linear.
    LightColor,
    /// [value] attenuation radius.
    LightRange,
    /// [value]; > 0.5 is visible.
    Visible,
}
impl TrackProperty {
    pub fn arity(self) -> usize {
        match self {
            Self::Translation | Self::Rotation | Self::Scale | Self::LightColor => 3,
            Self::LightIntensity | Self::LightRange | Self::Visible => 1,
        }
    }
    pub fn needs_light(self) -> bool {
        matches!(
            self,
            Self::LightIntensity | Self::LightColor | Self::LightRange
        )
    }
}

/// Smooth value noise added to a track (fire flicker, sway, shake).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackNoise {
    /// Peak offset added to every component.
    pub amplitude: f32,
    /// Changes per second.
    pub frequency_hz: f32,
    #[serde(default)]
    pub seed: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceKey {
    pub time_ms: f32,
    pub value: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// Client-local entity id.
    pub entity: u32,
    pub property: TrackProperty,
    pub keys: Vec<SequenceKey>,
    #[serde(default)]
    pub interpolation: KeyInterpolation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<TrackNoise>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraKey {
    pub time_ms: f32,
    pub eye: [f32; 3],
    pub target: [f32; 3],
    /// Default: the camera's current field of view.
    #[serde(default)]
    pub fov_y_degrees: Option<f32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraTrack {
    pub keys: Vec<CameraKey>,
    #[serde(default)]
    pub interpolation: KeyInterpolation,
    /// Camera shake added to eye and target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shake: Option<TrackNoise>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceEvent {
    pub time_ms: f32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceSpec {
    pub duration_ms: f32,
    #[serde(default)]
    pub looping: bool,
    /// Playback rate (1 = real time, 0 = hold).
    #[serde(default = "one")]
    pub speed: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<CameraTrack>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub events: Vec<SequenceEvent>,
}
fn one() -> f32 {
    1.
}

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}
fn check_times(times: impl Iterator<Item = f32>, duration: f32, what: &str) -> Result<usize> {
    let t: Vec<f32> = times.collect();
    ensure!(
        !t.is_empty() && t.len() <= MAX_KEYS,
        "{what}: 1..={MAX_KEYS} keys"
    );
    ensure!(
        finite(&t) && t.windows(2).all(|w| w[0] < w[1]),
        "{what}: key times must increase"
    );
    ensure!(
        t[0] >= 0. && t[t.len() - 1] <= duration,
        "{what}: keys outside the sequence"
    );
    Ok(t.len())
}
fn check_noise(n: &TrackNoise) -> Result<()> {
    ensure!(
        finite(&[n.amplitude, n.frequency_hz])
            && n.amplitude.abs() <= 1e6
            && (0.0..=1000.).contains(&n.frequency_hz),
        "invalid noise"
    );
    Ok(())
}

impl SequenceSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.duration_ms.is_finite()
                && self.duration_ms > 0.
                && self.duration_ms <= 86_400_000.,
            "sequence duration"
        );
        ensure!(
            self.speed.is_finite() && (0.0..=16.).contains(&self.speed),
            "sequence speed must be 0..16"
        );
        ensure!(
            self.tracks.len() <= MAX_TRACKS && self.events.len() <= MAX_KEYS,
            "sequence too large"
        );
        if let Some(c) = &self.camera {
            check_times(
                c.keys.iter().map(|k| k.time_ms),
                self.duration_ms,
                "camera track",
            )?;
            for k in &c.keys {
                ensure!(
                    finite(&k.eye) && finite(&k.target) && k.eye != k.target,
                    "camera key"
                );
                if let Some(f) = k.fov_y_degrees {
                    ensure!(f.is_finite() && (1.0..=179.).contains(&f), "camera key fov");
                }
            }
            if let Some(n) = &c.shake {
                check_noise(n)?;
            }
        }
        for t in &self.tracks {
            check_times(t.keys.iter().map(|k| k.time_ms), self.duration_ms, "track")?;
            ensure!(
                t.keys.iter().all(|k| k.value.len() == t.property.arity()
                    && finite(&k.value)
                    && k.value.iter().all(|v| v.abs() <= 1e7)),
                "track {:?} keys need {} finite values",
                t.property,
                t.property.arity()
            );
            if t.property == TrackProperty::Scale {
                ensure!(
                    t.keys
                        .iter()
                        .all(|k| k.value.iter().all(|v| v.abs() > 1e-6)),
                    "scale keys must be non-zero"
                );
            }
            if let Some(n) = &t.noise {
                check_noise(n)?;
            }
        }
        for e in &self.events {
            ensure!(
                e.time_ms.is_finite()
                    && (0.0..=self.duration_ms).contains(&e.time_ms)
                    && !e.name.is_empty()
                    && e.name.len() <= 128,
                "sequence event"
            );
        }
        Ok(())
    }
}

/// A playing (or finished) sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequencePlayback {
    pub spec: SequenceSpec,
    pub time_ms: f32,
    pub playing: bool,
    /// Completed loops.
    #[serde(default)]
    pub loops: u32,
    /// Events fired since the last read (bounded).
    #[serde(default)]
    pub fired: Vec<String>,
}

fn hash(seed: u32, i: i64) -> f32 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (seed as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 24) as f32 * 2. - 1.
}
/// Smooth value noise in [-1, 1].
pub fn noise(seed: u32, t: f32) -> f32 {
    let i = t.floor();
    let f = t - i;
    let s = f * f * (3. - 2. * f);
    let (a, b) = (hash(seed, i as i64), hash(seed, i as i64 + 1));
    a + (b - a) * s
}

/// Value of keyed data at `t` (component-wise).
pub fn sample(keys: &[(f32, &[f32])], t: f32, mode: KeyInterpolation) -> Vec<f32> {
    let n = keys.len();
    if n == 1 || t <= keys[0].0 {
        return keys[0].1.to_vec();
    }
    if t >= keys[n - 1].0 {
        return keys[n - 1].1.to_vec();
    }
    let i = keys.partition_point(|k| k.0 <= t) - 1;
    let (t0, a) = keys[i];
    let (t1, b) = keys[i + 1];
    let u = (t - t0) / (t1 - t0);
    match mode {
        KeyInterpolation::Step => a.to_vec(),
        KeyInterpolation::Linear => a.iter().zip(b).map(|(a, b)| a + (b - a) * u).collect(),
        KeyInterpolation::Smooth => {
            let s = u * u * (3. - 2. * u);
            a.iter().zip(b).map(|(a, b)| a + (b - a) * s).collect()
        }
        KeyInterpolation::Spline => {
            // Cubic Hermite with Catmull-Rom tangents for uneven spacing.
            let tangent = |j: usize, c: usize| {
                let (lo, hi) = (j.saturating_sub(1), (j + 1).min(n - 1));
                (keys[hi].1[c] - keys[lo].1[c]) / (keys[hi].0 - keys[lo].0) * (t1 - t0)
            };
            let (u2, u3) = (u * u, u * u * u);
            (0..a.len())
                .map(|c| {
                    let (m0, m1) = (tangent(i, c), tangent(i + 1, c));
                    (2. * u3 - 3. * u2 + 1.) * a[c]
                        + (u3 - 2. * u2 + u) * m0
                        + (-2. * u3 + 3. * u2) * b[c]
                        + (u3 - u2) * m1
                })
                .collect()
        }
    }
}

impl Track {
    pub fn value_at(&self, t: f32) -> Vec<f32> {
        let keys: Vec<(f32, &[f32])> = self
            .keys
            .iter()
            .map(|k| (k.time_ms, k.value.as_slice()))
            .collect();
        let mut v = sample(&keys, t, self.interpolation);
        if let Some(n) = &self.noise {
            for (c, x) in v.iter_mut().enumerate() {
                *x += n.amplitude
                    * noise(
                        n.seed.wrapping_add(c as u32 * 7919),
                        t / 1000. * n.frequency_hz,
                    );
            }
        }
        v
    }
}
impl CameraTrack {
    /// (eye, target, fov) at `t`; `fov` is None when no key sets one.
    pub fn value_at(&self, t: f32) -> ([f32; 3], [f32; 3], Option<f32>) {
        let flat: Vec<Vec<f32>> = self
            .keys
            .iter()
            .map(|k| [k.eye.as_slice(), k.target.as_slice()].concat())
            .collect();
        let keys: Vec<(f32, &[f32])> = self
            .keys
            .iter()
            .zip(&flat)
            .map(|(k, v)| (k.time_ms, v.as_slice()))
            .collect();
        let mut v = sample(&keys, t, self.interpolation);
        if let Some(n) = &self.shake {
            for (c, x) in v.iter_mut().enumerate() {
                *x += n.amplitude
                    * noise(
                        n.seed.wrapping_add(c as u32 * 7919),
                        t / 1000. * n.frequency_hz,
                    );
            }
        }
        let fovs: Vec<(f32, [f32; 1])> = self
            .keys
            .iter()
            .filter_map(|k| k.fov_y_degrees.map(|f| (k.time_ms, [f])))
            .collect();
        let fov = (!fovs.is_empty()).then(|| {
            let keys: Vec<(f32, &[f32])> = fovs.iter().map(|(t, f)| (*t, f.as_slice())).collect();
            sample(&keys, t, KeyInterpolation::Smooth)[0]
        });
        ([v[0], v[1], v[2]], [v[3], v[4], v[5]], fov)
    }
}

impl SequencePlayback {
    pub fn new(spec: SequenceSpec, start_ms: f32) -> Result<Self> {
        spec.validate()?;
        if !(start_ms.is_finite() && (0.0..=spec.duration_ms).contains(&start_ms)) {
            bail!("sequence start outside 0..duration");
        }
        Ok(Self {
            spec,
            time_ms: start_ms,
            playing: true,
            loops: 0,
            fired: Vec::new(),
        })
    }
    /// Advances by `dt_ms` of world time, firing events crossed on the way.
    pub fn advance(&mut self, dt_ms: f32) {
        if !self.playing {
            return;
        }
        let d = self.spec.duration_ms;
        let mut from = self.time_ms;
        let mut left = dt_ms * self.spec.speed;
        // Event times in (from, to]; a looping sequence may wrap several times.
        while left > 0. {
            let to = from + left;
            let end = to.min(d);
            for e in &self.spec.events {
                if e.time_ms > from && e.time_ms <= end && self.fired.len() < 256 {
                    self.fired.push(e.name.clone());
                }
            }
            if to < d {
                self.time_ms = to;
                break;
            }
            left = to - d;
            if self.spec.looping {
                self.loops += 1;
                from = 0.;
                self.time_ms = 0.;
                // Events at t = 0 fire at the start of every loop.
                for e in self.spec.events.iter().filter(|e| e.time_ms == 0.) {
                    if self.fired.len() < 256 {
                        self.fired.push(e.name.clone());
                    }
                }
                if left <= 0. {
                    break;
                }
            } else {
                self.time_ms = d;
                self.playing = false;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(t: f32, v: &[f32]) -> SequenceKey {
        SequenceKey {
            time_ms: t,
            value: v.to_vec(),
        }
    }
    #[test]
    fn interpolation_modes_hit_keys_and_differ_between_them() {
        let keys = [(0., &[0f32][..]), (1000., &[10.][..]), (2000., &[0.][..])];
        for m in [
            KeyInterpolation::Linear,
            KeyInterpolation::Step,
            KeyInterpolation::Smooth,
            KeyInterpolation::Spline,
        ] {
            assert_eq!(sample(&keys, 1000., m), vec![10.]);
            assert_eq!(sample(&keys, -5., m), vec![0.]);
            assert_eq!(sample(&keys, 3000., m), vec![0.]);
        }
        assert_eq!(sample(&keys, 500., KeyInterpolation::Linear), vec![5.]);
        assert_eq!(sample(&keys, 500., KeyInterpolation::Step), vec![0.]);
        // The spline overshoots linear on the way up (tangent 0 at the top,
        // 10/2000·1000 = 5 at the start).
        let s = sample(&keys, 250., KeyInterpolation::Spline)[0];
        assert!(s > 0. && s < 10., "{s}");
    }
    #[test]
    fn noise_is_smooth_deterministic_and_bounded() {
        let a: Vec<f32> = (0..1000).map(|i| noise(3, i as f32 * 0.01)).collect();
        assert!(a.iter().all(|v| (-1.0..=1.0).contains(v)));
        assert!(
            a.windows(2).all(|w| (w[0] - w[1]).abs() < 0.05),
            "continuous"
        );
        assert_eq!(noise(3, 1.234), noise(3, 1.234));
        assert_ne!(noise(3, 1.234), noise(4, 1.234));
    }
    #[test]
    fn playback_fires_events_loops_and_stops() {
        let spec = SequenceSpec {
            duration_ms: 1000.,
            looping: false,
            speed: 1.,
            camera: None,
            tracks: vec![Track {
                entity: 1,
                property: TrackProperty::Translation,
                keys: vec![key(0., &[0., 0., 0.]), key(1000., &[1., 2., 3.])],
                interpolation: KeyInterpolation::Linear,
                noise: None,
            }],
            events: vec![
                SequenceEvent {
                    time_ms: 500.,
                    name: "half".into(),
                },
                SequenceEvent {
                    time_ms: 1000.,
                    name: "end".into(),
                },
            ],
        };
        let mut p = SequencePlayback::new(spec.clone(), 0.).unwrap();
        p.advance(400.);
        assert!(p.fired.is_empty());
        p.advance(200.);
        assert_eq!(p.fired, vec!["half"]);
        p.advance(1000.);
        assert_eq!(p.fired, vec!["half", "end"]);
        assert!(!p.playing && p.time_ms == 1000.);
        let mut l = SequencePlayback::new(
            SequenceSpec {
                looping: true,
                ..spec.clone()
            },
            0.,
        )
        .unwrap();
        l.advance(2600.);
        assert_eq!(l.loops, 2);
        assert!((l.time_ms - 600.).abs() < 1e-3);
        assert_eq!(l.fired, vec!["half", "end", "half", "end", "half"]);
        // Validation.
        let mut bad = spec.clone();
        bad.tracks[0].keys[1].value = vec![1.];
        assert!(bad.validate().is_err());
        let mut bad = spec;
        bad.tracks[0].keys[1].time_ms = 2000.;
        assert!(bad.validate().is_err());
    }
}
