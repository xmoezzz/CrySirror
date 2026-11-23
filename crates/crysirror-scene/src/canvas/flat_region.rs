//! Recover a bounded 1:1 bitmap capture when repeated scroll blits have
//! accumulated more frozen pages than a single nested snapshot can hold.
use super::*;

pub(super) fn flatten(nodes: &[CanvasNode], region: [f32; 4], size: [u32; 2]) -> Option<Paint> {
    let mut out = Builder {
        origin: [region[0], region[1]],
        layers: vec![spec(
            1,
            [0., 0., size[0] as f32, size[1] as f32],
            Paint::Solid { color: [0; 4] },
        )],
        visits: 0,
        serial: 1,
    };
    out.nodes(nodes, [0.; 2], region)?;
    let paint = Paint::Snapshot {
        size,
        layers: out.layers.into(),
    };
    paint.validate().ok()?;
    Some(paint)
}

/// Rebuild the frozen bitmap in its own pixel space. The containing layer
/// still applies its UV, placement, clip, opacity and blend exactly once.
pub(crate) fn flatten_snapshot(paint: &Paint) -> Option<Paint> {
    let Paint::Snapshot { size, .. } = paint else { return None };
    let rect = [0., 0., size[0] as f32, size[1] as f32];
    let bitmap = CanvasNode::Draw { layer: spec(0, rect, paint.clone()) };
    flatten(std::slice::from_ref(&bitmap), rect, *size)
}

struct Builder {
    origin: [f32; 2],
    layers: Vec<LayerSpec>,
    visits: usize,
    serial: u32,
}

fn translated(mut rect: [f32; 4], offset: [f32; 2]) -> [f32; 4] {
    rect[0] += offset[0];
    rect[1] += offset[1];
    rect
}

impl Builder {
    fn visit(&mut self) -> Option<()> {
        self.visits += 1;
        (self.visits <= 32768).then_some(())
    }

    fn nodes(&mut self, nodes: &[CanvasNode], offset: [f32; 2], clip: [f32; 4]) -> Option<()> {
        for node in nodes {
            self.visit()?;
            match node {
                CanvasNode::Group {
                    offset: delta,
                    clip: local,
                    opacity: 255,
                    rotate,
                    scale,
                    nodes,
                } if *rotate == 0. && *scale == [1., 1.] => {
                    let at = [offset[0] + delta[0], offset[1] + delta[1]];
                    if let Some(visible) = intersect(clip, translated(*local, at)) {
                        self.nodes(nodes, at, visible)?;
                    }
                }
                CanvasNode::Draw { layer }
                    if layer.visible
                        && layer.transform == SpriteTransform::default()
                        && layer.filter.is_none()
                        && layer.source_crop.is_none()
                        && layer.viewport_clip.is_none()
                        && layer.pose_copies.is_none()
                        && matches!(layer.blend, Blend::Normal | Blend::AtopCopy) =>
                {
                    let rect = translated(layer.rect, offset);
                    let local_clip = layer.clip.map(|c| translated(c, [rect[0], rect[1]]));
                    if let Some(clip) = local_clip.map_or(Some(clip), |c| intersect(c, clip)) {
                        self.draw(
                            &layer.paint,
                            rect,
                            layer.uv.unwrap_or([0., 0., 1., 1.]),
                            clip,
                            layer.opacity,
                            layer.sampling,
                            layer.blend,
                            true,
                        )?;
                    }
                }
                CanvasNode::Draw { layer } if !layer.visible => {}
                _ => return None,
            }
        }
        Some(())
    }

    fn draw(
        &mut self,
        paint: &Paint,
        rect: [f32; 4],
        uv: [f32; 4],
        clip: [f32; 4],
        opacity: u8,
        sampling: crate::layer::ImageSampling,
        blend: Blend,
        isolated: bool,
    ) -> Option<()> {
        let Some(r) = intersect(rect, clip) else {
            return Some(());
        };
        match paint {
            Paint::Solid { color } if color[3] == 0 && blend == Blend::Normal => {}
            Paint::Image { .. } | Paint::Solid { .. } => {
                if !matches!(blend, Blend::Normal | Blend::AtopCopy) {
                    return None;
                }
                let local = translated(r, [-self.origin[0], -self.origin[1]]);
                if blend == Blend::AtopCopy {
                    if !isolated {
                        return None;
                    }
                    // CopyRgb replaces RGB throughout its rectangle and keeps
                    // alpha. Earlier such writes it covers cannot affect the
                    // result, including the alpha of intervening normal draws.
                    self.layers.retain(|layer| {
                        layer.blend != Blend::AtopCopy || !contains(local, layer.rect)
                    });
                }
                if self.layers.len() >= 4096 {
                    return None;
                }
                self.serial += 1;
                let id = self.serial;
                let mut leaf = spec(id, local, paint.clone());
                let u = |x| uv[0] + (x - rect[0]) * (uv[2] - uv[0]) / rect[2];
                let v = |y| uv[1] + (y - rect[1]) * (uv[3] - uv[1]) / rect[3];
                leaf.uv = Some([u(r[0]), v(r[1]), u(r[0] + r[2]), v(r[1] + r[3])]);
                leaf.parent = Some(1);
                leaf.order = [0, id as i32];
                leaf.opacity = opacity;
                leaf.sampling = sampling;
                leaf.blend = blend;
                self.layers.push(leaf);
            }
            Paint::Snapshot { size, layers } if opacity == 255 && blend == Blend::Normal => {
                let [w, h] = size.map(|n| n as f32);
                let at = [rect[0] - uv[0] * w, rect[1] - uv[1] * h];
                if ((uv[2] - uv[0]) * w - rect[2]).abs() > 0.001
                    || ((uv[3] - uv[1]) * h - rect[3]).abs() > 0.001
                    || at.iter().any(|v| (v - v.round()).abs() > 0.001)
                {
                    return None;
                }
                let mut source = LayerSystem::default();
                source.install(ClientId(0), layers).ok()?;
                let local = translated(r, [-self.origin[0], -self.origin[1]]);
                let isolated = isolated
                    && !self
                        .layers
                        .iter()
                        .skip(1)
                        .any(|s| intersect(s.rect, local).is_some());
                for p in source.frame(*size).ok()? {
                    self.visit()?;
                    if !matches!(p.blend, Blend::Normal | Blend::AtopCopy)
                        || p.filter.is_some()
                        || p.composite.is_some()
                        || p.puppet.is_some()
                        || p.output.is_some()
                        || p.target.is_some()
                        || p.coverage != crate::layer::coverage::Operation::None
                    {
                        return None;
                    }
                    let [a, b, c, d] = p.quad;
                    if a[1] != b[1]
                        || b[0] != c[0]
                        || c[1] != d[1]
                        || d[0] != a[0]
                        || b[0] <= a[0]
                        || d[1] <= a[1]
                    {
                        return None;
                    }
                    let Some(visible) = intersect(r, translated(p.clip?, at)) else {
                        continue;
                    };
                    self.draw(
                        &p.paint,
                        translated([a[0], a[1], b[0] - a[0], d[1] - a[1]], at),
                        p.uv,
                        visible,
                        p.opacity,
                        p.sampling,
                        p.blend,
                        isolated,
                    )?;
                }
            }
            _ => return None,
        }
        Some(())
    }
}
