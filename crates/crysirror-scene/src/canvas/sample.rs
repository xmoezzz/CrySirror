//! Exact retained-solid pixel queries. This deliberately does not rasterize
//! images, transformed groups, text or effects and does not invent a matte.
use super::{CanvasNode, CanvasSystem};
use crate::layer::{Blend, LayerSpec, Paint};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::state::ClientId;

impl CanvasSystem {
    /// Read one straight RGBA pixel in the bounded opaque-solid profile.
    /// Known empty/transparent coverage returns transparent black. A covering
    /// opaque normal solid determines the result independently of older nodes;
    /// otherwise a touched unsupported operation fails instead of guessing.
    /// The retained state is immutable during this query; no asset/device read
    /// or second compositor is involved.
    pub fn sample_pixel(&self, client: ClientId, id: u32, point: [u32; 2]) -> Result<[u8; 4]> {
        let surface = self
            .surface(client, id)
            .context("canvas pixel surface missing")?;
        ensure!(
            point[0] < surface.width && point[1] < surface.height,
            "canvas pixel point outside surface"
        );
        ensure!(surface.nodes.len() <= 4096, "canvas pixel node budget");
        let p = [point[0] as f32 + 0.5, point[1] as f32 + 0.5];
        for node in surface.nodes.iter().rev() {
            match node {
                CanvasNode::Draw { layer } => {
                    if let Some(color) = sample_draw(layer, p)? {
                        return Ok(color);
                    }
                }
                CanvasNode::Group { opacity: 0, .. } => {}
                CanvasNode::Group { .. } => {
                    bail!("canvas pixel sampling requires a raster backend for groups")
                }
            }
        }
        Ok([0; 4])
    }
}

fn contains([x, y, w, h]: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= x && p[1] >= y && p[0] < x + w && p[1] < y + h
}

fn sample_draw(layer: &LayerSpec, point: [f32; 2]) -> Result<Option<[u8; 4]>> {
    if !layer.visible || layer.opacity == 0 {
        return Ok(None);
    }
    let t = &layer.transform;
    ensure!(
        layer.parent.is_none()
            && t.z == 0.
            && t.scale_x == 1.
            && t.scale_y == 1.
            && t.scale_z == 1.
            && t.rotate == 0.
            && t.rotate_x == 0.
            && t.rotate_y == 0.
            && t.pivot_x == 0.
            && t.pivot_y == 0.
            && t.pivot_z == 0.
            && !t.object_anchor
            && t.texture_center_x == 0.
            && t.texture_center_y == 0.
            && !t.camera_enabled
            && !t.billboard
            && !t.culling,
        "canvas pixel sampling requires a raster backend for transforms/parents"
    );
    ensure!(
        layer.filter.is_none(),
        "canvas pixel sampling requires a raster backend for filters"
    );
    if !contains(layer.rect, point) {
        return Ok(None);
    }
    // Layer clips are sprite-local, unlike the surface-space query point.
    if layer
        .clip
        .is_some_and(|clip| !contains(clip, [point[0] - layer.rect[0], point[1] - layer.rect[1]]))
    {
        return Ok(None);
    }
    ensure!(
        layer.blend == Blend::Normal,
        "canvas pixel sampling requires a raster backend for blending"
    );
    match layer.paint {
        Paint::Solid { color } if color[3] == 0 => Ok(None),
        Paint::Solid { color } if color[3] == 255 && layer.opacity == 255 => Ok(Some(color)),
        Paint::Solid { .. } => {
            bail!("canvas pixel sampling requires a raster backend for translucent solids")
        }
        _ => bail!("canvas pixel sampling requires a raster backend for non-solid paint"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CanvasSurface, spec};
    use super::*;
    use std::sync::Arc;

    fn canvas(nodes: Vec<CanvasNode>) -> CanvasSystem {
        let mut c = CanvasSystem::default();
        c.clients.entry(1).or_default().insert(
            7,
            Arc::new(CanvasSurface {
                width: 20,
                height: 10,
                nodes,
            }),
        );
        c
    }
    fn solid(rect: [f32; 4], color: [u8; 4]) -> CanvasNode {
        CanvasNode::Draw {
            layer: spec(0, rect, Paint::Solid { color }),
        }
    }

    #[test]
    fn exact_opaque_and_transparent_coverage_has_no_implicit_matte() {
        let c = canvas(vec![solid([2., 3., 5., 2.], [17, 81, 202, 255])]);
        assert_eq!(
            c.sample_pixel(ClientId(1), 7, [2, 3]).unwrap(),
            [17, 81, 202, 255]
        );
        assert_eq!(
            c.sample_pixel(ClientId(1), 7, [6, 4]).unwrap(),
            [17, 81, 202, 255]
        );
        assert_eq!(c.sample_pixel(ClientId(1), 7, [7, 4]).unwrap(), [0; 4]);
        assert_eq!(
            canvas(vec![]).sample_pixel(ClientId(1), 7, [0, 0]).unwrap(),
            [0; 4]
        );
        assert!(
            canvas(vec![solid([0., 0., 20., 10.], [10, 20, 30, 128])])
                .sample_pixel(ClientId(1), 7, [0, 0])
                .is_err()
        );
    }

    #[test]
    fn clips_are_local_and_uncovered_unknown_pixels_fail() {
        let mut unknown = spec(
            0,
            [0., 0., 20., 10.],
            Paint::Image {
                asset: "unknown.png".into(),
                tint: [255; 4],
            },
        );
        let mut foreground = spec(
            1,
            [3., 2., 10., 6.],
            Paint::Solid {
                color: [90, 80, 70, 255],
            },
        );
        foreground.clip = Some([1., 1., 2., 2.]);
        let c = canvas(vec![
            CanvasNode::Draw {
                layer: unknown.clone(),
            },
            CanvasNode::Draw { layer: foreground },
        ]);
        assert_eq!(
            c.sample_pixel(ClientId(1), 7, [4, 3]).unwrap(),
            [90, 80, 70, 255]
        );
        assert!(c.sample_pixel(ClientId(1), 7, [3, 3]).is_err());
        unknown.transform.scale_x = 2.;
        let c = canvas(vec![
            CanvasNode::Draw { layer: unknown },
            solid([0., 0., 20., 10.], [1, 2, 3, 255]),
        ]);
        assert_eq!(
            c.sample_pixel(ClientId(1), 7, [10, 5]).unwrap(),
            [1, 2, 3, 255]
        );
    }

    #[test]
    fn missing_scoped_surface_bounds_and_unsupported_operations_fail() {
        let c = canvas(vec![]);
        for (client, id, p) in [
            (ClientId(2), 7, [0, 0]),
            (ClientId(1), 8, [0, 0]),
            (ClientId(1), 7, [20, 0]),
            (ClientId(1), 7, [0, 10]),
        ] {
            assert!(c.sample_pixel(client, id, p).is_err());
        }
        let mut layer = spec(
            0,
            [0., 0., 20., 10.],
            Paint::Solid {
                color: [1, 2, 3, 255],
            },
        );
        layer.blend = Blend::Add;
        assert!(
            canvas(vec![CanvasNode::Draw { layer }])
                .sample_pixel(ClientId(1), 7, [0, 0])
                .is_err()
        );
        assert!(
            canvas(vec![CanvasNode::Group {
                offset: [0., 0.],
                clip: [0., 0., 20., 10.],
                opacity: 255,
                nodes: vec![],
                rotate: 0.,
                scale: [1., 1.]
            }])
            .sample_pixel(ClientId(1), 7, [0, 0])
            .is_err()
        );
    }
}
