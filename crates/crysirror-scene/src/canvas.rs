// SPDX-License-Identifier: MPL-2.0
//! Engine-owned retained drawing surfaces. Copies snapshot logical draw nodes,
//! not another engine's framebuffer. Presentation remains ordinary Layer nodes.
use crate::layer::{
    Blend, LayerCommand, LayerSpec, LayerSystem, Paint, SpriteTransform, intersect,
};
use anyhow::{Context, Result, ensure};
use crysirror_core::state::ClientId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanvasCommand {
    Create {
        id: u32,
        width: u32,
        height: u32,
    },
    Clear {
        id: u32,
        color: [u8; 4],
    },
    Draw {
        id: u32,
        layer: LayerSpec,
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
    },
    Present {
        id: u32,
        layer: u32,
        #[serde(default)]
        order: i32,
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
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasSystem {
    clients: BTreeMap<u32, BTreeMap<u32, CanvasSurface>>,
    presentations: BTreeMap<u32, BTreeMap<u32, Presentation>>,
    #[serde(default)]
    serial: BTreeMap<u32, u32>,
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
        filter: None,
    }
}
impl CanvasSystem {
    pub fn surface(&self, client: ClientId, id: u32) -> Option<&CanvasSurface> {
        self.clients.get(&client.0)?.get(&id)
    }
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients.keys().copied()
    }
    pub fn validate(&self) -> Result<()> {
        for owner in self.clients.values() {
            ensure!(owner.len() <= 128, "canvas surface limit");
            for s in owner.values() {
                ensure!(
                    s.width > 0 && s.height > 0 && s.width <= 8192 && s.height <= 8192,
                    "canvas dimensions"
                );
                let mut count = 0;
                validate_nodes(&s.nodes, 0, &mut count)?;
            }
        }
        Ok(())
    }
    fn get_mut(&mut self, c: ClientId, id: u32) -> Result<&mut CanvasSurface> {
        self.clients
            .get_mut(&c.0)
            .and_then(|x| x.get_mut(&id))
            .context("canvas absent")
    }
    pub fn apply_batch(
        &mut self,
        c: ClientId,
        commands: &[CanvasCommand],
        layers: &mut LayerSystem,
    ) -> Result<()> {
        ensure!(commands.len() <= 4096, "canvas batch limit");
        let mut next = self.clone();
        let mut next_layers = layers.clone();
        let mut changed = false;
        for command in commands {
            match command {
                CanvasCommand::Create { id, width, height } => {
                    ensure!(
                        *width > 0 && *height > 0 && *width <= 8192 && *height <= 8192,
                        "canvas dimensions"
                    );
                    next.clients.entry(c.0).or_default().insert(
                        *id,
                        CanvasSurface {
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
                        },
                    );
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
                CanvasCommand::Draw { id, layer } => {
                    next.get_mut(c, *id)?.nodes.push(CanvasNode::Draw {
                        layer: layer.clone(),
                    })
                }
                CanvasCommand::Set { id, layer } => {
                    next.get_mut(c, *id)?.nodes = vec![CanvasNode::Draw {
                        layer: layer.clone(),
                    }]
                }
                CanvasCommand::Copy {
                    source,
                    destination,
                    rect,
                    position,
                    opacity,
                    replace,
                } => {
                    ensure!(
                        rect.iter()
                            .chain(position)
                            .all(|v| v.is_finite() && v.abs() <= 1_000_000.)
                            && rect[2] > 0.
                            && rect[3] > 0.,
                        "canvas copy bounds"
                    );
                    ensure!(
                        *opacity == 255,
                        "isolated canvas copy opacity requires a render-target composition API"
                    );
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
                    if *replace
                        && *rect == [0., 0., source.width as f32, source.height as f32]
                        && *position == [0., 0.]
                        && dst.width == source.width
                        && dst.height == source.height
                    {
                        dst.nodes = source.nodes;
                    } else {
                        if *replace {
                            ensure!(
                                clip[0] + offset[0] == 0.
                                    && clip[1] + offset[1] == 0.
                                    && clip[2] == dst.width as f32
                                    && clip[3] == dst.height as f32,
                                "partial replace canvas copy requires a region render-target API"
                            );
                            dst.nodes.clear();
                        }
                        dst.nodes.push(CanvasNode::Group {
                            offset,
                            clip,
                            opacity: *opacity,
                            nodes: source.nodes,
                        });
                    }
                }
                CanvasCommand::Present { id, layer, order } => {
                    ensure!(next.surface(c, *id).is_some(), "present canvas absent");
                    ensure!(*layer < 0x2000_0000, "reserved canvas layer id");
                    let owner = next.presentations.entry(c.0).or_default();
                    if let Some(p) = owner.get_mut(layer) {
                        p.source = *id;
                        p.order = *order;
                    } else {
                        owner.insert(
                            *layer,
                            Presentation {
                                source: *id,
                                order: *order,
                                generated: vec![],
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
                            .map(|id| LayerCommand::Remove { id })
                            .collect::<Vec<_>>();
                        commands.push(LayerCommand::Remove { id: *layer });
                        next_layers.apply_batch(c, &commands)?;
                    } else {
                        continue;
                    }
                }
            }
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        next.validate()?;
        next.rebuild(c, &mut next_layers)?;
        *self = next;
        *layers = next_layers;
        Ok(())
    }
    fn rebuild(&mut self, c: ClientId, layers: &mut LayerSystem) -> Result<()> {
        let entries = self.presentations.get(&c.0).cloned().unwrap_or_default();
        for (root, p) in entries {
            let s = self
                .surface(c, p.source)
                .context("present source absent")?
                .clone();
            let mut commands = p
                .generated
                .iter()
                .rev()
                .map(|id| LayerCommand::Remove { id: *id })
                .collect::<Vec<_>>();
            let mut parent = spec(
                root,
                [0., 0., s.width as f32, s.height as f32],
                Paint::Solid { color: [0; 4] },
            );
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
            layers.apply_batch(c, &commands)?;
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
            } => {
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
                ..
            } => {
                let mut x = spec(
                    id,
                    [offset[0], offset[1], 1., 1.],
                    Paint::Solid { color: [0; 4] },
                );
                x.clip = Some(*clip);
                x.opacity = *opacity;
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
