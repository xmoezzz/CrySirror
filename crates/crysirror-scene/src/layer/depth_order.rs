// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Client-scoped stable floating-depth ordering, independent of native VMs.
//!
//! An explicit depth creates an ordering scope for the whole layer subtree.
//! Within each scope, `order[0]` remains the major bucket. Ordinary nodes retain
//! their minor order and draw before depth scopes in that bucket. Depth scopes
//! sort by finite depth, then the client's last committed order; new scopes
//! append in admission order. A nested depth stays within its ancestor scope.
//! `frame(&self)` only derives keys. The owner explicitly commits the history
//! after accepting a logical frame, never after an intermediate property write.
use super::{LayerSpec, Primitive, MAX_LAYERS};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

#[derive(Clone, Debug, PartialEq)]
pub struct DepthOrderStep {
    pub scope: u32,
    pub major: i32,
    pub depth: f32,
    pub rank: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DepthOrderKey {
    /// Outermost explicit depth scope first; at most65 entries.
    pub path: Vec<DepthOrderStep>,
}

/// Common ordering for ordinary layers, retained depth scopes, UI, frozen
/// sources and renderer merges. Ordinary-only inputs retain the original rule.
pub fn compare_primitives(a: &Primitive, b: &Primitive) -> Ordering {
    compare_keys(
        a.order,
        a.owner,
        a.id,
        a.sub,
        a.depth_order.as_ref(),
        b.order,
        b.owner,
        b.id,
        b.sub,
        b.depth_order.as_ref(),
    )
}
fn compare_keys(
    ao: [i32; 2],
    ac: u32,
    ai: u32,
    asub: u8,
    ak: Option<&DepthOrderKey>,
    bo: [i32; 2],
    bc: u32,
    bi: u32,
    bsub: u8,
    bk: Option<&DepthOrderKey>,
) -> Ordering {
    let ap = ak.map(|k| k.path.as_slice()).unwrap_or(&[]);
    let bp = bk.map(|k| k.path.as_slice()).unwrap_or(&[]);
    let mut level = 0;
    loop {
        let aa = ap.get(level);
        let bb = bp.get(level);
        let major = aa
            .map_or(ao[0], |s| s.major)
            .cmp(&bb.map_or(bo[0], |s| s.major));
        if major != Ordering::Equal {
            return major;
        }
        match (aa, bb) {
            (None, None) => return (ao[1], ac, ai, asub).cmp(&(bo[1], bc, bi, bsub)),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a), Some(b)) => {
                // Validated finite values; signed zero is the same depth.
                let cmp = a
                    .depth
                    .partial_cmp(&b.depth)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| ac.cmp(&bc))
                    .then_with(|| a.rank.cmp(&b.rank))
                    .then_with(|| a.scope.cmp(&b.scope));
                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
        }
        level += 1;
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Domain {
    depths: BTreeMap<u32, f32>,
    admissions: Vec<u32>,
    history: Vec<u32>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DepthOrders {
    clients: BTreeMap<u32, Domain>,
}
pub(super) type DepthKeys = BTreeMap<(u32, u32), DepthOrderKey>;
impl DepthOrders {
    pub fn set(&mut self, client: u32, id: u32, depth: Option<f32>) -> Result<()> {
        if let Some(value) = depth {
            ensure!(value.is_finite(), "layer depth must be finite");
            let domain = self.clients.entry(client).or_default();
            domain.depths.insert(id, value);
            if !domain.admissions.contains(&id) {
                domain.admissions.push(id);
            }
        } else if let Some(domain) = self.clients.get_mut(&client) {
            // Disabling is a pending property change. Keep the last committed
            // identity until an owner frame accepts that it left the domain.
            // None→Some before that frame must not change an unseen tie order.
            domain.depths.remove(&id);
        }
        Ok(())
    }
    pub fn get(&self, client: u32, id: u32) -> Option<f32> {
        self.clients.get(&client)?.depths.get(&id).copied()
    }
    pub fn remove(&mut self, client: u32, id: u32) {
        if let Some(domain) = self.clients.get_mut(&client) {
            domain.depths.remove(&id);
            domain.admissions.retain(|n| *n != id);
            domain.history.retain(|n| *n != id);
            if domain.admissions.is_empty() {
                self.clients.remove(&client);
            }
        }
    }
    pub fn validate(&self, owners: &BTreeMap<u32, BTreeMap<u32, LayerSpec>>) -> Result<()> {
        ensure!(
            self.clients.len() <= 64
                && self
                    .clients
                    .values()
                    .map(|d| d.admissions.len())
                    .sum::<usize>()
                    <= MAX_LAYERS,
            "layer depth count"
        );
        for (client, domain) in &self.clients {
            let owner = owners.get(client).context("layer depth client missing")?;
            ensure!(!domain.admissions.is_empty(), "empty layer depth domain");
            let ids: BTreeSet<_> = domain.depths.keys().copied().collect();
            ensure!(
                ids.iter().all(|id| owner.contains_key(id))
                    && domain.depths.values().all(|d| d.is_finite()),
                "layer depth owner/id/value"
            );
            let admissions: BTreeSet<_> = domain.admissions.iter().copied().collect();
            let history: BTreeSet<_> = domain.history.iter().copied().collect();
            ensure!(
                ids.is_subset(&admissions)
                    && admissions.iter().all(|id| owner.contains_key(id))
                    && admissions.len() == domain.admissions.len(),
                "layer depth admission identities"
            );
            ensure!(
                history.is_subset(&admissions) && history.len() == domain.history.len(),
                "layer depth history identities"
            );
        }
        Ok(())
    }
    pub fn keys(&self, owners: &BTreeMap<u32, BTreeMap<u32, LayerSpec>>) -> Result<DepthKeys> {
        self.validate(owners)?;
        let mut out = BTreeMap::new();
        for (client, domain) in &self.clients {
            if domain.depths.is_empty() {
                continue;
            }
            let owner = owners.get(client).context("layer depth client missing")?;
            let mut ranks = BTreeMap::new();
            for id in domain.history.iter().chain(&domain.admissions) {
                if !ranks.contains_key(id) {
                    ranks.insert(*id, ranks.len() as u32);
                }
            }
            for (id, spec) in owner {
                let mut path = Vec::new();
                let mut cursor = Some(spec.id);
                let mut count = 0;
                while let Some(node) = cursor {
                    count += 1;
                    ensure!(count <= 65, "layer depth parent cycle/depth");
                    let layer = owner.get(&node).context("layer depth parent missing")?;
                    if let Some(depth) = domain.depths.get(&node) {
                        path.push(DepthOrderStep {
                            scope: node,
                            major: layer.order[0],
                            depth: *depth,
                            rank: *ranks.get(&node).context("layer depth rank missing")?,
                        });
                    }
                    cursor = layer.parent;
                }
                if !path.is_empty() {
                    path.reverse();
                    out.insert((*client, *id), DepthOrderKey { path });
                }
            }
        }
        Ok(out)
    }
    pub fn commit(&mut self, owners: &BTreeMap<u32, BTreeMap<u32, LayerSpec>>) -> Result<()> {
        self.validate(owners)?;
        let keys = self.keys(owners)?;
        let mut updates = Vec::new();
        for (client, domain) in &self.clients {
            let owner = &owners[client];
            let mut ids: Vec<_> = domain
                .admissions
                .iter()
                .filter(|id| domain.depths.contains_key(id))
                .copied()
                .collect();
            ids.sort_by(|a, b| {
                compare_keys(
                    owner[a].order,
                    *client,
                    *a,
                    0,
                    keys.get(&(*client, *a)),
                    owner[b].order,
                    *client,
                    *b,
                    0,
                    keys.get(&(*client, *b)),
                )
            });
            updates.push((*client, ids));
        }
        for (client, ids) in updates {
            let domain = self.clients.get_mut(&client).unwrap();
            domain.history = ids;
            let active = &domain.depths;
            domain.admissions.retain(|id| active.contains_key(id));
        }
        self.clients.retain(|_, domain| !domain.depths.is_empty());
        Ok(())
    }
    /// Static snapshots carry the effective order in their own immutable specs,
    /// so rendering a snapshot never consults the later live depth side-map.
    pub fn freeze(
        &self,
        client: u32,
        owners: &BTreeMap<u32, BTreeMap<u32, LayerSpec>>,
        out: &mut [LayerSpec],
    ) -> Result<()> {
        let keys = self.keys(owners)?;
        if !out.iter().any(|l| keys.contains_key(&(client, l.id))) {
            return Ok(());
        }
        out.sort_by(|a, b| {
            compare_keys(
                a.order,
                client,
                a.id,
                0,
                keys.get(&(client, a.id)),
                b.order,
                client,
                b.id,
                0,
                keys.get(&(client, b.id)),
            )
        });
        for (rank, layer) in out.iter_mut().enumerate() {
            layer.order = [rank as i32, 0];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerCommand as C, LayerSystem};
    use crysirror_core::state::ClientId;
    fn layer(id: u32, parent: Option<u32>, order: [i32; 2]) -> LayerSpec {
        serde_json::from_value(serde_json::json!({"id":id,"parent":parent,"rect":[0,0,8,8],"order":order,"paint":{"kind":"solid","color":[0,0,0,0]}})).unwrap()
    }
    fn put(s: &mut LayerSystem, id: u32, parent: Option<u32>, order: [i32; 2]) {
        s.apply_batch(
            ClientId(1),
            &[C::Put {
                layer: layer(id, parent, order),
            }],
        )
        .unwrap();
    }
    fn depth(s: &mut LayerSystem, id: u32, value: Option<f32>) {
        s.apply_batch(ClientId(1), &[C::SetDepth { id, depth: value }])
            .unwrap();
    }
    fn ids(s: &LayerSystem) -> Vec<u32> {
        s.frame([64, 64]).unwrap().iter().map(|p| p.id).collect()
    }
    #[test]
    fn history_commits_only_at_owner_boundary_and_equal_depth_keeps_latest_order() {
        let mut s = LayerSystem::default();
        for id in [99, 1, 42] {
            put(&mut s, id, None, [0, 0]);
            depth(&mut s, id, Some(0.));
        }
        assert_eq!(ids(&s), [99, 1, 42]);
        let before = serde_json::to_value(&s).unwrap();
        ids(&s);
        ids(&s);
        assert_eq!(serde_json::to_value(&s).unwrap(), before);
        s.commit_frame_order().unwrap();
        depth(&mut s, 99, Some(2.));
        assert_eq!(ids(&s), [1, 42, 99]);
        s.commit_frame_order().unwrap();
        depth(&mut s, 99, Some(-0.));
        assert_eq!(ids(&s), [1, 42, 99]);
        depth(&mut s, 1, Some(5.));
        assert_eq!(ids(&s), [42, 99, 1]);
        depth(&mut s, 1, Some(0.));
        assert_eq!(ids(&s), [1, 42, 99]);
        assert!(s
            .apply_batch(
                ClientId(1),
                &[C::SetDepth {
                    id: 1,
                    depth: Some(f32::NAN)
                }]
            )
            .is_err());
        assert_eq!(ids(&s), [1, 42, 99]);
        depth(&mut s, 42, None);
        depth(&mut s, 42, Some(0.));
        assert_eq!(ids(&s), [1, 42, 99]);
        depth(&mut s, 42, None);
        s.commit_frame_order().unwrap();
        depth(&mut s, 42, Some(0.));
        assert_eq!(ids(&s), [1, 99, 42]);
    }
    #[test]
    fn groups_inherit_depth_and_nested_scopes_do_not_escape_their_parent() {
        let mut s = LayerSystem::default();
        for (id, parent, order) in [
            (900, None, [0, 999]),
            (100, None, [0, -1]),
            (1, Some(100), [0, 0]),
            (2, Some(100), [9, 0]),
            (200, None, [0, 0]),
            (500, None, [1, 0]),
        ] {
            put(&mut s, id, parent, order);
        }
        depth(&mut s, 100, Some(10.));
        depth(&mut s, 200, Some(20.));
        depth(&mut s, 2, Some(30.));
        assert_eq!(ids(&s), [900, 100, 1, 2, 200, 500]);
        let frame = s.frame([64, 64]).unwrap();
        let child = frame.iter().find(|p| p.id == 1).unwrap();
        assert_eq!(child.depth_order.as_ref().unwrap().path[0].scope, 100);
        assert_eq!(
            frame
                .iter()
                .find(|p| p.id == 2)
                .unwrap()
                .depth_order
                .as_ref()
                .unwrap()
                .path
                .len(),
            2
        );
        let mut reversed = frame.clone();
        reversed.reverse();
        reversed.sort_by(compare_primitives);
        assert_eq!(reversed.iter().map(|p| p.id).collect::<Vec<_>>(), ids(&s));
        depth(&mut s, 100, None);
        depth(&mut s, 200, None);
        depth(&mut s, 2, None);
        assert_eq!(ids(&s), [100, 1, 200, 900, 500, 2]);
    }
    #[test]
    fn restore_rollback_remove_and_readmission_keep_scoped_canonical_history() {
        let mut s = LayerSystem::default();
        for id in [9, 1] {
            put(&mut s, id, None, [0, 0]);
            depth(&mut s, id, Some(0.));
        }
        s.commit_frame_order().unwrap();
        let before = serde_json::to_value(&s).unwrap();
        assert!(s
            .apply_batch(
                ClientId(1),
                &[
                    C::SetDepth {
                        id: 1,
                        depth: Some(-5.)
                    },
                    C::Remove { id: 999 }
                ]
            )
            .is_err());
        assert_eq!(serde_json::to_value(&s).unwrap(), before);
        let result: Result<()> = s.transaction(|s| {
            depth(s, 1, Some(-5.));
            s.commit_frame_order()?;
            s.apply_batch(ClientId(1), &[C::Remove { id: 9 }])?;
            anyhow::bail!("rollback")
        });
        assert!(result.is_err());
        assert_eq!(serde_json::to_value(&s).unwrap(), before);
        let restored: LayerSystem = serde_json::from_value(before.clone()).unwrap();
        restored.validate().unwrap();
        assert_eq!(ids(&s.clone()), ids(&restored));
        let mut foreign = before.clone();
        foreign["depth_orders"]["clients"]["1"]["depths"]["999"] = serde_json::json!(0.);
        assert!(serde_json::from_value::<LayerSystem>(foreign)
            .unwrap()
            .validate()
            .is_err());
        assert!(s
            .apply_batch(
                ClientId(2),
                &[C::SetDepth {
                    id: 1,
                    depth: Some(0.)
                }]
            )
            .is_err());
        s.apply_batch(ClientId(1), &[C::Remove { id: 9 }]).unwrap();
        put(&mut s, 9, None, [0, 0]);
        depth(&mut s, 9, Some(0.));
        assert_eq!(ids(&s), [1, 9]);
        s.apply_batch(ClientId(1), &[C::Remove { id: 1 }, C::Remove { id: 9 }])
            .unwrap();
        s.validate().unwrap();
        assert!(ids(&s).is_empty());
    }
    #[test]
    fn frozen_subtrees_bake_depth_and_do_not_follow_later_live_changes() {
        let mut s = LayerSystem::default();
        put(&mut s, 100, None, [0, -1]);
        for id in [9, 1] {
            put(&mut s, id, Some(100), [0, 0]);
            depth(&mut s, id, Some(if id == 9 { 10. } else { 0. }));
        }
        let frozen = s.snapshot_subtree_specs(ClientId(1), 100).unwrap();
        let frozen_json = serde_json::to_value(&frozen).unwrap();
        depth(&mut s, 9, Some(-10.));
        s.commit_frame_order().unwrap();
        assert_eq!(serde_json::to_value(&frozen).unwrap(), frozen_json);
        let mut static_scene = LayerSystem::default();
        static_scene.install(ClientId(1), &frozen).unwrap();
        assert_eq!(ids(&static_scene), [100, 1, 9]);
        assert_eq!(ids(&s), [100, 9, 1]);
    }
}
