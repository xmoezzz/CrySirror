// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Owner-scoped retained Layer/UI tracks, sampled by the shared ScalarTween.
use super::Easing;
use crate::animation::ScalarTween;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Logical property controlled by one retained two-dimensional motion track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionProperty {
    /// Local rectangle x coordinate in logical pixels.
    X,
    /// Local rectangle y coordinate in logical pixels.
    Y,
    /// Layer horizontal transform scale; UI widgets reject this property.
    ScaleX,
    /// Layer vertical transform scale; UI widgets reject this property.
    ScaleY,
}
impl MotionProperty {
    pub(crate) fn layer_value(self, layer: &super::LayerSpec) -> f32 {
        match self {
            Self::X => layer.rect[0],
            Self::Y => layer.rect[1],
            Self::ScaleX => layer.transform.scale_x,
            Self::ScaleY => layer.transform.scale_y,
        }
    }
    pub(crate) fn set_layer(self, layer: &mut super::LayerSpec, value: f32) {
        match self {
            Self::X => layer.rect[0] = value,
            Self::Y => layer.rect[1] = value,
            Self::ScaleX => layer.transform.scale_x = value,
            Self::ScaleY => layer.transform.scale_y = value,
        }
    }
    pub(crate) fn rect_value(self, rect: [f32; 4]) -> Result<f32> {
        match self {
            Self::X => Ok(rect[0]),
            Self::Y => Ok(rect[1]),
            _ => anyhow::bail!("UI motion only supports rectangle X/Y"),
        }
    }
    pub(crate) fn set_rect(self, rect: &mut [f32; 4], value: f32) {
        match self {
            Self::X => rect[0] = value,
            Self::Y => rect[1] = value,
            _ => unreachable!("validated UI motion property"),
        }
    }
}
/// One bounded scalar track. External time uses the caller's declared units;
/// automatic tracks use engine milliseconds. No host clock is sampled here.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionTrackSpec {
    /// Local position or Layer transform component to animate.
    pub property: MotionProperty,
    /// Explicit start, or the current property value when absent.
    #[serde(default)]
    pub from: Option<f32>,
    /// Finite final property value.
    pub to: f32,
    /// Duration in engine milliseconds or external caller units; zero is immediate.
    pub duration: u64,
    /// Require explicit monotonic SetMotionElapsed instead of engine ticking.
    #[serde(default)]
    pub external: bool,
    /// Shared scalar curve, applied to normalized progress.
    #[serde(default)]
    pub easing: Easing,
}
/// Read-only state of one engine-owned track.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionTrackStatus {
    /// Component owned by the track.
    pub property: MotionProperty,
    /// Resolved initial property value.
    pub from: f32,
    /// Requested final property value.
    pub to: f32,
    /// Current sampled value already applied to the target.
    pub current: f32,
    /// Duration in the track's declared clock units.
    pub duration: u64,
    /// Last elapsed value in the same units.
    pub elapsed: u64,
    /// Whether elapsed time is explicitly supplied by the caller.
    pub external: bool,
    /// True once the endpoint has been applied.
    pub complete: bool,
}
/// Snapshot of a target's retained tracks. An existing target without tracks
/// returns an empty list and complete=true.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionStatus {
    /// Active or completed tracks, in deterministic property order.
    pub tracks: Vec<MotionTrackStatus>,
    /// Every retained track has reached its endpoint.
    pub complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Track {
    curve: ScalarTween,
    external: bool,
    elapsed: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionTracks {
    by_owner: BTreeMap<u32, BTreeMap<u32, BTreeMap<MotionProperty, Track>>>,
}
fn endpoints(property: MotionProperty, from: f32, to: f32, scale: bool) -> Result<()> {
    ensure!(
        [from, to]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
        "motion endpoint bounds"
    );
    if matches!(property, MotionProperty::ScaleX | MotionProperty::ScaleY) {
        ensure!(scale, "UI motion only supports rectangle X/Y");
        ensure!(
            [from, to]
                .iter()
                .all(|v| v.abs() >= 0.0001 && v.abs() <= 4096.)
                && from.signum() == to.signum(),
            "motion cannot cross a singular transform scale"
        );
    }
    Ok(())
}
impl MotionTracks {
    pub(crate) fn begin(
        &mut self,
        owner: u32,
        id: u32,
        spec: &MotionTrackSpec,
        current: f32,
        scale: bool,
    ) -> Result<f32> {
        let from = spec.from.unwrap_or(current);
        endpoints(spec.property, from, spec.to, scale)?;
        ensure!(spec.duration <= 86_400_000, "motion duration limit");
        let replacing = self
            .by_owner
            .get(&owner)
            .and_then(|o| o.get(&id))
            .is_some_and(|t| t.contains_key(&spec.property));
        if !replacing {
            let count = |o: &BTreeMap<u32, BTreeMap<MotionProperty, Track>>| {
                o.values().map(BTreeMap::len).sum::<usize>()
            };
            ensure!(
                self.by_owner.get(&owner).map_or(0, count) < 4096
                    && self.by_owner.values().map(count).sum::<usize>() < 16384,
                "motion track limit"
            );
        }
        let track = Track {
            curve: ScalarTween::new(from, spec.to, 0, spec.duration, spec.easing)?,
            external: spec.external,
            elapsed: 0,
        };
        let value = track.curve.sample(0);
        self.by_owner
            .entry(owner)
            .or_default()
            .entry(id)
            .or_default()
            .insert(spec.property, track);
        Ok(value)
    }
    pub(crate) fn advance(
        &mut self,
        owner: u32,
        id: u32,
        property: MotionProperty,
        elapsed: u64,
    ) -> Result<f32> {
        let track = self
            .by_owner
            .get_mut(&owner)
            .and_then(|o| o.get_mut(&id))
            .and_then(|t| t.get_mut(&property))
            .context("motion track missing")?;
        ensure!(track.external, "motion uses automatic engine time");
        ensure!(
            elapsed >= track.elapsed,
            "motion elapsed cannot move backwards"
        );
        track.elapsed = elapsed;
        Ok(track.curve.sample(elapsed))
    }
    pub(crate) fn end(
        &mut self,
        owner: u32,
        id: u32,
        property: Option<MotionProperty>,
        finish: bool,
    ) -> Result<Vec<(MotionProperty, f32)>> {
        let mut values = Vec::new();
        if let Some(property) = property {
            let track = self
                .by_owner
                .get_mut(&owner)
                .and_then(|o| o.get_mut(&id))
                .and_then(|t| t.remove(&property))
                .context("motion track missing")?;
            if finish {
                values.push((property, track.curve.end));
            }
        } else if let Some(tracks) = self.by_owner.get_mut(&owner).and_then(|o| o.remove(&id)) {
            if finish {
                values.extend(tracks.into_iter().map(|(p, t)| (p, t.curve.end)));
            }
        }
        self.clean();
        Ok(values)
    }
    pub(crate) fn cancel(&mut self, owner: u32, id: u32) {
        if let Some(o) = self.by_owner.get_mut(&owner) {
            o.remove(&id);
        }
        self.clean();
    }
    fn clean(&mut self) {
        for o in self.by_owner.values_mut() {
            o.retain(|_, t| !t.is_empty());
        }
        self.by_owner.retain(|_, o| !o.is_empty());
    }
    pub(crate) fn status(&self, owner: u32, id: u32) -> MotionStatus {
        let tracks = self
            .by_owner
            .get(&owner)
            .and_then(|o| o.get(&id))
            .into_iter()
            .flat_map(|t| t.iter())
            .map(|(&property, t)| MotionTrackStatus {
                property,
                from: t.curve.start,
                to: t.curve.end,
                current: t.curve.sample(t.elapsed),
                duration: t.curve.duration_ms,
                elapsed: t.elapsed,
                external: t.external,
                complete: t.curve.complete(t.elapsed),
            })
            .collect::<Vec<_>>();
        MotionStatus {
            complete: tracks.iter().all(|t| t.complete),
            tracks,
        }
    }
    pub(crate) fn tick(&mut self, delta: u64) -> Vec<(u32, u32, MotionProperty, f32)> {
        let mut values = Vec::new();
        for (&owner, targets) in &mut self.by_owner {
            for (&id, tracks) in targets {
                for (&property, t) in tracks {
                    if !t.external && !t.curve.complete(t.elapsed) {
                        t.elapsed = t.elapsed.saturating_add(delta).min(t.curve.duration_ms);
                        values.push((owner, id, property, t.curve.sample(t.elapsed)));
                    }
                }
            }
        }
        values
    }
    pub(crate) fn validate(&self, scale: bool, target: impl Fn(u32, u32) -> bool) -> Result<()> {
        let mut total = 0;
        for (&owner, targets) in &self.by_owner {
            let mut count = 0;
            for (&id, tracks) in targets {
                ensure!(
                    target(owner, id) && !tracks.is_empty() && tracks.len() <= 4,
                    "motion target missing"
                );
                for (&p, t) in tracks {
                    endpoints(p, t.curve.start, t.curve.end, scale)?;
                    t.curve.validate()?;
                    ensure!(
                        t.curve.started_ms == 0 && t.curve.duration_ms <= 86_400_000,
                        "invalid retained motion clock"
                    );
                    count += 1;
                }
            }
            ensure!(count <= 4096, "motion owner limit");
            total += count;
        }
        ensure!(total <= 16384, "motion total limit");
        Ok(())
    }
}
