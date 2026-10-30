//! Immutable logical snapshots of engine-rendered UI. No font rasterizer,
//! device readback or native VM is required to freeze the current controls.
use super::UiSystem;
use crate::layer::{LayerSpec, Paint, SpriteTransform, MAX_LAYERS};
use anyhow::{ensure, Context, Result};
use std::collections::BTreeSet;

impl UiSystem {
    /// Freeze this owner's current UI paint/layout/style/clip/order into
    /// independent layers. `root` selects only an ordinary widget subtree;
    /// `None` also includes this owner's currently rendered menus/dialogs.
    /// Widget values/focus/hover are sampled now. Later edits do not mutate
    /// the returned data. Window decorations and OS cursors are not UI paints.
    pub fn snapshot_layers(
        &self,
        owner: u32,
        canvas: [u32; 2],
        root: Option<u32>,
    ) -> Result<Vec<LayerSpec>> {
        self.validate()?;
        let allowed = if let Some(root) = root {
            let widgets = self
                .clients
                .get(&owner)
                .context("UI snapshot client widgets missing")?;
            ensure!(widgets.contains_key(&root), "UI snapshot root missing");
            let mut ids = BTreeSet::new();
            for (&id, _) in widgets {
                let mut current = Some(id);
                let mut depth = 0;
                while let Some(cursor) = current {
                    ensure!(depth <= 64, "UI snapshot parent depth");
                    if cursor == root {
                        ids.insert(id);
                        break;
                    }
                    current = widgets
                        .get(&cursor)
                        .context("UI snapshot parent missing")?
                        .spec
                        .parent;
                    depth += 1;
                }
            }
            Some(ids)
        } else {
            None
        };
        let mut primitives = self.frame_widgets(canvas, Some(owner))?;
        if allowed.is_none() {
            primitives.extend(
                self.menus
                    .frame(canvas)?
                    .into_iter()
                    .filter(|p| p.owner == owner),
            );
            primitives.extend(
                self.dialogs
                    .frame(canvas)?
                    .into_iter()
                    .filter(|p| p.owner == owner),
            );
        }
        primitives.retain(|p| allowed.as_ref().is_none_or(|ids| ids.contains(&p.id)));
        primitives.sort_by_key(|p| (p.order, p.owner, p.id, p.sub));
        ensure!(
            primitives.len() <= MAX_LAYERS,
            "UI snapshot primitive limit"
        );
        let mut layers = Vec::with_capacity(primitives.len());
        for primitive in primitives {
            ensure!(primitive.owner == owner, "UI snapshot owner mismatch");
            ensure!(
                primitive.composite.is_none() && primitive.puppet.is_none(),
                "UI snapshot cannot freeze derived composite/mesh state"
            );
            let [x, y, z, w] = primitive.quad[0];
            let [width, height] = primitive.extent;
            ensure!(
                z == 0.
                    && w == 1.
                    && primitive.quad
                        == [
                            [x, y, 0., 1.],
                            [x + width, y, 0., 1.],
                            [x + width, y + height, 0., 1.],
                            [x, y + height, 0., 1.]
                        ],
                "UI snapshot requires exact axis-aligned primitive geometry"
            );
            if let Some(clip) = primitive.clip {
                ensure!(
                    clip.iter().all(|v| v.is_finite()),
                    "UI snapshot clip must be finite"
                );
                if clip[2] <= 0. || clip[3] <= 0. {
                    continue;
                }
            }
            let uv = if matches!(&primitive.paint, Paint::GlyphRun { .. }) {
                ensure!(
                    primitive.uv == [0., 0., 1., 1.],
                    "UI glyph snapshot UV transform"
                );
                None
            } else {
                Some(primitive.uv)
            };
            layers.push(LayerSpec {
                id: layers.len() as u32 + 1,
                parent: None,
                rect: [x, y, width, height],
                paint: primitive.paint,
                transform: SpriteTransform::default(),
                order: primitive.order,
                clip: primitive.clip.map(|c| [c[0] - x, c[1] - y, c[2], c[3]]),
                opacity: primitive.opacity,
                visible: true,
                blend: primitive.blend,
                uv,
                sampling: primitive.sampling,
                filter: primitive.filter,
            });
        }
        Ok(layers)
    }
}
