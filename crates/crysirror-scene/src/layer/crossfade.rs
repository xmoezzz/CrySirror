// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! Scoped, live subtree crossfades. The engine owns time and isolated inputs;
//! callers decide what to do with the original nodes after completion.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubtreeCrossfade {
    pub source: u32,
    pub duration_ms: u64,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<RuleWipe>,
    /// Progress comes from the client (`SetCrossfadeElapsed`), not the
    /// engine clock (KiriKiri transition `callback` tick sources).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub external: bool,
}
/// Rule-image wipe (KiriKiri `universal` transition): with phase
/// `progress * (255 + vague)` (vague 0 is a hard edge), a pixel whose rule value is below
/// `phase - vague` shows the new subtree, at or above `phase` the old one, and
/// in between blends linearly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleWipe {
    /// Grayscale rule image asset, stretched over the destination root.
    pub asset: String,
    /// Soft-edge width in rule levels, 1..=4096.
    pub vague: u32,
}
#[derive(Clone, Debug)]
pub struct CompositeFrame {
    pub before: Vec<Primitive>,
    pub after: Vec<Primitive>,
    pub progress: f32,
    /// Rule wipe and the destination root's canvas rect [x, y, w, h].
    pub rule: Option<(RuleWipe, [f32; 4])>,
}
impl LayerSystem {
    pub fn crossfade(&self, client: ClientId, id: u32) -> Option<&SubtreeCrossfade> {
        self.crossfades.get(&client.0)?.get(&id)
    }
    pub(super) fn begin_crossfade(
        &mut self,
        client: ClientId,
        id: u32,
        source: u32,
        duration_ms: u64,
        rule: Option<RuleWipe>,
        external: bool,
    ) -> Result<()> {
        ensure!(
            id != source && (1..=600_000).contains(&duration_ms),
            "crossfade endpoints/duration"
        );
        if let Some(r) = &rule {
            ensure!(
                !r.asset.is_empty() && r.asset.len() <= 512 && r.vague <= 4096,
                "rule wipe asset/vague"
            );
        }
        ensure!(
            self.crossfades.values().map(BTreeMap::len).sum::<usize>() < 8,
            "crossfade limit"
        );
        let owner = self
            .clients
            .get(&client.0)
            .context("crossfade client missing")?;
        let a = owner.get(&id).context("crossfade destination missing")?;
        let b = owner.get(&source).context("crossfade source missing")?;
        // Endpoints may sit under different parents (KiriKiri fore/back page
        // layers): the source subtree is rendered in the destination's place.
        ensure!(a.rect[2..] == b.rect[2..], "crossfade needs equally sized roots");
        ensure!(a.parent.is_some() == b.parent.is_some(), "crossfade roots must both be children or both roots");
        ensure!(
            a.blend == Blend::Normal && b.blend == Blend::Normal,
            "crossfade root blend"
        );
        ensure!(
            self.crossfade(client, id).is_none(),
            "crossfade already active"
        );
        let aa = descendants(owner, id);
        let bb = descendants(owner, source);
        for (&other, fade) in self.crossfades.get(&client.0).into_iter().flatten() {
            ensure!(
                nested_or_disjoint(owner, (id, source), (other, fade.source)),
                "overlapping crossfades"
            );
        }
        ensure!(aa.is_disjoint(&bb), "overlapping crossfade endpoints");
        self.crossfades.entry(client.0).or_default().insert(
            id,
            SubtreeCrossfade {
                source,
                duration_ms,
                elapsed_ms: 0,
                rule,
                external,
            },
        );
        Ok(())
    }
    /// Set an externally clocked crossfade's progress (clamped to its
    /// duration; reaching it completes the crossfade).
    pub(super) fn set_crossfade_elapsed(&mut self, client: ClientId, id: u32, elapsed_ms: u64) -> Result<()> {
        let fade = self
            .crossfades
            .get_mut(&client.0)
            .and_then(|f| f.get_mut(&id))
            .context("crossfade missing")?;
        ensure!(fade.external, "crossfade is engine-clocked");
        fade.elapsed_ms = elapsed_ms.min(fade.duration_ms);
        Ok(())
    }
    pub(super) fn validate_crossfades(&self) -> Result<()> {
        ensure!(
            self.crossfades.values().map(BTreeMap::len).sum::<usize>() <= 8,
            "crossfade limit"
        );
        for (client, fades) in &self.crossfades {
            let owner = self
                .clients
                .get(client)
                .context("crossfade client missing")?;
            for (id, fade) in fades {
                ensure!(
                    *id != fade.source
                        && (1..=600_000).contains(&fade.duration_ms)
                        && fade.elapsed_ms <= fade.duration_ms,
                    "invalid crossfade state"
                );
                let a = owner.get(id).context("crossfade destination missing")?;
                let b = owner
                    .get(&fade.source)
                    .context("crossfade source missing")?;
                ensure!(
                    a.parent.is_some() == b.parent.is_some() && a.rect[2..] == b.rect[2..],
                    "crossfade needs equally sized roots"
                );
                let aa = descendants(owner, *id);
                let bb = descendants(owner, fade.source);
                ensure!(aa.is_disjoint(&bb), "overlapping crossfade endpoints");
                for child in aa.union(&bb) {
                    ensure!(
                        owner[child].blend == Blend::Normal,
                        "isolated crossfade requires normal subtree blends"
                    );
                }
                for (&other, f) in fades {
                    if other != *id {
                        ensure!(
                            nested_or_disjoint(owner, (*id, fade.source), (other, f.source)),
                            "overlapping crossfades"
                        );
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn composite_frame(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        let all: Vec<(u32, u32)> = self
            .crossfades
            .iter()
            .flat_map(|(&c, f)| f.keys().map(move |&id| (c, id)))
            .collect();
        self.compose(canvas, &all)
    }
    /// The plain frame with `fades` applied. Strictly nested crossfades are
    /// composed inside-out: an inner composite is part of its outer
    /// crossfade's isolated input.
    fn compose(&self, canvas: [u32; 2], fades: &[(u32, u32)]) -> Result<Vec<Primitive>> {
        let plain = self.frame_plain(canvas)?;
        let mut out = plain;
        // Top-level fades are not inside another listed fade's endpoints.
        let inside = |client: u32, inner: u32, outer: u32| -> bool {
            let (Some(owner), Some(f)) = (self.clients.get(&client), self.crossfade(ClientId(client), outer)) else {
                return false;
            };
            inner != outer
                && (descendants(owner, outer).contains(&inner) || descendants(owner, f.source).contains(&inner))
        };
        for &(client, id) in fades {
            if fades.iter().any(|&(c, o)| c == client && inside(client, id, o)) {
                continue;
            }
            let owner = self
                .clients
                .get(&client)
                .context("crossfade client missing")?;
            let fade = self.crossfade(ClientId(client), id).context("crossfade missing")?;
            let a = owner.get(&id).context("crossfade destination removed")?;
            let b = owner
                .get(&fade.source)
                .context("crossfade source removed")?;
            ensure!(a.rect[2..] == b.rect[2..], "crossfade geometry changed incompatibly");
            let aa = descendants(owner, id);
            let bb = descendants(owner, fade.source);
            let nested: Vec<(u32, u32)> = fades
                .iter()
                .copied()
                .filter(|&(c, o)| c == client && inside(client, o, id))
                .collect();
            let positions: Vec<_> = out
                .iter()
                .enumerate()
                .filter(|(_, p)| p.owner == client && aa.contains(&p.id))
                .map(|(i, _)| i)
                .collect();
            if let (Some(first), Some(last)) = (positions.first(), positions.last()) {
                ensure!(
                    out[*first..=*last].iter().all(
                        |p| p.owner == client && (aa.contains(&p.id) || bb.contains(&p.id))
                    ),
                    "crossfade subtree interleaves external layers"
                );
            }
            let inner_frame = if nested.is_empty() { None } else { Some(self.compose(canvas, &nested)?) };
            let before: Vec<_> = inner_frame
                .as_ref()
                .unwrap_or(&out)
                .iter()
                .filter(|p| p.owner == client && aa.contains(&p.id))
                .cloned()
                .collect();
            let mut next = self.clone();
            let target = next
                .clients
                .get_mut(&client)
                .unwrap()
                .get_mut(&fade.source)
                .unwrap();
            target.visible = a.visible;
            target.rect[0] = a.rect[0];
            target.rect[1] = a.rect[1];
            // Rendered in the destination's place: under its parent too.
            target.parent = a.parent;
            let after = next
                .compose(canvas, &nested)?
                .into_iter()
                .filter(|p| p.owner == client && bb.contains(&p.id))
                .collect();
            out.retain(|p| p.owner != client || (!aa.contains(&p.id) && !bb.contains(&p.id)));
            let mut p = rectangle(
                client,
                id,
                0,
                [0., 0., canvas[0] as f32, canvas[1] as f32],
                Paint::Image {
                    asset: format!("__crysirror_composite/{id}"),
                    tint: [255; 4],
                },
                None,
                a.order,
            );
            let rule = match &fade.rule {
                Some(r) => Some((r.clone(), absolute_rect(owner, id)?)),
                None => None,
            };
            p.composite = Some(Arc::new(CompositeFrame {
                before,
                after,
                progress: fade.elapsed_ms as f32 / fade.duration_ms as f32,
                rule,
            }));
            out.push(p);
        }
        out.sort_by_key(|p| (p.order, p.owner, p.id, p.sub));
        Ok(out)
    }
}
/// Two crossfades are compatible when their endpoint subtrees are disjoint or
/// one crossfade lies wholly inside a single endpoint subtree of the other.
fn nested_or_disjoint(owner: &BTreeMap<u32, LayerSpec>, x: (u32, u32), y: (u32, u32)) -> bool {
    let (xa, xb) = (descendants(owner, x.0), descendants(owner, x.1));
    let (ya, yb) = (descendants(owner, y.0), descendants(owner, y.1));
    let xs: BTreeSet<_> = xa.union(&xb).copied().collect();
    let ys: BTreeSet<_> = ya.union(&yb).copied().collect();
    // Strict nesting: the inner pair sits below (not at) one outer endpoint root.
    xs.is_disjoint(&ys)
        || (xs.is_subset(&ya) && !xs.contains(&y.0))
        || (xs.is_subset(&yb) && !xs.contains(&y.1))
        || (ys.is_subset(&xa) && !ys.contains(&x.0))
        || (ys.is_subset(&xb) && !ys.contains(&x.1))
}
/// Canvas rect of an untransformed layer: parent offsets summed.
fn absolute_rect(owner: &BTreeMap<u32, LayerSpec>, id: u32) -> Result<[f32; 4]> {
    let root = owner.get(&id).context("rule wipe root missing")?;
    let [mut x, mut y, w, h] = root.rect;
    let mut cur = root;
    loop {
        let t = &cur.transform;
        ensure!(
            t.rotate == 0. && t.scale_x == 1. && t.scale_y == 1. && !t.camera_enabled,
            "rule wipe requires an untransformed destination chain"
        );
        let Some(p) = cur.parent else { break };
        cur = owner.get(&p).context("rule wipe parent missing")?;
        x += cur.rect[0];
        y += cur.rect[1];
    }
    Ok([x, y, w, h])
}
fn descendants(owner: &BTreeMap<u32, LayerSpec>, root: u32) -> BTreeSet<u32> {
    let mut ids = BTreeSet::from([root]);
    loop {
        let old = ids.len();
        for (&id, node) in owner {
            if node.parent.is_some_and(|p| ids.contains(&p)) {
                ids.insert(id);
            }
        }
        if ids.len() == old {
            return ids;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: u32, parent: Option<u32>, order: i32) -> LayerSpec {
        serde_json::from_value(serde_json::json!({"id":id,"parent":parent,"rect":[0,0,8,8],"order":[0,order],"paint":{"kind":"solid","color":[255,0,0,255]}})).unwrap()
    }
    fn fixture() -> LayerSystem {
        let mut layers = LayerSystem::default();
        let mut source = node(3, None, 3);
        source.visible = false;
        source.rect[0] = 20.;
        layers
            .install(
                ClientId(1),
                &[
                    node(1, None, 0),
                    node(2, Some(1), 1),
                    source,
                    node(4, Some(3), 4),
                ],
            )
            .unwrap();
        layers
    }
    fn begin(layers: &mut LayerSystem) {
        layers
            .apply_batch(
                ClientId(1),
                &[LayerCommand::BeginCrossfade {
                    id: 1,
                    source: 3,
                    duration_ms: 1000,
                    rule: None, external: false,
                }],
            )
            .unwrap();
    }
    #[test]
    fn scoped_crossfade_preserves_nodes_and_roundtrips_time_and_children() {
        let mut layers = fixture();
        layers.install(ClientId(2), &[node(1, None, 20)]).unwrap();
        let original = serde_json::to_value(&layers).unwrap();
        begin(&mut layers);
        layers.tick(500);
        let encoded = serde_json::to_value(&layers).unwrap();
        let restored: LayerSystem = serde_json::from_value(encoded).unwrap();
        restored.validate().unwrap();
        let frame = restored.frame([32, 32]).unwrap();
        assert!(
            frame
                .iter()
                .any(|p| p.owner == 2 && p.id == 1 && p.composite.is_none())
        );
        let fade = frame.iter().find_map(|p| p.composite.as_ref()).unwrap();
        assert_eq!(fade.progress, 0.5);
        assert_eq!(
            fade.before.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            fade.after.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!(fade.after[0].quad[0][0], 0.);
        assert_eq!(layers.get(ClientId(1), 3).unwrap().rect[0], 20.);
        assert!(!layers.get(ClientId(1), 3).unwrap().visible);
        layers
            .apply_batch(ClientId(1), &[LayerCommand::EndCrossfade { id: 1 }])
            .unwrap();
        let ended = serde_json::to_value(&layers).unwrap();
        assert_eq!(ended["clients"], original["clients"]);
        assert!(
            layers
                .frame([32, 32])
                .unwrap()
                .iter()
                .all(|p| p.composite.is_none())
        );
    }
    #[test]
    fn malformed_restored_state_and_incompatible_mutations_fail_transactionally() {
        let mut layers = fixture();
        begin(&mut layers);
        let value = serde_json::to_value(&layers).unwrap();
        for field in ["duration_ms", "source", "elapsed_ms"] {
            let mut bad = value.clone();
            bad["crossfades"]["1"]["1"][field] =
                serde_json::json!(if field == "elapsed_ms" { 1001 } else { 0 });
            assert!(
                serde_json::from_value::<LayerSystem>(bad)
                    .unwrap()
                    .validate()
                    .is_err(),
                "{field}"
            );
        }
        let mut changed = node(3, None, 3);
        changed.rect[2] = 9.;
        assert!(
            layers
                .apply_batch(ClientId(1), &[LayerCommand::Put { layer: changed }])
                .is_err()
        );
        assert_eq!(layers.get(ClientId(1), 3).unwrap().rect[2], 8.);
        let mut child = node(2, Some(1), 1);
        child.blend = Blend::Multiply;
        assert!(
            layers
                .apply_batch(ClientId(1), &[LayerCommand::Put { layer: child }])
                .is_err()
        );
        assert!(
            layers
                .apply_batch(
                    ClientId(1),
                    &[LayerCommand::BeginCrossfade {
                        id: 3,
                        source: 1,
                        duration_ms: 1000,
                        rule: None, external: false,
                    }]
                )
                .is_err()
        );
        layers
            .apply_batch(ClientId(1), &[LayerCommand::Remove { id: 3 }])
            .unwrap();
        assert!(layers.crossfade(ClientId(1), 1).is_none());
        layers.frame([32, 32]).unwrap();
    }
    #[test]
    fn crossfade_rejects_interleaved_external_layers_and_enforces_global_budget() {
        let mut layers = fixture();
        layers.install(ClientId(2), &[node(8, None, 0)]).unwrap();
        begin(&mut layers);
        assert!(
            layers
                .frame([32, 32])
                .unwrap_err()
                .to_string()
                .contains("interleaves")
        );
        let mut layers = LayerSystem::default();
        for owner in 1..=9 {
            layers
                .install(ClientId(owner), &[node(1, None, 0), node(2, None, 1)])
                .unwrap();
            let result = layers.apply_batch(
                ClientId(owner),
                &[LayerCommand::BeginCrossfade {
                    id: 1,
                    source: 2,
                    duration_ms: 10,
                    rule: None, external: false,
                }],
            );
            assert_eq!(result.is_ok(), owner <= 8);
        }
    }
}
