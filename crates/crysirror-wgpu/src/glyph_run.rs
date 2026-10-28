// SPDX-License-Identifier: MPL-2.0
//! GPU extraction of retained positioned glyphs. Each glyph is a resource and
//! a quad; no source engine or whole-run framebuffer participates.
use super::Renderer;
use anyhow::{Context, Result, ensure};
use crysirror_runtime::assets::Assets;
use crysirror_scene::{
    Primitive,
    layer::{Paint, PositionedGlyph},
};
use crysirror_text::glyph::GlyphRaster;

pub(super) struct GlyphBitmap {
    pub origin: [i32; 2],
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
fn phase(baseline: [f32; 2]) -> [f32; 2] {
    baseline.map(|v| {
        let p = v - v.floor();
        if p == 0. { 0. } else { p }
    })
}
pub(super) fn key(assets: &Assets, paint: &Paint) -> Result<String> {
    let Paint::GlyphRun {
        font,
        size,
        bold,
        glyphs,
        blur,
        ..
    } = paint
    else {
        anyhow::bail!("expected extracted glyph paint");
    };
    ensure!(glyphs.len() == 1, "glyph draw must have one glyph");
    let g = &glyphs[0];
    let request = GlyphRaster {
        glyph_id: assets.text.glyph_id(font, g.character)?,
        size: *size,
        baseline: phase(g.baseline),
    };
    Ok(format!(
        "{}:bold={bold}{}",
        assets.text.glyph_cell_key(font, request)?,
        blur_key(blur)
    ))
}
fn blur_key(blur: &Option<crysirror_scene::layer::GlyphBlur>) -> String {
    blur.map_or(String::new(), |b| format!(":blur={},{}", b.radius, b.level))
}
impl Renderer {
    pub(super) fn expand_glyph_runs(
        &mut self,
        frame: Vec<Primitive>,
        assets: &Assets,
    ) -> Result<Vec<Primitive>> {
        // Drop glyphs unused by this frame before budgeting fresh resources;
        // changing a complete paragraph must not count both paragraphs forever.
        let mut needed = std::collections::BTreeSet::new();
        for p in &frame {
            if let Paint::GlyphRun {
                font,
                size,
                bold,
                glyphs,
                blur,
                ..
            } = &p.paint
            {
                p.paint.validate()?;
                let visible = p.paint.glyph_reveal_status().expect("glyph run").visible;
                for g in glyphs.iter().take(visible) {
                    let request = GlyphRaster {
                        glyph_id: assets.text.glyph_id(font, g.character)?,
                        size: g.size.unwrap_or(*size),
                        baseline: phase(g.baseline),
                    };
                    needed.insert(format!(
                        "{}:bold={}{}",
                        assets.text.glyph_cell_key(font, request)?,
                        g.bold.unwrap_or(*bold),
                        blur_key(blur)
                    ));
                }
            }
        }
        self.glyph_cells.retain(|key, _| needed.contains(key));
        let mut expanded = Vec::new();
        let mut count = 0usize;
        let mut bytes: usize = self.glyph_cells.values().map(|c| c.rgba.len()).sum();
        for p in frame {
            let Paint::GlyphRun {
                font,
                size,
                color,
                bold,
                glyphs,
                blur,
                ..
            } = &p.paint
            else {
                expanded.push(p);
                continue;
            };
            p.paint.validate()?;
            ensure!(
                p.uv == [0., 0., 1., 1.],
                "glyph runs do not take image UV crops"
            );
            let visible = p.paint.glyph_reveal_status().expect("glyph run").visible;
            for g in glyphs.iter().take(visible) {
                count = count.checked_add(1).context("glyph frame count overflow")?;
                ensure!(count <= 65_536, "positioned glyph frame budget");
                let size = g.size.unwrap_or(*size);
                let color = g.color.unwrap_or(*color);
                let bold = g.bold.unwrap_or(*bold);
                let request = GlyphRaster {
                    glyph_id: assets.text.glyph_id(font, g.character)?,
                    size,
                    baseline: phase(g.baseline),
                };
                let paint = Paint::GlyphRun {
                    font: font.clone(),
                    size,
                    color,
                    bold,
                    glyphs: vec![PositionedGlyph {
                        character: g.character,
                        baseline: request.baseline,
                        size: None,
                        color: None,
                        bold: None,
                    }],
                    reveal: None,
                    blur: *blur,
                };
                let key = key(assets, &paint)?;
                if !self.glyph_cells.contains_key(&key) {
                    let cell = assets.text.rasterize_glyph_cell(font, request)?;
                    let width = cell.width + u32::from(bold && cell.width > 0);
                    let length = width as usize * cell.height as usize * 4;
                    ensure!(
                        bytes
                            .checked_add(length)
                            .is_some_and(|n| n <= 64 * 1024 * 1024),
                        "positioned glyph resource budget exceeds 64 MiB"
                    );
                    bytes += length;
                    let mut rgba = vec![0u8; length];
                    for y in 0..cell.height as usize {
                        for x in 0..cell.width as usize {
                            let alpha = (cell.coverage[y * cell.width as usize + x].clamp(0., 1.)
                                * 255.)
                                .round() as u8;
                            let at = (y * width as usize + x) * 4;
                            rgba[at..at + 4].copy_from_slice(&[255, 255, 255, alpha]);
                        }
                        if bold && width > 0 {
                            for x in (1..width as usize).rev() {
                                let at = (y * width as usize + x) * 4;
                                rgba[at..at + 3].fill(255);
                                rgba[at + 3] = rgba[at + 3].max(rgba[at - 1]);
                            }
                        }
                    }
                    let mut bitmap = GlyphBitmap {
                        origin: cell.origin,
                        width,
                        height: cell.height,
                        rgba,
                    };
                    if let Some(b) = blur {
                        let coverage: Vec<u8> = bitmap.rgba.chunks_exact(4).map(|p| p[3]).collect();
                        let (out, w, h) = crysirror_text::glyph_blur::blur_coverage(
                            &coverage,
                            bitmap.width as usize,
                            bitmap.height as usize,
                            u32::from(b.radius),
                            i32::from(b.level),
                        );
                        let r = i32::from(b.radius);
                        bytes += (w * h * 4).saturating_sub(bitmap.rgba.len());
                        ensure!(bytes <= 64 * 1024 * 1024, "positioned glyph resource budget exceeds 64 MiB");
                        bitmap = GlyphBitmap {
                            origin: [bitmap.origin[0] - r, bitmap.origin[1] - r],
                            width: w as u32,
                            height: h as u32,
                            rgba: out.iter().flat_map(|&a| [255, 255, 255, a]).collect(),
                        };
                    }
                    self.glyph_cells.insert(key.clone(), bitmap);
                }
                let cell = &self.glyph_cells[&key];
                if cell.width == 0 || cell.height == 0 {
                    continue;
                }
                let origin = [
                    g.baseline[0].floor() + cell.origin[0] as f32,
                    g.baseline[1].floor() + cell.origin[1] as f32,
                ];
                let mut draw = p.clone();
                draw.quad = local_quad(
                    &p,
                    [origin[0], origin[1], cell.width as f32, cell.height as f32],
                )?;
                draw.size = [cell.width, cell.height];
                draw.extent = [cell.width as f32, cell.height as f32];
                draw.paint = paint;
                expanded.push(draw);
            }
        }
        Ok(expanded)
    }
}

/// Subrect extraction interpolates homogeneous positions, preserving parent
/// scale/rotation/projected camera transforms rather than treating text as a
/// screen-space overlay. Primitive stores divided xyz plus homogeneous w.
fn local_quad(p: &Primitive, rect: [f32; 4]) -> Result<[[f32; 4]; 4]> {
    ensure!(
        p.extent.iter().all(|v| v.is_finite() && *v > 0.),
        "glyph layer extent is empty"
    );
    let positions = p.quad.map(|[x, y, z, w]| [x * w, y * w, z * w, w]);
    let point = |x: f32, y: f32| -> Result<[f32; 4]> {
        let (u, v) = (x / p.extent[0], y / p.extent[1]);
        let h: [f32; 4] = std::array::from_fn(|i| {
            positions[0][i] * (1. - u) * (1. - v)
                + positions[1][i] * u * (1. - v)
                + positions[2][i] * u * v
                + positions[3][i] * (1. - u) * v
        });
        ensure!(
            h.iter().all(|v| v.is_finite()) && h[3].abs() > 1e-6,
            "glyph projection has invalid homogeneous coordinates"
        );
        Ok([h[0] / h[3], h[1] / h[3], h[2] / h[3], h[3]])
    };
    let [x, y, w, h] = rect;
    Ok([
        point(x, y)?,
        point(x + w, y)?,
        point(x + w, y + h)?,
        point(x, y + h)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Gpu;
    use crysirror_core::state::{ClientId, SceneState};
    use crysirror_scene::layer::{
        GlyphReveal, GlyphRevealUnit, LayerCommand, LayerSpec, LayerSystem, rectangle,
    };
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn glyph_subrects_preserve_homogeneous_and_fractional_local_coordinates() {
        let mut p = rectangle(
            1,
            1,
            0,
            [0., 0., 100., 100.],
            Paint::Solid { color: [255; 4] },
            None,
            [0, 0],
        );
        p.quad = [
            [10., 20., 0., 2.],
            [30., 20., 0., 1.],
            [30., 60., 0., 1.],
            [10., 60., 0., 2.],
        ];
        let q = local_quad(&p, [50., 50., 25., 25.]).unwrap();
        assert!((q[0][0] - 25. / 1.5).abs() < 1e-5);
        assert!((q[0][1] - 40.).abs() < 1e-5);
        assert!((q[0][3] - 1.5).abs() < 1e-5);
        let p = rectangle(
            1,
            1,
            0,
            [10., 20., 100.5, 50.5],
            Paint::Solid { color: [255; 4] },
            None,
            [0, 0],
        );
        let q = local_quad(&p, [10.25, 5.25, 20., 10.]).unwrap();
        assert!((q[0][0] - 20.25).abs() < 1e-5);
        assert!((q[0][1] - 25.25).abs() < 1e-5);
    }

    #[test]
    #[ignore = "requires a real GPU and CRYSIRROR_TEST_FONT pointing to a runtime system font; no font is bundled"]
    fn retained_glyphs_are_individual_cached_draws_with_shared_state_and_clip() {
        let font =
            std::env::var_os("CRYSIRROR_TEST_FONT").expect("supply a local system font path");
        let root = tempfile::tempdir().unwrap();
        let mut assets = Assets::new(root.path()).unwrap();
        assets
            .text
            .add_font("test", std::fs::read(font).unwrap())
            .unwrap();
        let spec: LayerSpec = serde_json::from_value(json!({"id":1,"rect":[0.,0.,140.,80.],
            "clip":[0.,0.,90.,80.],"paint":{"kind":"glyph_run","font":"test","size":36.,
            "color":[255,0,0,255],"glyphs":[{"character":"A","baseline":[5.,45.]},
                {"character":"B","baseline":[75.,45.],"color":[0,255,0,255],"bold":true}]}}))
        .unwrap();
        let mut layers = LayerSystem::default();
        layers.install(ClientId(1), &[spec.clone()]).unwrap();
        let instance = wgpu::Instance::default();
        let gpu = Arc::new(pollster::block_on(Gpu::request(&instance, None)).unwrap());
        let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer
            .set_engine_frame(layers.frame([140, 80]).unwrap())
            .unwrap();
        let (vertices, draws) = renderer
            .prepare(&SceneState::default(), &mut assets, [140, 80])
            .unwrap();
        assert_eq!(draws.len(), 2, "two glyphs must not become one run texture");
        assert_eq!(vertices.len(), 12);
        assert_eq!(renderer.glyph_cells.len(), 2);
        assert!(draws.iter().all(|d| d.key.starts_with("glyph:")));
        let cached: Vec<_> = renderer.glyph_cells.keys().cloned().collect();
        let pixels = renderer
            .capture(
                &SceneState::default(),
                &mut assets,
                [140, 80],
                [0, 0, 0, 255],
            )
            .unwrap();
        assert!(
            pixels
                .enumerate_pixels()
                .any(|(x, _, p)| x < 60 && p.0[0] > 0 && p.0[1] == 0)
        );
        assert!(
            pixels
                .enumerate_pixels()
                .any(|(x, _, p)| (75..90).contains(&x) && p.0[1] > 0 && p.0[0] == 0)
        );
        assert!(
            pixels
                .enumerate_pixels()
                .filter(|(x, _, _)| *x >= 90)
                .all(|(_, _, p)| p.0 == [0, 0, 0, 255]),
            "retained clip must apply to every glyph"
        );
        if let Some(path) = std::env::var_os("CRYSIRROR_GLYPH_CAPTURE") {
            pixels.save(path).unwrap();
        }
        let mut moved = layers.frame([140, 80]).unwrap();
        for point in &mut moved[0].quad {
            point[0] += 10.;
        }
        renderer.set_engine_frame(moved).unwrap();
        let (_, draws) = renderer
            .prepare(&SceneState::default(), &mut assets, [140, 80])
            .unwrap();
        assert_eq!(draws.len(), 2);
        assert_eq!(
            renderer.glyph_cells.keys().cloned().collect::<Vec<_>>(),
            cached,
            "integer translation reuses glyph resources"
        );
        let mut delayed = spec;
        if let Paint::GlyphRun { reveal, .. } = &mut delayed.paint {
            *reveal = Some(GlyphReveal {
                unit: GlyphRevealUnit::UnicodeScalar,
                interval_ms: 20,
                initial_visible: 0,
                elapsed_ms: 0,
                forced_complete: false,
                paused: false,
            });
        }
        layers
            .apply_batch(ClientId(1), &[LayerCommand::Put { layer: delayed }])
            .unwrap();
        renderer
            .set_engine_frame(layers.frame([140, 80]).unwrap())
            .unwrap();
        let (_, draws) = renderer
            .prepare(&SceneState::default(), &mut assets, [140, 80])
            .unwrap();
        assert!(
            draws.is_empty(),
            "unrevealed glyphs must not create GPU draws"
        );
        layers.tick(20);
        renderer
            .set_engine_frame(layers.frame([140, 80]).unwrap())
            .unwrap();
        let first = renderer
            .capture(
                &SceneState::default(),
                &mut assets,
                [140, 80],
                [0, 0, 0, 255],
            )
            .unwrap();
        assert!(first.pixels().any(|p| p.0[0] > 0));
        assert!(
            first.pixels().all(|p| p.0[1] == 0),
            "the second green glyph is not visible yet"
        );
        layers
            .apply_batch(ClientId(1), &[LayerCommand::CompleteReveal { id: 1 }])
            .unwrap();
        renderer
            .set_engine_frame(layers.frame([140, 80]).unwrap())
            .unwrap();
        let (_, draws) = renderer
            .prepare(&SceneState::default(), &mut assets, [140, 80])
            .unwrap();
        assert_eq!(
            draws.len(),
            2,
            "completion must show all glyph draws without a VM timer"
        );
        layers.set_visible(ClientId(1), 1, false).unwrap();
        renderer
            .set_engine_frame(layers.frame([140, 80]).unwrap())
            .unwrap();
        let blank = renderer
            .capture(
                &SceneState::default(),
                &mut assets,
                [140, 80],
                [0, 0, 0, 255],
            )
            .unwrap();
        assert!(blank.pixels().all(|p| p.0 == [0, 0, 0, 255]));
        assert!(renderer.glyph_cells.is_empty());
    }
}
