//! Sprite auxiliary/linked mask policy; stored coverage math belongs to image.
use super::*;
#[derive(Clone)]
pub(super) struct SpriteMaskRecipe {
    pub bitmap: i32,
    pub raster: DisplayRaster,
    pub signature: Value,
}
impl BgiVm {
    pub(super) fn sprite_mask_recipe(&self, object: i32) -> Result<Option<SpriteMaskRecipe>> {
        let Some(p) = self.graph.objects.get(&object) else {
            return Ok(None);
        };
        let mode = p
            .named_properties
            .get("target-object-mode")
            .copied()
            .unwrap_or(0);
        if !matches!(mode, 0 | 1 | 6) {
            return Ok(None);
        }
        let source_format = if mode == 6 {
            2
        } else {
            p.format_resource
                .and_then(|id| self.bitmaps.get(&(id as u32)))
                .map_or(2, |b| b.format)
        };
        let position = self
            .graph
            .draw_position(object)
            .context("BGI masked sprite position")?;
        let (bitmap, channel, origin, attenuation, retain_full, bounds) =
            if self.graph.sprite_link_masks.contains(&object) {
                let linked = *self
                    .graph
                    .sprite_relations
                    .get(&object)
                    .context("BGI sprite mask relation")?;
                let linked_p = self
                    .graph
                    .objects
                    .get(&linked)
                    .context("BGI linked sprite object")?;
                let bitmap = linked_p
                    .format_resource
                    .context("BGI linked sprite primary is not configured")?;
                let b = self.bitmap(bitmap)?.1;
                // Native4163C0 returns its documented format error without
                // modifying the temporary source when the provider is not ARGB.
                if b.format != 2 {
                    return Ok(None);
                }
                let linked_position = self
                    .graph
                    .draw_position(linked)
                    .context("BGI linked sprite position")?;
                let bounds = self
                    .graph
                    .images
                    .painted
                    .get(&linked)
                    .map(|image| image.image.size())
                    .unwrap_or(b.size);
                (
                    bitmap,
                    3,
                    [
                        position[0].wrapping_sub(linked_position[0]),
                        position[1].wrapping_sub(linked_position[1]),
                    ],
                    linked_p.transparency_parameter() as u16,
                    true,
                    Some(bounds),
                )
            } else if let Some(bitmap) = p.aux_resource {
                let b = self.bitmap(bitmap)?.1;
                let binding = self
                    .graph
                    .aux_bindings
                    .get(&object)
                    .context("BGI auxiliary bitmap binding")?;
                ensure!(
                    *binding == (bitmap, b.generation),
                    "BGI auxiliary bitmap allocation expired"
                );
                (
                    bitmap,
                    if b.format == 3 { 0 } else { 3 },
                    position,
                    0,
                    false,
                    None,
                )
            } else {
                return Ok(None);
            };
        let b = self.bitmap(bitmap)?.1;
        let signature = json!({"bitmap":bitmap,"generation":b.generation,"revision":self.graph.bitmap_versions.get(&(bitmap as u32)),"origin":origin,"channel":channel,"replace":source_format==1,"attenuation":attenuation,"full":retain_full,"bounds":bounds});
        Ok(Some(SpriteMaskRecipe {
            bitmap,
            raster: DisplayRaster::CoverageMask {
                origin,
                channel,
                replace: source_format == 1,
                attenuation,
                retain_full,
                bounds,
            },
            signature,
        }))
    }
    pub(super) fn clear_native_sprite_mask(&mut self, object: i32) {
        if let Some(p) = self.graph.objects.get_mut(&object) {
            p.aux_resource = None;
        }
        self.graph.aux_bindings.remove(&object);
        self.graph.sprite_link_masks.remove(&object);
        self.graph.mask_signatures.remove(&object);
    }
    pub(super) fn unlink_native_sprite(&mut self, object: i32) -> i32 {
        let flag = self.graph.sprite_link_masks.contains(&object)
            || self
                .graph
                .objects
                .get(&object)
                .is_some_and(|p| p.aux_resource.is_some());
        if !flag || !self.graph.sprite_relations.contains_key(&object) {
            return 13;
        }
        let linked = self.graph.sprite_relations.remove(&object).unwrap();
        self.graph.sprite_mask_owners.remove(&linked);
        self.clear_native_sprite_mask(object);
        self.graph.mark_visual(linked);
        0
    }
    pub(super) fn sprite_mask_call(&mut self, call: &NativeCall) -> Result<Option<i32>> {
        let a = &call.args;
        let object = a[0];
        if call.group == 0x91 {
            if self.graph.tree.kind(object) != Some(Kind::Sprite) {
                return Ok(Some(255));
            }
            let linked = a[1];
            if linked == object
                || (linked != 0 && self.graph.tree.kind(linked) != Some(Kind::Sprite))
            {
                return Ok(Some(11));
            }
            let status = if linked == 0 {
                self.unlink_native_sprite(object)
            } else {
                self.clear_native_sprite_mask(object);
                if self.graph.sprite_relations.contains_key(&object) {
                    13
                } else if self.graph.sprite_mask_owners.contains_key(&linked) {
                    14
                } else {
                    self.graph.sprite_relations.insert(object, linked);
                    self.graph.sprite_mask_owners.insert(linked, object);
                    self.graph.sprite_link_masks.insert(object);
                    self.graph.mark_visual(linked);
                    0
                }
            };
            self.refresh_native_sprite_masks()?;
            return Ok(Some(status));
        }
        ensure!(
            self.graph.tree.kind(object) == Some(Kind::Sprite),
            "BGI invalid sprite auxiliary handle"
        );
        if a[1] == -1 {
            self.clear_native_sprite_mask(object);
        } else {
            let bitmap = self.bitmap(a[1])?.1.clone();
            ensure!(
                matches!(bitmap.format, 2 | 3),
                "BGI sprite auxiliary bitmap is neither grayscale nor ARGB"
            );
            self.unlink_native_sprite(object);
            self.graph.objects.entry(object).or_default().aux_resource = Some(a[1]);
            self.graph
                .aux_bindings
                .insert(object, (a[1], bitmap.generation));
        }
        self.graph.mask_signatures.remove(&object);
        self.refresh_native_sprite_masks()?;
        Ok(None)
    }
    pub(super) fn refresh_native_sprite_masks(&mut self) -> Result<()> {
        let ids = self
            .graph
            .objects
            .iter()
            .filter_map(|(&id, p)| {
                (p.aux_resource.is_some()
                    || self.graph.sprite_link_masks.contains(&id)
                    || self.graph.mask_signatures.contains_key(&id))
                .then_some(id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            let p = &self.graph.objects[&id];
            let Some(primary) = p.format_resource else {
                continue;
            };
            // Linking an unconfigured provider is legal; the native draw then
            // rejects its absent primary bitmap rather than revealing source.
            let missing_provider = self.graph.sprite_link_masks.contains(&id)
                && self.graph.sprite_relations.get(&id).is_some_and(|linked| {
                    self.graph
                        .objects
                        .get(linked)
                        .is_none_or(|p| p.format_resource.is_none())
                });
            if missing_provider {
                self.graph.mask_unready.insert(id);
                continue;
            }
            self.graph.mask_unready.remove(&id);
            let recipe = self.sprite_mask_recipe(id)?;
            let signature = recipe.as_ref().map_or(Value::Null, |r| r.signature.clone());
            if self.graph.mask_signatures.get(&id) == Some(&signature) {
                continue;
            }
            self.graph.mask_signatures.insert(id, signature);
            if !self.graph.images.plans.contains_key(&id) {
                let b = self.bitmap(primary)?.1;
                self.set_graph_plan(id, GraphPlan::bitmap(primary, b.format));
            } else {
                self.graph.images.dirty(id);
            }
        }
        Ok(())
    }
}

impl BgiVm {
    pub(super) fn forget_sprite_mask_links(&mut self, object: i32) {
        self.clear_native_sprite_mask(object);
        self.graph.mask_unready.remove(&object);
        if let Some(linked) = self.graph.sprite_relations.remove(&object) {
            self.graph.sprite_mask_owners.remove(&linked);
            self.graph.mark_visual(linked);
        }
        if let Some(owner) = self.graph.sprite_mask_owners.remove(&object) {
            self.graph.sprite_relations.remove(&owner);
            self.clear_native_sprite_mask(owner);
            self.graph.images.dirty(owner);
        }
    }
}
