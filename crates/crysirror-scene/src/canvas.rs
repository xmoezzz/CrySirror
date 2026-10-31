// SPDX-License-Identifier: MPL-2.0
//! Engine-owned retained drawing surfaces. Copies snapshot logical draw nodes,
//! not another engine's framebuffer. Presentation remains ordinary Layer nodes.
use crate::layer::{
    Blend, LayerCommand, LayerSpec, LayerSystem, Paint, SpriteTransform, intersect,
};
use anyhow::{Context, Result, ensure};
use crysirror_core::state::ClientId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
mod sample;
mod image_dependencies;
/// Per-client surface budget; native layer trees keep one surface per layer.
pub const MAX_SURFACES: usize = crysirror_core::service::MAX_CANVAS_SURFACES_PER_CLIENT;
/// A surface whose node tree grows past this is frozen into one snapshot
/// draw (same pixels) and queued for materialization into a plain image.
/// Region copies between surfaces (save/restore of a glyph cell, say) split
/// straddling nodes into bands, so trees otherwise grow without bound.
pub const COMPACT_NODES: usize = 256;
/// A full-surface snapshot that a presenting renderer should rasterize once
/// and publish as an immutable image (`CanvasSystem::resolve_materialized`).
#[derive(Clone, Debug)]
pub struct MaterializeRequest {
    pub client: u32,
    pub surface: u32,
    pub size: [u32; 2],
    pub layers: Arc<Vec<LayerSpec>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasSurface {
    pub width: u32,
    pub height: u32,
    pub nodes: Vec<CanvasNode>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanvasNode {
    Draw {
        layer: LayerSpec,
    },
    Group {
        offset: [f32; 2],
        clip: [f32; 4],
        opacity: u8,
        nodes: Vec<CanvasNode>,
        /// Rotation (radians) and scale applied about the group origin,
        /// before `offset`: dest = offset + R(rotate)·S(scale)·local.
        #[serde(default, skip_serializing_if = "is_zero")]
        rotate: f32,
        #[serde(default = "unit_scale", skip_serializing_if = "is_unit")]
        scale: [f32; 2],
    },
}
fn is_zero(v: &f32) -> bool {
    *v == 0.
}
fn unit_scale() -> [f32; 2] {
    [1., 1.]
}
fn is_unit(v: &[f32; 2]) -> bool {
    *v == [1., 1.]
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanvasCommand {
    Create {
        id: u32,
        width: u32,
        height: u32,
    },
    /// Release one unpresented surface. Hide its presentations first. Frozen
    /// copies own their draw nodes and remain valid after the source is gone.
    Destroy { id: u32 },
    Clear {
        id: u32,
        color: [u8; 4],
    },
    /// Change the surface size, keeping its contents anchored at the
    /// top-left; area outside previous content stays transparent.
    Resize {
        id: u32,
        width: u32,
        height: u32,
    },
    /// Append a draw; with `atop` it is drawn source-atop (see Copy).
    Draw {
        id: u32,
        layer: LayerSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        atop: Option<AtopMode>,
    },
    /// Atomically replace this surface's contents with one logical resource draw.
    Set {
        id: u32,
        layer: LayerSpec,
    },
    Copy {
        source: u32,
        destination: u32,
        rect: [f32; 4],
        position: [f32; 2],
        opacity: u8,
        #[serde(default)]
        replace: bool,
        /// Destination-space draw clip (krkr `Layer.setClip`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip: Option<[f32; 4]>,
        /// Draw keeping the destination alpha (krkr holdAlpha): source-atop or
        /// a Photoshop multiply/screen. The touched region becomes one
        /// isolated snapshot.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        atop: Option<AtopMode>,
    },
    /// Fill `rect` with a solid colour; `replace` replaces the region's pixels
    /// (transparent colours included) instead of drawing over them.
    ///
    /// `keep_alpha` sets only the colour of the region's existing content and
    /// keeps its alpha (krkr `holdAlpha` fills); `color[3]` is then ignored.
    Fill {
        id: u32,
        rect: [f32; 4],
        color: [u8; 4],
        #[serde(default)]
        replace: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        keep_alpha: bool,
    },
    /// Multiply the alpha of `rect`'s existing content by `opacity`/255, colour
    /// unchanged (krkr negative-opacity colorRect). The region is frozen as one
    /// isolated snapshot so overlapping draws fade as a single composite.
    Fade {
        id: u32,
        rect: [f32; 4],
        opacity: u8,
    },
    /// Recolour `rect`'s existing content through a layer filter (mono,
    /// invert, bright/dark, colour mix), blended over the original by
    /// `opacity`/255 (RealLive `grpMono`/`grpInvert`/`grpLight`). The region
    /// is frozen as one isolated snapshot, like `fade`.
    Filter {
        id: u32,
        rect: [f32; 4],
        filter: crate::layer::LayerFilter,
        #[serde(default = "opaque", skip_serializing_if = "is_opaque")]
        opacity: u8,
    },
    /// Copy `rect` of `source` through an affine map of source-surface
    /// coordinates (`[a, b, c, d, tx, ty]`: x' = a·x + c·y + tx,
    /// y' = b·x + d·y + ty) without skew. `replace` replaces the covered
    /// destination region; it requires an axis-aligned result.
    TransformCopy {
        source: u32,
        destination: u32,
        rect: [f32; 4],
        matrix: [f32; 6],
        #[serde(default)]
        replace: bool,
        /// Destination-space draw clip (krkr `Layer.setClip`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip: Option<[f32; 4]>,
        /// Group opacity of the transformed copy, drawn over the destination
        /// (krkr `operateStretch`/`operateAffine` opacity); 255 by default.
        #[serde(default = "opaque", skip_serializing_if = "is_opaque")]
        opacity: u8,
    },
    /// Freeze a visible, static, normal-alpha Layer subtree as one snapshot draw.
    /// The renderer rasterizes it at its original size before later effects.
    /// `rect` is source-root-local; `position` and `clip` are destination-space.
    /// The region is replaced, including alpha. Root placement/visibility/opacity
    /// and inheritance from outside the subtree do not enter the snapshot.
    CopySubtree {
        source: u32,
        destination: u32,
        rect: [f32; 4],
        position: [f32; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip: Option<[f32; 4]>,
    },
    /// Replace (or create) a surface with a previously queried state
    /// (`EngineQuery::Canvas`), e.g. a native game's saved DCs.
    Restore {
        id: u32,
        surface: CanvasSurface,
    },
    Present {
        id: u32,
        layer: u32,
        #[serde(default)]
        order: i32,
        /// Existing client layer the presentation is attached under; its
        /// transform, visibility, opacity and clip are inherited.
        #[serde(default)]
        parent: Option<u32>,
        /// Top-left of the surface in the parent's (or canvas) space.
        #[serde(default)]
        position: [f32; 2],
    },
    Hide {
        layer: u32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Presentation {
    source: u32,
    order: i32,
    generated: Vec<u32>,
    #[serde(default)]
    parent: Option<u32>,
    #[serde(default)]
    position: [f32; 2],
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasSystem {
    /// Copy-on-write surfaces: a staged clone shares them until written.
    clients: BTreeMap<u32, BTreeMap<u32, Arc<CanvasSurface>>>,
    presentations: BTreeMap<u32, BTreeMap<u32, Presentation>>,
    #[serde(default)]
    serial: BTreeMap<u32, u32>,
    /// Per (client, surface) content revision, bumped by every mutation.
    #[serde(skip)]
    revisions: BTreeMap<(u32, u32), u64>,
    /// Surfaces whose whole content is one copy of another surface's region.
    #[serde(skip)]
    provenance: BTreeMap<(u32, u32), Provenance>,
    /// Compacted surfaces awaiting materialization, by (client, surface).
    #[serde(skip)]
    materialize: BTreeMap<(u32, u32), MaterializeRequest>,
}
/// The last node of a surface is `region` of `source` (at `revision`) copied
/// with `offset` over its first `base` nodes. Copying it back unchanged puts
/// those base nodes underneath the source region: (region over base).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Provenance {
    source: u32,
    revision: u64,
    region: [f32; 4],
    offset: [f32; 2],
    base: usize,
}
fn opaque() -> u8 {
    255
}
fn is_opaque(v: &u8) -> bool {
    *v == 255
}
fn contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    inner[0] >= outer[0]
        && inner[1] >= outer[1]
        && inner[0] + inner[2] <= outer[0] + outer[2]
        && inner[1] + inner[3] <= outer[1] + outer[3]
}
fn spec(id: u32, rect: [f32; 4], paint: Paint) -> LayerSpec {
    LayerSpec {
        id,
        parent: None,
        rect,
        paint,
        transform: SpriteTransform::default(),
        order: [0, 0],
        clip: None,
        opacity: 255,
        visible: true,
        blend: Blend::Normal,
        uv: None,
        sampling:Default::default(),
        filter: None,
    }
}
impl CanvasSystem {
    pub fn surface(&self, client: ClientId, id: u32) -> Option<&CanvasSurface> {
        self.clients.get(&client.0)?.get(&id).map(|s| &**s)
    }
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients.keys().copied()
    }
    pub fn validate(&self) -> Result<()> {
        for owner in self.clients.values() {
            ensure!(owner.len() <= MAX_SURFACES, "canvas surface limit");
            for s in owner.values() {
                ensure!(
                    s.width > 0 && s.height > 0 && s.width <= 8192 && s.height <= 8192,
                    "canvas dimensions"
                );
                let mut count = 0;
                validate_nodes(&s.nodes, 0, &mut count).map_err(|e| {
                    anyhow::anyhow!("{e:#} (canvas {}x{}, {} top-level nodes)", s.width, s.height, s.nodes.len())
                })?;
            }
        }
        Ok(())
    }
    fn get_mut(&mut self, c: ClientId, id: u32) -> Result<&mut CanvasSurface> {
        self.clients
            .get_mut(&c.0)
            .and_then(|x| x.get_mut(&id))
            .map(Arc::make_mut)
            .context("canvas absent")
    }
    pub fn apply_batch(
        &mut self,
        c: ClientId,
        commands: &[CanvasCommand],
        layers: &mut LayerSystem,
    ) -> Result<()> {
        ensure!(commands.len() <= 4096, "canvas batch limit");
        // Surfaces are copy-on-write, so staging clones only the touched
        // ones; layer changes are journaled by one layer transaction.
        let mut next = self.clone();
        layers.transaction(|next_layers| {
            next.apply_commands(c, commands, next_layers)
        })?;
        *self = next;
        Ok(())
    }
    fn apply_commands(
        &mut self,
        c: ClientId,
        commands: &[CanvasCommand],
        next_layers: &mut LayerSystem,
    ) -> Result<()> {
        let next = self;
        let mut changed = false;
        let mut dirty_sources = BTreeSet::new();
        let mut dirty_presentations = BTreeSet::new();
        for command in commands {
            let mut provenance = None;
            match command {
                CanvasCommand::Destroy { id } => {
                    ensure!(!next.presentations.get(&c.0).is_some_and(|owner|owner.values().any(|p|p.source==*id)),
                        "cannot destroy a presented canvas; hide its presentations first");
                    ensure!(next.clients.get_mut(&c.0).and_then(|owner|owner.remove(id)).is_some(),"canvas surface absent");
                    next.revisions.remove(&(c.0,*id));
                    next.materialize.remove(&(c.0,*id));
                    next.provenance.retain(|(owner,surface),p|*owner!=c.0 || (*surface!=*id && p.source!=*id));
                }
                CanvasCommand::Create { id, width, height } => {
                    ensure!(
                        *width > 0 && *height > 0 && *width <= 8192 && *height <= 8192,
                        "canvas dimensions"
                    );
                    next.clients.entry(c.0).or_default().insert(
                        *id,
                        Arc::new(CanvasSurface {
                            width: *width,
                            height: *height,
                            nodes: vec![CanvasNode::Draw {
                                layer: spec(
                                    0,
                                    [0., 0., *width as f32, *height as f32],
                                    Paint::Solid {
                                        color: [0, 0, 0, 255],
                                    },
                                ),
                            }],
                        }),
                    );
                }
                CanvasCommand::Restore { id, surface } => {
                    ensure!(
                        surface.width > 0 && surface.height > 0 && surface.width <= 8192 && surface.height <= 8192,
                        "canvas dimensions"
                    );
                    let mut count = 0;
                    validate_nodes(&surface.nodes, 0, &mut count)?;
                    next.clients.entry(c.0).or_default().insert(*id, Arc::new(surface.clone()));
                }
                CanvasCommand::Clear { id, color } => {
                    let s = next.get_mut(c, *id)?;
                    s.nodes = vec![CanvasNode::Draw {
                        layer: spec(
                            0,
                            [0., 0., s.width as f32, s.height as f32],
                            Paint::Solid { color: *color },
                        ),
                    }];
                }
                CanvasCommand::Resize { id, width, height } => {
                    ensure!(
                        *width > 0 && *height > 0 && *width <= 8192 && *height <= 8192,
                        "canvas dimensions"
                    );
                    let s = next.get_mut(c, *id)?;
                    if *width < s.width || *height < s.height {
                        // Shrinking discards the cropped pixels. A later
                        // growth must not resurrect draws outside this size.
                        s.nodes = clip_to_band_mode(
                            &s.nodes,
                            [0.,0.,(*width).min(s.width) as f32,(*height).min(s.height) as f32],
                            true,
                            true,
                        );
                    }
                    s.width = *width;
                    s.height = *height;
                }
                CanvasCommand::Draw { id, layer, atop } => {
                    let node = CanvasNode::Draw { layer: layer.clone() };
                    let s = next.get_mut(c, *id)?;
                    if let Some(mode) = atop {
                        let Some(bounds) = node_bounds_mode(&node, *mode == AtopMode::Copy) else { continue };
                        let full = [0., 0., s.width as f32, s.height as f32];
                        let Some(region) = intersect(bounds, full) else { continue };
                        draw_atop(s, region, node, *mode)?;
                    } else {
                        s.nodes.push(node);
                    }
                }
                CanvasCommand::Set { id, layer } => {
                    next.get_mut(c, *id)?.nodes = vec![CanvasNode::Draw {
                        layer: layer.clone(),
                    }]
                }
                CanvasCommand::Fill {
                    id,
                    rect,
                    color,
                    replace,
                    keep_alpha,
                } => {
                    ensure!(
                        rect.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                        "canvas fill bounds"
                    );
                    ensure!(!(*replace && *keep_alpha), "canvas fill replace with keep_alpha");
                    let s = next.get_mut(c, *id)?;
                    let full = [0., 0., s.width as f32, s.height as f32];
                    let Some(r) = intersect(*rect, full) else {
                        continue;
                    };
                    if *keep_alpha {
                        let nodes = std::mem::take(&mut s.nodes);
                        let mut inside = clip_to_band(&nodes, r, true);
                        recolor(&mut inside, [color[0], color[1], color[2]])?;
                        s.nodes = exclude_region(nodes, full, r);
                        s.nodes.extend(inside);
                    } else if *replace {
                        s.nodes = exclude_region(std::mem::take(&mut s.nodes), full, r);
                    }
                    if color[3] != 0 && !*keep_alpha {
                        s.nodes.push(CanvasNode::Draw {
                            layer: spec(0, r, Paint::Solid { color: *color }),
                        });
                    }
                }
                CanvasCommand::Fade { id, rect, opacity } => {
                    ensure!(
                        rect.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                        "canvas fade bounds"
                    );
                    let s = next.get_mut(c, *id)?;
                    let full = [0., 0., s.width as f32, s.height as f32];
                    let Some(r) = intersect(*rect, full) else {
                        continue;
                    };
                    let nodes = std::mem::take(&mut s.nodes);
                    let inside = clip_to_band(&nodes, r, true);
                    s.nodes = exclude_region(nodes, full, r);
                    if *opacity != 0 && !inside.is_empty() {
                        let (paint, uv) = region_paint(&inside, r)?;
                        let mut draw = spec(0, r, paint);
                        draw.uv = Some(uv);
                        draw.opacity = *opacity;
                        s.nodes.push(CanvasNode::Draw { layer: draw });
                    }
                }
                CanvasCommand::Filter { id, rect, filter, opacity } => {
                    ensure!(
                        rect.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                        "canvas filter bounds"
                    );
                    filter.validate()?;
                    let s = next.get_mut(c, *id)?;
                    let full = [0., 0., s.width as f32, s.height as f32];
                    let Some(r) = intersect(*rect, full) else {
                        continue;
                    };
                    if *opacity == 0 {
                        continue;
                    }
                    let nodes = std::mem::take(&mut s.nodes);
                    let inside = clip_to_band(&nodes, r, true);
                    let (paint, uv) = region_paint(&inside, r)?;
                    let uv = Some(uv);
                    // Below opacity 255 the original stays under the filtered copy.
                    s.nodes = if *opacity == 255 { exclude_region(nodes, full, r) } else { nodes };
                    let mut draw = spec(0, r, paint);
                    draw.uv = uv;
                    draw.opacity = *opacity;
                    draw.filter = Some(filter.clone());
                    s.nodes.push(CanvasNode::Draw { layer: draw });
                }
                CanvasCommand::Copy {
                    source,
                    destination,
                    rect,
                    position,
                    opacity,
                    replace,
                    clip: draw_clip,
                    atop,
                } => {
                    ensure!(
                        rect.iter()
                            .chain(position)
                            .all(|v| v.is_finite() && v.abs() <= 1_000_000.)
                            && rect[2] > 0.
                            && rect[3] > 0.,
                        "canvas copy bounds"
                    );
                    ensure!(!(atop.is_some() && *replace), "canvas copy replace with atop");
                    let source_id = *source;
                    let source = next
                        .surface(c, *source)
                        .context("copy source absent")?
                        .clone();
                    let dst = next.get_mut(c, *destination)?;
                    let offset = [position[0] - rect[0], position[1] - rect[1]];
                    let Some(clip) =
                        intersect(*rect, [0., 0., source.width as f32, source.height as f32])
                            .and_then(|clip| {
                                intersect(
                                    clip,
                                    [-offset[0], -offset[1], dst.width as f32, dst.height as f32],
                                )
                            })
                    else {
                        continue;
                    };
                    // A destination draw clip narrows the copied area.
                    let Some(clip) = (match draw_clip {
                        Some(dc) => intersect(clip, [dc[0] - offset[0], dc[1] - offset[1], dc[2], dc[3]]),
                        None => Some(clip),
                    }) else {
                        continue;
                    };
                    // Copying a saved region back to where it came from, while
                    // the destination is unchanged since the save, is a no-op
                    // (krkr save/restore of a glyph cell around each redraw).
                    if let Some(p) = next.provenance.get(&(c.0, source_id)) {
                        let written = [clip[0] + offset[0], clip[1] + offset[1], clip[2], clip[3]];
                        if *replace
                            && p.source == *destination
                            && next.revisions.get(&(c.0, *destination)).copied().unwrap_or(0) == p.revision
                            && offset == [-p.offset[0], -p.offset[1]]
                            && contains(p.region, written)
                            && source.nodes.len() == p.base + 1
                        {
                            let under = clip_to_band(&source.nodes[..p.base], clip, false);
                            if under.is_empty() {
                                continue;
                            }
                            let dst = next.get_mut(c, *destination)?;
                            dst.nodes.insert(0, CanvasNode::Group {
                                offset,
                                clip,
                                opacity: 255,
                                nodes: under,
                                rotate: 0.,
                                scale: [1., 1.],
                            });
                            // Falls through to the revision bump below.
                            dirty_sources.insert(*destination);
                            *next.revisions.entry((c.0, *destination)).or_default() += 1;
                            next.provenance.remove(&(c.0, *destination));
                            changed = true;
                            continue;
                        }
                    }
                    let source_revision = next.revisions.get(&(c.0, source_id)).copied().unwrap_or(0);
                    let dst = next.get_mut(c, *destination)?;
                    if *destination != source_id && !*replace {
                        provenance = Some(Provenance {
                            source: source_id,
                            revision: source_revision,
                            region: clip,
                            offset,
                            base: dst.nodes.len(),
                        });
                    }
                    if *replace
                        && draw_clip.is_none()
                        && *rect == [0., 0., source.width as f32, source.height as f32]
                        && *position == [0., 0.]
                        && dst.width == source.width
                        && dst.height == source.height
                    {
                        dst.nodes = source.nodes;
                    } else {
                        if *replace {
                            // Region replace: keep the previous content only
                            // outside the destination region (as up to four
                            // clip bands), so the region shows the source alone,
                            // transparent pixels included.
                            let region = [clip[0] + offset[0], clip[1] + offset[1], clip[2], clip[3]];
                            let full = [0., 0., dst.width as f32, dst.height as f32];
                            dst.nodes = exclude_region(std::mem::take(&mut dst.nodes), full, region);
                        }
                        // Only source content inside the copied rect is kept.
                        let nodes = clip_to_band_mode(
                            &source.nodes, clip, false, *atop == Some(AtopMode::Copy),
                        );
                        let group = CanvasNode::Group {
                            offset,
                            clip,
                            opacity: 255,
                            nodes,
                            rotate: 0.,
                            scale: [1., 1.],
                        };
                        let region = [clip[0] + offset[0], clip[1] + offset[1], clip[2], clip[3]];
                        let group = with_opacity(group, *opacity, region, [dst.width, dst.height])?;
                        if let Some(mode) = atop {
                            provenance = None;
                            let region = [clip[0] + offset[0], clip[1] + offset[1], clip[2], clip[3]];
                            draw_atop(dst, region, group, *mode)?;
                        } else {
                            dst.nodes.push(group);
                        }
                    }
                }
                CanvasCommand::TransformCopy {
                    source,
                    destination,
                    rect,
                    matrix,
                    replace,
                    clip: draw_clip,
                    opacity,
                } => {
                    ensure!(
                        !*replace || *opacity == 255,
                        "canvas transform copy: replace is opaque"
                    );
                    if *opacity == 0 {
                        continue;
                    }
                    ensure!(
                        rect.iter().chain(matrix).all(|v| v.is_finite() && v.abs() <= 1_000_000.)
                            && rect[2] > 0.
                            && rect[3] > 0.,
                        "canvas transform copy bounds"
                    );
                    let [ma, mb, mc, md, tx, ty] = *matrix;
                    let sx = (ma * ma + mb * mb).sqrt();
                    ensure!(sx > 1e-6, "degenerate canvas transform");
                    let (cos, sin) = (ma / sx, mb / sx);
                    let sy = -mc * sin + md * cos;
                    ensure!(
                        sy.abs() > 1e-6 && (mc * cos + md * sin).abs() <= 1e-4 * (1. + sy.abs()),
                        "skewed canvas transform copy is not supported"
                    );
                    let rotate = mb.atan2(ma);
                    let source = next
                        .surface(c, *source)
                        .context("copy source absent")?
                        .clone();
                    let Some(local) =
                        intersect(*rect, [0., 0., source.width as f32, source.height as f32])
                    else {
                        continue;
                    };
                    let dst = next.get_mut(c, *destination)?;
                    let corners = [
                        [local[0], local[1]],
                        [local[0] + local[2], local[1]],
                        [local[0], local[1] + local[3]],
                        [local[0] + local[2], local[1] + local[3]],
                    ]
                    .map(|[x, y]| [ma * x + mc * y + tx, mb * x + md * y + ty]);
                    let (x0, y0) = corners.iter().fold((f32::INFINITY, f32::INFINITY), |m, p| (m.0.min(p[0]), m.1.min(p[1])));
                    let (x1, y1) = corners.iter().fold((f32::NEG_INFINITY, f32::NEG_INFINITY), |m, p| (m.0.max(p[0]), m.1.max(p[1])));
                    let bounds = match draw_clip {
                        Some(dc) => intersect([x0, y0, x1 - x0, y1 - y0], *dc),
                        None => Some([x0, y0, x1 - x0, y1 - y0]),
                    };
                    let Some(bounds) = bounds else {
                        continue;
                    };
                    if *replace {
                        let quarter = (rotate / std::f32::consts::FRAC_PI_2).round() * std::f32::consts::FRAC_PI_2;
                        ensure!(
                            (rotate - quarter).abs() <= 1e-4,
                            "replace through a non-axis-aligned canvas transform requires a region render-target API"
                        );
                        let full = [0., 0., dst.width as f32, dst.height as f32];
                        dst.nodes = exclude_region(std::mem::take(&mut dst.nodes), full, bounds);
                    }
                    let group = CanvasNode::Group {
                        offset: [tx, ty],
                        clip: local,
                        opacity: 255,
                        nodes: clip_to_band(&source.nodes, local, false),
                        rotate,
                        scale: [sx, sy],
                    };
                    let group = with_opacity(group, *opacity, bounds, [dst.width, dst.height])?;
                    // The draw clip is destination-space: an outer plain group.
                    dst.nodes.push(match draw_clip {
                        Some(_) => CanvasNode::Group {
                            offset: [0., 0.],
                            clip: bounds,
                            opacity: 255,
                            nodes: vec![group],
                            rotate: 0.,
                            scale: [1., 1.],
                        },
                        None => group,
                    });
                }
                CanvasCommand::CopySubtree { source, destination, rect, position, clip } => {
                    ensure!(rect.iter().chain(position).all(|v| v.is_finite() && v.abs() <= 1_000_000.)
                        && rect[2] > 0. && rect[3] > 0., "subtree copy bounds");
                    if let Some(clip) = clip {
                        ensure!(clip.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.)
                            && clip[2] >= 0. && clip[3] >= 0., "subtree copy clip");
                    }
                    // Commands are ordered: snapshot any preceding Canvas edits
                    // through their current engine-owned Layer presentations.
                    next.rebuild(c, next_layers, &dirty_sources, &dirty_presentations)?;
                    dirty_sources.clear(); dirty_presentations.clear();
                    let specs = next_layers.snapshot_subtree_specs(c, *source)?;
                    let root = specs.iter().find(|s| s.id == *source).context("snapshot root missing")?;
                    let dst = next.surface(c, *destination).context("snapshot destination absent")?;
                    let offset = [position[0] - rect[0], position[1] - rect[1]];
                    let bounds = intersect(*rect, [0., 0., root.rect[2], root.rect[3]])
                        .and_then(|r| intersect(r, [-offset[0], -offset[1], dst.width as f32, dst.height as f32]))
                        .and_then(|r| match clip {
                            Some(dc) => intersect(r, [dc[0] - offset[0], dc[1] - offset[1], dc[2], dc[3]]),
                            None => Some(r),
                        });
                    let Some(bounds) = bounds else { continue; };
                    let size = [root.rect[2].ceil() as u32, root.rect[3].ceil() as u32];
                    let paint = Paint::Snapshot { size, layers: specs.into() };
                    paint.validate()?;
                    let dst = next.get_mut(c, *destination)?;
                    let region = [bounds[0] + offset[0], bounds[1] + offset[1], bounds[2], bounds[3]];
                    let full = [0., 0., dst.width as f32, dst.height as f32];
                    dst.nodes = exclude_region(std::mem::take(&mut dst.nodes), full, region);
                    let mut draw = spec(0, region, paint);
                    draw.uv = Some([bounds[0] / size[0] as f32, bounds[1] / size[1] as f32,
                        (bounds[0] + bounds[2]) / size[0] as f32, (bounds[1] + bounds[3]) / size[1] as f32]);
                    dst.nodes.push(CanvasNode::Draw { layer: draw });
                }
                CanvasCommand::Present {
                    id,
                    layer,
                    order,
                    parent,
                    position,
                } => {
                    ensure!(next.surface(c, *id).is_some(), "present canvas absent");
                    ensure!(*layer < 0x2000_0000, "reserved canvas layer id");
                    ensure!(
                        position.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                        "canvas presentation position"
                    );
                    if let Some(parent) = parent {
                        ensure!(*parent != *layer, "canvas presentation parent cycle");
                        ensure!(
                            next_layers.get(c, *parent).is_some(),
                            "canvas presentation parent layer absent"
                        );
                    }
                    let owner = next.presentations.entry(c.0).or_default();
                    if let Some(p) = owner.get_mut(layer) {
                        p.source = *id;
                        p.order = *order;
                        p.parent = *parent;
                        p.position = *position;
                    } else {
                        owner.insert(
                            *layer,
                            Presentation {
                                source: *id,
                                order: *order,
                                generated: vec![],
                                parent: *parent,
                                position: *position,
                            },
                        );
                    }
                }
                CanvasCommand::Hide { layer } => {
                    if let Some(p) = next
                        .presentations
                        .get_mut(&c.0)
                        .and_then(|p| p.remove(layer))
                    {
                        let mut commands = p
                            .generated
                            .into_iter()
                            .rev()
                            .filter(|id| next_layers.get(c, *id).is_some())
                            .map(|id| LayerCommand::Remove { id })
                            .collect::<Vec<_>>();
                        if next_layers.get(c, *layer).is_some() {
                            commands.push(LayerCommand::Remove { id: *layer });
                        }
                        next_layers.apply_batch(c, &commands)?;
                        // Layer removal recursively removes nested presentations.
                        // Drop their bindings, retaining the independent canvases.
                        next.prune_missing_parents(c, &next_layers);
                    } else {
                        continue;
                    }
                }
            }
            // `continue` above means a true no-op (for example a clipped-out
            // copy), so it must not invalidate any presentation's derived IDs.
            let touched = match command {
                CanvasCommand::Create { id, .. }
                | CanvasCommand::Restore { id, .. }
                | CanvasCommand::Clear { id, .. }
                | CanvasCommand::Resize { id, .. }
                | CanvasCommand::Draw { id, .. }
                | CanvasCommand::Set { id, .. }
                | CanvasCommand::Fill { id, .. }
                | CanvasCommand::Fade { id, .. }
                | CanvasCommand::Filter { id, .. } => Some(*id),
                CanvasCommand::Copy { destination, .. }
                | CanvasCommand::TransformCopy { destination, .. }
                | CanvasCommand::CopySubtree { destination, .. } => Some(*destination),
                CanvasCommand::Present { layer, .. } => {
                    dirty_presentations.insert(*layer);
                    None
                }
                CanvasCommand::Hide { .. } | CanvasCommand::Destroy { .. } => None,
            };
            if let Some(id) = touched {
                dirty_sources.insert(id);
                *next.revisions.entry((c.0, id)).or_default() += 1;
                match provenance {
                    Some(p) => next.provenance.insert((c.0, id), p),
                    None => next.provenance.remove(&(c.0, id)),
                };
            }
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        for id in &dirty_sources {
            next.compact(c, *id);
        }
        next.validate_touched(c, &dirty_sources)?;
        next.rebuild(c, next_layers, &dirty_sources, &dirty_presentations)?;
        Ok(())
    }
    /// Freeze an overgrown surface into one full-surface snapshot draw. If the
    /// snapshot cannot be formed (nesting budget), the tree is kept and the
    /// presentation limit reports it.
    fn compact(&mut self, c: ClientId, id: u32) {
        fn count(nodes: &[CanvasNode]) -> usize {
            nodes.iter().map(|n| match n {
                CanvasNode::Group { nodes, .. } => 1 + count(nodes),
                CanvasNode::Draw { .. } => 1,
            }).sum()
        }
        let Some(s) = self.surface(c, id) else { return };
        if count(&s.nodes) <= COMPACT_NODES {
            return;
        }
        let full = [0., 0., s.width as f32, s.height as f32];
        let Ok((paint, uv)) = region_paint(&s.nodes, full) else { return };
        let Paint::Snapshot { size, layers } = &paint else { return };
        let request = MaterializeRequest { client: c.0, surface: id, size: *size, layers: layers.clone() };
        let mut draw = spec(0, full, paint);
        draw.uv = Some(uv);
        if let Ok(s) = self.get_mut(c, id) {
            s.nodes = vec![CanvasNode::Draw { layer: draw }];
            self.provenance.remove(&(c.0, id));
            self.materialize.insert((c.0, id), request);
        }
    }
    /// Snapshots of compacted surfaces still waiting for a renderer.
    /// Freeze the current logical surface without modifying it. Composition,
    /// clipping and group isolation use the same snapshot contract as copies.
    pub fn capture_request(&self, client: ClientId, id: u32) -> Result<MaterializeRequest> {
        let surface = self.surface(client, id).context("Canvas not found")?;
        let (paint, _) = region_paint(&surface.nodes, [0., 0., surface.width as f32, surface.height as f32])?;
        let Paint::Snapshot { size, layers } = paint else { anyhow::bail!("Canvas snapshot contract") };
        Ok(MaterializeRequest { client: client.0, surface: id, size, layers })
    }
    /// Snapshots of compacted surfaces still waiting for a renderer.
    pub fn materialize_requests(&self) -> Vec<MaterializeRequest> {
        self.materialize.values().cloned().collect()
    }
    /// Replace every draw of the frozen `layers` snapshot (in any surface of
    /// the client, including copies of it) with the published `asset` image
    /// of identical pixels, and refresh affected presentations.
    pub fn resolve_materialized(
        &mut self,
        request: &MaterializeRequest,
        asset: &str,
        layers: &mut LayerSystem,
    ) -> Result<()> {
        // A worker/renderer may complete after another compaction of the
        // same surface. Its immutable inputs can still replace retained
        // copies of that generation, but cannot clear the newer request.
        let current = self
            .materialize
            .get(&(request.client, request.surface))
            .is_some_and(|pending| Arc::ptr_eq(&pending.layers, &request.layers));
        fn replace(nodes: &mut [CanvasNode], target: &Arc<Vec<LayerSpec>>, asset: &str) -> bool {
            let mut hit = false;
            for n in nodes {
                match n {
                    CanvasNode::Group { nodes, .. } => hit |= replace(nodes, target, asset),
                    CanvasNode::Draw { layer } => {
                        let same = matches!(&layer.paint, Paint::Snapshot { layers, .. } if Arc::ptr_eq(layers, target));
                        if same {
                            layer.paint = Paint::Image { asset: asset.to_owned(), tint: [255; 4] };
                            hit = true;
                        }
                    }
                }
            }
            hit
        }
        let c = ClientId(request.client);
        let mut next = self.clone();
        if current {
            next.materialize.remove(&(request.client, request.surface));
        }
        let ids: Vec<u32> = next.clients.get(&c.0).map(|o| o.keys().copied().collect()).unwrap_or_default();
        let mut touched = BTreeSet::new();
        for id in ids {
            let hit = next.surface(c, id).is_some_and(|s| {
                let mut probe = s.nodes.clone();
                replace(&mut probe, &request.layers, asset)
            });
            if hit {
                let s = next.get_mut(c, id)?;
                replace(&mut s.nodes, &request.layers, asset);
                touched.insert(id);
            }
        }
        if !touched.is_empty() {
            layers.transaction(|next_layers| next.rebuild(c, next_layers, &touched, &BTreeSet::new()))?;
        }
        *self = next;
        Ok(())
    }
    /// [`CanvasSystem::validate`] restricted to the surfaces a batch wrote.
    fn validate_touched(&self, c: ClientId, ids: &BTreeSet<u32>) -> Result<()> {
        let Some(owner) = self.clients.get(&c.0) else {
            return Ok(());
        };
        ensure!(owner.len() <= MAX_SURFACES, "canvas surface limit");
        for id in ids {
            if let Some(s) = owner.get(id) {
                ensure!(
                    s.width > 0 && s.height > 0 && s.width <= 8192 && s.height <= 8192,
                    "canvas dimensions"
                );
                let mut count = 0;
                validate_nodes(&s.nodes, 0, &mut count).map_err(|e| {
                    anyhow::anyhow!("{e:#} (canvas {}x{}, {} top-level nodes)", s.width, s.height, s.nodes.len())
                })?;
            }
        }
        Ok(())
    }
    fn prune_missing_parents(&mut self, c: ClientId, layers: &LayerSystem) {
        if let Some(owner) = self.presentations.get_mut(&c.0) {
            owner.retain(|_, p| p.parent.is_none_or(|parent| layers.get(c, parent).is_some()));
        }
    }
    fn rebuild(
        &mut self,
        c: ClientId,
        layers: &mut LayerSystem,
        dirty_sources: &BTreeSet<u32>,
        dirty_presentations: &BTreeSet<u32>,
    ) -> Result<()> {
        self.prune_missing_parents(c, layers);
        let entries: Vec<_> = self.presentations.get(&c.0).into_iter()
            .flat_map(|owner| owner.iter())
            .filter(|(root, p)| dirty_sources.contains(&p.source) || dirty_presentations.contains(root))
            .map(|(root, p)| (*root, p.clone()))
            .collect();
        for (root, p) in entries {
            let s = self
                .surface(c, p.source)
                .context("present source absent")?
                .clone();
            let mut commands = p
                .generated
                .iter()
                .rev()
                // Derived nodes may already be gone after an explicit ancestor
                // Remove. Keep the public Layer Remove contract strict; only
                // this owner-managed cleanup skips absent generated IDs.
                .filter(|id| layers.get(c, **id).is_some())
                .map(|id| LayerCommand::Remove { id: *id })
                .collect::<Vec<_>>();
            let mut parent = spec(
                root,
                [p.position[0], p.position[1], s.width as f32, s.height as f32],
                Paint::Solid { color: [0; 4] },
            );
            parent.parent = p.parent;
            parent.order = [p.order, -1];
            parent.clip = Some([0., 0., s.width as f32, s.height as f32]);
            commands.push(LayerCommand::Put { layer: parent });
            let mut ids = Vec::new();
            let mut draw_order = 0;
            let serial = self.serial.entry(c.0).or_default();
            emit(
                &s.nodes,
                root,
                p.order,
                &mut draw_order,
                serial,
                &mut ids,
                &mut commands,
            )?;
            layers.apply_batch(c, &commands).with_context(|| {
                fn stats(nodes: &[CanvasNode], depth: usize) -> (usize, usize) {
                    nodes.iter().fold((0, depth), |(n, d), node| match node {
                        CanvasNode::Group { nodes, .. } => {
                            let (cn, cd) = stats(nodes, depth + 1);
                            (n + 1 + cn, d.max(cd))
                        }
                        CanvasNode::Draw { .. } => (n + 1, d),
                    })
                }
                let (nodes, depth) = stats(&s.nodes, 0);
                // Developer diagnostic: the failing source surface (Debug form).
                if let Some(path) = std::env::var_os("CRYSIRROR_CANVAS_FAILURE_DUMP") {
                    let _ = std::fs::write(path, format!("{:#?}", s.nodes));
                }
                format!(
                    "rebuild canvas presentation {root}, source {} ({nodes} nodes, {} top-level, depth {depth})",
                    p.source,
                    s.nodes.len()
                )
            })?;
            self.presentations
                .get_mut(&c.0)
                .unwrap()
                .get_mut(&root)
                .unwrap()
                .generated = ids;
        }
        Ok(())
    }
}
/// Conservative surface-space bounds of a node, `None` for an invisible one.
fn node_bounds_mode(node: &CanvasNode, keep_rgb: bool) -> Option<[f32; 4]> {
    match node {
        CanvasNode::Draw { layer } => {
            if (!keep_rgb && matches!(layer.paint, Paint::Solid { color } if color[3] == 0)) || !layer.visible || layer.opacity == 0 {
                return None;
            }
            let tr = &layer.transform;
            // Without rotation/scale/projection the rect is the drawn area.
            let identity = tr.rotate == 0.
                && tr.rotate_x == 0.
                && tr.rotate_y == 0.
                && tr.scale_x == 1.
                && tr.scale_y == 1.
                && !tr.camera_enabled
                && !tr.billboard;
            if identity {
                Some(layer.rect)
            } else {
                Some([-1e6, -1e6, 2e6, 2e6])
            }
        }
        CanvasNode::Group { offset, clip, rotate, scale, .. } => {
            let (s, c) = rotate.sin_cos();
            let map = |x: f32, y: f32| {
                let (x, y) = (x * scale[0], y * scale[1]);
                [offset[0] + x * c - y * s, offset[1] + x * s + y * c]
            };
            let pts = [
                map(clip[0], clip[1]),
                map(clip[0] + clip[2], clip[1]),
                map(clip[0], clip[1] + clip[3]),
                map(clip[0] + clip[2], clip[1] + clip[3]),
            ];
            let (x0, y0) = pts.iter().fold((f32::INFINITY, f32::INFINITY), |m, p| (m.0.min(p[0]), m.1.min(p[1])));
            let (x1, y1) = pts.iter().fold((f32::NEG_INFINITY, f32::NEG_INFINITY), |m, p| (m.0.max(p[0]), m.1.max(p[1])));
            Some([x0, y0, x1 - x0, y1 - y0])
        }
    }
}
/// Set the colour of every draw to `rgb`, keeping coverage and alpha. With
/// source-over blending, composing uniformly coloured draws yields `rgb` with
/// the original composite alpha; other blend modes would change the result.
fn recolor(nodes: &mut [CanvasNode], rgb: [u8; 3]) -> Result<()> {
    for node in nodes {
        match node {
            CanvasNode::Group { nodes, .. } => recolor(nodes, rgb)?,
            CanvasNode::Draw { layer } => {
                ensure!(
                    matches!(layer.blend, crate::layer::Blend::Normal),
                    "canvas colour fill over a non-normal blend draw"
                );
                match &mut layer.paint {
                    Paint::Solid { color }
                    | Paint::Text { color, .. }
                    | Paint::GlyphRun { color, .. } => {
                        color[..3].copy_from_slice(&rgb);
                        continue;
                    }
                    _ => {}
                }
                let filter = layer
                    .filter
                    .get_or_insert_with(|| crate::layer::LayerFilter {
                        mono: 0.,
                        invert: 0.,
                        bright: 0.,
                        dark: 0.,
                        color_rate: 0.,
                        color: [0.; 3],
                        add: [0.; 3],
                        tone_curve: None,
                        mask: None,
                        alpha_test: false,
                        distortion: None,
                    });
                filter.color_rate = 1.;
                filter.color = rgb.map(|v| f32::from(v) / 255.);
                filter.add = [0.; 3];
            }
        }
    }
    Ok(())
}
fn plain_group(node: &CanvasNode) -> bool {
    matches!(node, CanvasNode::Group { offset, opacity, rotate, scale, .. }
        if *offset == [0., 0.] && *opacity == 255 && *rotate == 0. && *scale == [1., 1.])
}
/// Restrict `nodes` to `band`: invisible and non-intersecting nodes are
/// dropped, plain clip groups are re-clipped in place (no extra nesting),
/// and the remaining nodes share one clip group for the band.
/// `wrap`: whether loose nodes need a band clip group (false inside a group
/// that already clips to the band).
fn clip_to_band(nodes: &[CanvasNode], band: [f32; 4], wrap_loose: bool) -> Vec<CanvasNode> {
    clip_to_band_mode(nodes, band, wrap_loose, false)
}
fn clip_to_band_mode(nodes: &[CanvasNode], band: [f32; 4], wrap_loose: bool, keep_rgb: bool) -> Vec<CanvasNode> {
    let wrap = |loose: Vec<CanvasNode>| CanvasNode::Group {
        offset: [0., 0.],
        clip: band,
        opacity: 255,
        nodes: loose,
        rotate: 0.,
        scale: [1., 1.],
    };
    let mut out = Vec::new();
    let mut loose = Vec::new();
    for node in nodes {
        let Some(bounds) = node_bounds_mode(node, keep_rgb) else { continue };
        if intersect(bounds, band).is_none() {
            continue;
        }
        if let (true, CanvasNode::Group { clip, nodes: inner, .. }) = (plain_group(node), node) {
            let Some(clip) = intersect(*clip, band) else { continue };
            let inner = clip_to_band_mode(inner, clip, false, keep_rgb);
            if inner.is_empty() {
                continue;
            }
            // Painter order: earlier loose nodes are flushed first.
            if !loose.is_empty() {
                let l = std::mem::take(&mut loose);
                if wrap_loose { out.push(wrap(l)) } else { out.extend(l) }
            }
            out.push(CanvasNode::Group {
                offset: [0., 0.],
                clip,
                opacity: 255,
                nodes: inner,
                rotate: 0.,
                scale: [1., 1.],
            });
        } else {
            loose.push(node.clone());
        }
    }
    if !loose.is_empty() {
        if wrap_loose { out.push(wrap(loose)) } else { out.extend(loose) }
    }
    out
}
/// Previous content outside `region`, so a following draw replaces it.
fn exclude_region(nodes: Vec<CanvasNode>, full: [f32; 4], region: [f32; 4]) -> Vec<CanvasNode> {
    subtract_rect(full, region)
        .into_iter()
        .flat_map(|band| clip_to_band(&nodes, band, true))
        .collect()
}
/// `full` minus `region` as at most four disjoint rectangles (x, y, w, h):
/// top band, bottom band, then left and right of the region's row span.
pub fn subtract_rect(full: [f32; 4], region: [f32; 4]) -> Vec<[f32; 4]> {
    let Some(r) = intersect(full, region) else {
        return vec![full];
    };
    let (fx0, fy0, fx1, fy1) = (full[0], full[1], full[0] + full[2], full[1] + full[3]);
    let (rx0, ry0, rx1, ry1) = (r[0], r[1], r[0] + r[2], r[1] + r[3]);
    let mut out = Vec::new();
    let mut push = |x0: f32, y0: f32, x1: f32, y1: f32| {
        if x1 > x0 && y1 > y0 {
            out.push([x0, y0, x1 - x0, y1 - y0]);
        }
    };
    push(fx0, fy0, fx1, ry0);
    push(fx0, ry1, fx1, fy1);
    push(fx0, ry0, rx0, ry1);
    push(rx1, ry0, fx1, ry1);
    out
}
fn validate_nodes(nodes: &[CanvasNode], depth: usize, count: &mut usize) -> Result<()> {
    ensure!(depth < 32, "canvas snapshot depth");
    for n in nodes {
        *count += 1;
        ensure!(*count <= 4096, "canvas node budget");
        match n {
            CanvasNode::Draw { layer } => {
                ensure!(
                    layer.parent.is_none(),
                    "canvas draw cannot reference outside parent"
                );
                layer.paint.validate()?;
                if let Some(reveal) = layer.paint.glyph_reveal_status() {
                    ensure!(
                        reveal.complete,
                        "dynamic canvas glyph reveal requires a canonical canvas clock; use a Layer glyph run"
                    );
                }
                layer.transform.validate()?;
                ensure!(
                    layer.rect.iter().all(|v| v.is_finite())
                        && layer.rect[2] > 0.
                        && layer.rect[3] > 0.,
                    "canvas node bounds"
                );
            }
            CanvasNode::Group {
                offset,
                clip,
                opacity,
                nodes,
                rotate,
                scale,
            } => {
                ensure!(
                    rotate.is_finite() && scale.iter().all(|s| s.is_finite() && s.abs() > 1e-6),
                    "canvas group transform"
                );
                ensure!(
                    offset.iter().chain(clip).all(|v| v.is_finite())
                        && clip[2] > 0.
                        && clip[3] > 0.,
                    "canvas group bounds"
                );
                ensure!(
                    *opacity == 255,
                    "isolated canvas group opacity requires a render-target composition API"
                );
                validate_nodes(nodes, depth + 1, count)?;
            }
        }
    }
    Ok(())
}
/// How an atop draw combines with the destination it keeps the alpha of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtopMode {
    Normal,
    Multiply,
    Screen,
    /// Colour replaced, destination alpha kept.
    Copy,
}
/// Visible leaf draws (mutable), depth first.
fn leaf_draws(nodes: &mut [CanvasNode]) -> Vec<&mut LayerSpec> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            CanvasNode::Group { nodes, .. } => out.extend(leaf_draws(nodes)),
            CanvasNode::Draw { layer } => {
                if layer.visible && layer.opacity != 0 && !matches!(layer.paint, Paint::Solid { color } if color[3] == 0) {
                    out.push(layer)
                }
            }
        }
    }
    out
}
/// `group` drawn at `opacity` over a surface of `size`. Exact without a
/// render target when the group holds one draw (the opacity folds into it);
/// overlapping draws are frozen into one isolated snapshot draw over the
/// destination-space `region` they cover, and that draw carries the opacity.
fn with_opacity(group: CanvasNode, opacity: u8, region: [f32; 4], size: [u32; 2]) -> Result<CanvasNode> {
    if opacity == 255 {
        return Ok(group);
    }
    let mut probe = vec![group];
    {
        let mut leaves = leaf_draws(&mut probe);
        if leaves.len() <= 1 {
            if let Some(leaf) = leaves.pop() {
                leaf.opacity = ((u16::from(leaf.opacity) * u16::from(opacity) + 127) / 255) as u8;
            }
            return Ok(probe.pop().expect("probe group"));
        }
    }
    let full = [0., 0., size[0] as f32, size[1] as f32];
    let Some(region) = intersect(region, full) else {
        return Ok(probe.pop().expect("probe group"));
    };
    let clipped = CanvasNode::Group {
        offset: [0., 0.],
        clip: region,
        opacity: 255,
        nodes: probe,
        rotate: 0.,
        scale: [1., 1.],
    };
    let (paint, uv) = region_paint(&[clipped], region)?;
    let mut draw = spec(0, region, paint);
    draw.uv = Some(uv);
    draw.opacity = opacity;
    Ok(CanvasNode::Draw { layer: draw })
}
/// Draw `node` source-atop over `region` of `s`: the region's content and the
/// draw (every leaf marked Atop) are frozen into one isolated snapshot, so the
/// destination of the atop blend is exactly this surface's own content.
fn draw_atop(s: &mut CanvasSurface, region: [f32; 4], mut node: CanvasNode, mode: AtopMode) -> Result<()> {
    use crate::layer::Blend;
    let blend = match mode {
        AtopMode::Normal => Blend::Atop,
        AtopMode::Multiply => Blend::AtopMultiply,
        AtopMode::Screen => Blend::AtopScreen,
        AtopMode::Copy => Blend::AtopCopy,
    };
    fn mark(n: &mut CanvasNode, blend: Blend) -> Result<()> {
        match n {
            CanvasNode::Group { nodes, .. } => nodes.iter_mut().try_for_each(|n| mark(n, blend)),
            CanvasNode::Draw { layer } => {
                ensure!(matches!(layer.blend, Blend::Normal), "atop draw of a non-normal blend");
                layer.blend = blend;
                Ok(())
            }
        }
    }
    // Sequential per-draw application equals the composite only for normal
    // atop. Other modes blend the source as one image: a multi-draw source is
    // first frozen (over transparency) into a single snapshot draw.
    if mode != AtopMode::Normal {
        let mut probe = vec![node.clone()];
        if leaf_draws(&mut probe).len() > 1 {
            let (paint, uv) = region_paint(std::slice::from_ref(&node), region)?;
            let mut draw = spec(0, region, paint);
            draw.uv = Some(uv);
            node = CanvasNode::Draw { layer: draw };
        }
    }
    mark(&mut node, blend)?;
    let full = [0., 0., s.width as f32, s.height as f32];
    let nodes = std::mem::take(&mut s.nodes);
    let mut inside = clip_to_band(&nodes, region, true);
    s.nodes = exclude_region(nodes, full, region);
    inside.push(CanvasNode::Group {
        offset: [0., 0.],
        clip: region,
        opacity: 255,
        nodes: vec![node],
        rotate: 0.,
        scale: [1., 1.],
    });
    let (paint, uv) = region_paint(&inside, region)?;
    let mut draw = spec(0, region, paint);
    draw.uv = Some(uv);
    s.nodes.push(CanvasNode::Draw { layer: draw });
    Ok(())
}
/// A frozen region: `nodes` (surface coordinates) rasterized into a texture
/// the size of `region` only, under a root translated by `-region.xy`.
/// Returns the snapshot paint and the uv of `region` in it.
fn region_paint(nodes: &[CanvasNode], region: [f32; 4]) -> Result<(Paint, [f32; 4])> {
    let size = [region[2].ceil().max(1.) as u32, region[3].ceil().max(1.) as u32];
    let root = [-region[0], -region[1], region[0] + size[0] as f32, region[1] + size[1] as f32];
    let mut layers = vec![spec(1, root, Paint::Solid { color: [0; 4] })];
    let mut next_id = 1;
    snapshot_specs(nodes, 1, &mut next_id, &mut layers)?;
    let paint = Paint::Snapshot { size, layers: layers.into() };
    paint.validate()?;
    Ok((paint, [0., 0., region[2] / size[0] as f32, region[3] / size[1] as f32]))
}
/// A region snapshot made by this module (full-surface frozen tree shown
/// through a pixel crop equal to its rect): inlinable as a clipped subtree.
fn region_snapshot(layer: &LayerSpec) -> Option<&[LayerSpec]> {
    let Paint::Snapshot { size, layers } = &layer.paint else { return None };
    let [w, h] = [size[0] as f32, size[1] as f32];
    let r = layer.rect;
    // `region_paint` shape: a region-sized texture under a root at -region.xy.
    let crop = [0., 0., r[2] / w, r[3] / h];
    let t = &layer.transform;
    let root_full = layers
        .iter()
        .find(|l| l.parent.is_none())
        .is_some_and(|l| l.rect[0] == -r[0] && l.rect[1] == -r[1] && [w, h] == [r[2].ceil().max(1.), r[3].ceil().max(1.)]);
    (root_full
        && layer.uv.is_some_and(|uv| uv.iter().zip(crop).all(|(a, b)| (a - b).abs() < 1e-6))
        && layer.opacity == 255
        && layer.visible
        && layer.clip.is_none()
        && layer.filter.is_none()
        && matches!(layer.blend, crate::layer::Blend::Normal)
        && t.rotate == 0.
        && t.scale_x == 1.
        && t.scale_y == 1.)
        .then_some(layers.as_slice())
}
/// Canvas nodes as snapshot-local LayerSpecs under `parent` (ids from 2).
fn snapshot_specs(nodes: &[CanvasNode], parent: u32, next_id: &mut u32, out: &mut Vec<LayerSpec>) -> Result<()> {
    for (i, node) in nodes.iter().enumerate() {
        *next_id += 1;
        ensure!(*next_id <= 4096, "canvas snapshot node budget");
        let id = *next_id;
        if let CanvasNode::Draw { layer } = node {
            if let Some(inner) = region_snapshot(layer) {
                // Inline instead of nesting: remap ids, hang the frozen root
                // under `parent`, clipped to the region it showed.
                let mut map = std::collections::BTreeMap::new();
                for l in inner {
                    *next_id += 1;
                    ensure!(*next_id <= 4096, "canvas snapshot node budget");
                    map.insert(l.id, *next_id);
                }
                for l in inner {
                    let mut l = l.clone();
                    l.id = map[&l.id];
                    match l.parent {
                        Some(p) => l.parent = Some(map[&p]),
                        None => {
                            // Back in surface coordinates.
                            l.rect[0] = 0.;
                            l.rect[1] = 0.;
                            l.parent = Some(parent);
                            l.order = [0, i as i32];
                            l.clip = Some(layer.rect);
                        }
                    }
                    out.push(l);
                }
                continue;
            }
        }
        let mut s = match node {
            CanvasNode::Draw { layer } => layer.clone(),
            CanvasNode::Group { offset, clip, opacity, rotate, scale, .. } => {
                let mut x = spec(id, [offset[0], offset[1], 1., 1.], Paint::Solid { color: [0; 4] });
                x.clip = Some(*clip);
                x.opacity = *opacity;
                x.transform.rotate = *rotate;
                x.transform.scale_x = scale[0];
                x.transform.scale_y = scale[1];
                x
            }
        };
        s.id = id;
        s.parent = Some(parent);
        s.order = [0, i as i32];
        out.push(s);
        if let CanvasNode::Group { nodes, .. } = node {
            snapshot_specs(nodes, id, next_id, out)?;
        }
    }
    Ok(())
}
fn emit(
    nodes: &[CanvasNode],
    parent: u32,
    order: i32,
    draw_order: &mut i32,
    serial: &mut u32,
    ids: &mut Vec<u32>,
    commands: &mut Vec<LayerCommand>,
) -> Result<()> {
    for node in nodes {
        *serial = serial.checked_add(1).context("canvas identity exhausted")?;
        ensure!(*serial < 0x1000_0000, "canvas identity limit");
        let id = 0x2000_0000 + *serial;
        ids.push(id);
        *draw_order += 1;
        let mut s = match node {
            CanvasNode::Draw { layer } => layer.clone(),
            CanvasNode::Group {
                offset,
                clip,
                opacity,
                rotate,
                scale,
                ..
            } => {
                let mut x = spec(
                    id,
                    [offset[0], offset[1], 1., 1.],
                    Paint::Solid { color: [0; 4] },
                );
                x.clip = Some(*clip);
                x.opacity = *opacity;
                x.transform.rotate = *rotate;
                x.transform.scale_x = scale[0];
                x.transform.scale_y = scale[1];
                x
            }
        };
        s.id = id;
        s.parent = Some(parent);
        s.order = [order, *draw_order];
        commands.push(LayerCommand::Put { layer: s });
        if let CanvasNode::Group { nodes, .. } = node {
            emit(nodes, id, order, draw_order, serial, ids, commands)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod subtract_tests {
    use super::subtract_rect;
    #[test]
    fn complement_bands_cover_the_rest_exactly() {
        let full = [0., 0., 10., 10.];
        let bands = subtract_rect(full, [2., 3., 4., 5.]);
        let area: f32 = bands.iter().map(|b| b[2] * b[3]).sum();
        assert_eq!(area, 100. - 20.);
        assert_eq!(bands.len(), 4);
        assert_eq!(subtract_rect(full, [0., 0., 10., 10.]).len(), 0);
        assert_eq!(subtract_rect(full, [20., 20., 1., 1.]), vec![full]);
    }
}
