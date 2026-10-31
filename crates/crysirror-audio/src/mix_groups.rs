// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Compiled bounded control DAG. Mutation allocates on the owner; per-output-
//! frame evaluation follows dense parent-before-child indices without searches.
use crate::envelope::Ramp;
use anyhow::{Context, Result, ensure};
use crysirror_core::service::{AudioMixGroupInfo, AudioMixGroupSettings};
use std::collections::BTreeMap;
const GROUPS: usize = 256;
const CLIENT_GROUPS: usize = 128;
const DEPTH: usize = 8;
pub(crate) fn name(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty() && s.len() <= 128 && s.trim() == s && !s.chars().any(char::is_control),
        "invalid audio mix group name"
    );
    Ok(())
}
#[derive(Clone)]
struct Group {
    settings: AudioMixGroupSettings,
    parent: Option<usize>,
    gain: Ramp,
    output: f32,
}
#[derive(Default, Clone)]
pub(crate) struct MixGroups {
    ids: BTreeMap<(u32, String), usize>,
    slots: Vec<Option<Group>>,
    order: Vec<usize>,
}
impl MixGroups {
    pub(crate) fn is_fading(&self) -> bool {
        self.slots.iter().flatten().any(|g| g.gain.left > 0)
    }
    pub(crate) fn resolve(&self, client: u32, group: Option<&str>) -> Result<Option<usize>> {
        group
            .map(|n| {
                name(n)?;
                self.ids
                    .get(&(client, n.to_owned()))
                    .copied()
                    .context("audio mix group missing in this client")
            })
            .transpose()
    }
    pub(crate) fn validate(
        &self,
        client: u32,
        n: &str,
        s: &AudioMixGroupSettings,
    ) -> Result<(usize, Option<usize>, Vec<usize>)> {
        name(n)?;
        ensure!(
            s.gain.is_finite() && (0.0..=4.).contains(&s.gain),
            "mix group gain must be 0..4"
        );
        let old = self.resolve(client, Some(n)).ok().flatten();
        ensure!(
            old.is_some()
                || (self.ids.len() < GROUPS
                    && self.ids.keys().filter(|(c, _)| *c == client).count() < CLIENT_GROUPS),
            "audio mix group capacity exceeded"
        );
        let id = old.unwrap_or_else(|| {
            self.slots
                .iter()
                .position(Option::is_none)
                .unwrap_or(self.slots.len())
        });
        let parent = self.resolve(client, s.parent.as_deref())?;
        let mut parents: Vec<_> = self
            .slots
            .iter()
            .map(|g| g.as_ref().map(|g| g.parent))
            .collect();
        if id == parents.len() {
            parents.push(Some(parent));
        } else {
            parents[id] = Some(parent);
        }
        let mut order = Vec::new();
        for start in 0..parents.len() {
            if parents[start].is_none() {
                continue;
            }
            let mut at = Some(start);
            let mut path = [usize::MAX; DEPTH];
            let mut depth = 0;
            while let Some(i) = at {
                ensure!(!path[..depth].contains(&i), "audio mix group parent cycle");
                ensure!(
                    depth < DEPTH,
                    "audio mix group hierarchy exceeds depth eight"
                );
                path[depth] = i;
                depth += 1;
                at = parents
                    .get(i)
                    .copied()
                    .flatten()
                    .context("audio mix group parent missing")?;
            }
            order.push((depth, start));
        }
        order.sort_unstable();
        Ok((id, parent, order.into_iter().map(|(_, id)| id).collect()))
    }
    pub(crate) fn set(
        &mut self,
        client: u32,
        n: &str,
        settings: AudioMixGroupSettings,
        frames: u64,
    ) -> Result<()> {
        let (id, parent, order) = self.validate(client, n, &settings)?;
        let level = if settings.muted { 0. } else { settings.gain };
        if id == self.slots.len() {
            self.slots.push(None);
        }
        if let Some(group) = self.slots[id].as_mut() {
            group.settings = settings;
            group.parent = parent;
            group.gain.to(level, frames);
        } else {
            let mut gain = Ramp::new(1.);
            gain.to(level, frames);
            self.slots[id] = Some(Group {
                settings,
                parent,
                gain,
                output: 1.,
            });
            self.ids.insert((client, n.into()), id);
        }
        self.order = order;
        Ok(())
    }
    pub(crate) fn info(
        &self,
        client: u32,
        n: &str,
        rate: u32,
    ) -> Result<Option<AudioMixGroupInfo>> {
        name(n)?;
        let Some(&id) = self.ids.get(&(client, n.into())) else {
            return Ok(None);
        };
        let group = self.slots[id].as_ref().unwrap();
        let mut effective = group.gain.value;
        let mut parent = group.parent;
        while let Some(i) = parent {
            let p = self.slots[i].as_ref().unwrap();
            effective *= p.gain.value;
            parent = p.parent;
        }
        Ok(Some(AudioMixGroupInfo {
            settings: group.settings.clone(),
            gain: group.gain.value,
            effective_gain: effective,
            fade_frames: group.gain.left,
            output_rate: rate,
        }))
    }
    pub(crate) fn validate_remove(&self, client: u32, n: &str) -> Result<()> {
        let id = self.resolve(client, Some(n))?.unwrap();
        ensure!(
            !self.slots.iter().flatten().any(|g| g.parent == Some(id)),
            "mix group still has child groups"
        );
        Ok(())
    }
    pub(crate) fn remove(&mut self, client: u32, n: &str) -> Result<()> {
        self.validate_remove(client, n)?;
        let id = self.ids.remove(&(client, n.into())).unwrap();
        self.slots[id] = None;
        self.order.retain(|i| *i != id);
        Ok(())
    }
    pub(crate) fn step(&mut self) {
        for &id in &self.order {
            let parent = self.slots[id].as_ref().unwrap().parent;
            let parent_gain = parent.map_or(1., |p| self.slots[p].as_ref().unwrap().output);
            let g = self.slots[id].as_mut().unwrap();
            g.output = g.gain.step() * parent_gain;
        }
    }
    /// Owner-boundary product; unlike output(), includes immediate control writes
    /// that have not yet been sampled by a PCM render.
    pub(crate) fn current_gain(&self, mut id: Option<usize>) -> f32 {
        let mut gain = 1.;
        while let Some(i) = id {
            let group = self.slots[i].as_ref().unwrap();
            gain *= group.gain.value;
            id = group.parent;
        }
        gain
    }
    pub(crate) fn output(&self, id: Option<usize>) -> f32 {
        id.map_or(1., |i| self.slots[i].as_ref().unwrap().output)
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn shared_group_budget_is_scoped_bounded_reclaimable_and_isolated() {
        let mut groups = MixGroups::default();
        for client in [1, 2] {
            for i in 0..128 {
                groups
                    .set(
                        client,
                        &format!("g{i}"),
                        AudioMixGroupSettings::default(),
                        0,
                    )
                    .unwrap();
            }
        }
        assert!(
            groups
                .set(1, "overflow", AudioMixGroupSettings::default(), 0)
                .is_err()
        );
        assert!(
            groups
                .set(3, "global-overflow", AudioMixGroupSettings::default(), 0)
                .is_err()
        );
        assert!(groups.info(3, "g0", 48000).unwrap().is_none());
        groups.remove(1, "g0").unwrap();
        groups
            .set(
                3,
                "reclaimed",
                AudioMixGroupSettings {
                    gain: 0.25,
                    ..Default::default()
                },
                0,
            )
            .unwrap();
        assert!(groups.info(1, "g0", 48000).unwrap().is_none());
        assert_eq!(
            groups.info(2, "g0", 48000).unwrap().unwrap().effective_gain,
            1.0
        );
        assert_eq!(
            groups
                .info(3, "reclaimed", 48000)
                .unwrap()
                .unwrap()
                .effective_gain,
            0.25
        );
        groups.remove(1, "g1").unwrap();
        // A foreign parent's name cannot confer cross-client authority.
        assert!(
            groups
                .set(
                    1,
                    "foreign-parent",
                    AudioMixGroupSettings {
                        parent: Some("reclaimed".into()),
                        ..Default::default()
                    },
                    0
                )
                .is_err()
        );
    }
}
