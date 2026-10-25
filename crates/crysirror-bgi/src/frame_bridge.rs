// SPDX-License-Identifier: MPL-2.0
//! Native display plans rendered by shared CPU image and glyph services.
//! Native whole-frame rendering is not called here.
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{service::HostServices, state::ClientId};
use crysirror_image::{
    composition::{FloatBlend, FloatDraw, FloatSampling, ImageView},
    ImageSurface,
};
use crysirror_text::{glyph::GlyphRaster, TextSystem};
use ethornell_app::crysirror_frame::{FrameDraw, FramePlan, GlyphDraw};

pub struct FrameBridge {
    text: TextSystem,
}
pub struct FrameStats {
    pub width: u32,
    pub height: u32,
    pub image_draws: usize,
    pub glyph_draws: usize,
    /// Hash of native-format CPU bytes before XRGB display normalization.
    pub framebuffer_hash: u64,
}
impl FrameBridge {
    pub fn new(font_bytes: Vec<u8>, face_index: u32) -> Result<Self> {
        let mut text = TextSystem::default();
        text.add_font_face("bgi-session", font_bytes, face_index)?;
        Ok(Self { text })
    }
    pub fn compose(&self, plan: &FramePlan) -> Result<ImageSurface> {
        ensure!(plan.draws.len() <= 65_536, "BGI frame draw budget exceeded");
        let mut out = ImageSurface::filled(plan.width, plan.height, [0, 0, 0, 255])?;
        for draw in &plan.draws {
            match draw {
                FrameDraw::Image(draw) => {
                    let image = plan
                        .images
                        .get(&draw.key)
                        .with_context(|| format!("missing BGI draw image {}", draw.key))?;
                    let blend = match draw.blend_mode {
                        // Native BackF explicitly selects mode 1. Preserve the pinned
                        // Rust snapshot/GPU display contract here (float source-alpha
                        // coverage); this is not the raw 7-bit bitmap mode-1 kernel.
                        1 | 0x20 | 128 => FloatBlend::Coverage,
                        2 => FloatBlend::Add,
                        3 => FloatBlend::Subtract,
                        0xf0 => FloatBlend::ConstantInterpolation256,
                        value => bail!("unsupported BGI display blend selector {value:#x} image={} priority={}",draw.key,draw.native_priority),
                    };
                    out.draw_image(
                        ImageView {
                            width: image.width,
                            height: image.height,
                            rgba: &image.rgba,
                        },
                        FloatDraw {
                            destination: draw.destination,
                            source: draw.source,
                            opacity: draw.opacity,
                            ignore_source_alpha: draw.ignore_source_alpha,
                            blend,
                            quad: draw.destination_quad,
                            // CPU non-affine draw uses its nearest/clamped path; native mode-5
                            // quad sampling preserves its distinct half-texel/1/16 profile.
                            sampling: if draw.destination_quad.is_none() {
                                FloatSampling::NearestClamp
                            } else if draw.linear_sampling {
                                FloatSampling::Bilinear16Transparent
                            } else {
                                FloatSampling::NearestTransparent
                            },
                            clip: draw.clip,
                        },
                    )?;
                }
                FrameDraw::Glyph(glyph) => self.glyph(&mut out, glyph)?,
            }
        }
        Ok(out)
    }
    fn glyph(&self, out: &mut ImageSurface, glyph: &GlyphDraw) -> Result<()> {
        let mut cell = self.text.rasterize_glyph_cell(
            "bgi-session",
            GlyphRaster {
                glyph_id: glyph.glyph_id,
                size: glyph.size,
                baseline: glyph.baseline,
            },
        )?;
        cell.origin[0] = cell.origin[0].saturating_add(glyph.offset[0]);
        cell.origin[1] = cell.origin[1].saturating_add(glyph.offset[1]);
        if cell.width == 0 || cell.height == 0 {
            return Ok(());
        }
        if let Some(effect) = glyph.effect {
            ensure!(
                effect.mode == 2,
                "unsupported BGI glyph effect {}",
                effect.mode
            );
            ensure!(
                (0..=1024).contains(&effect.dx)
                    && (0..=1024).contains(&effect.dy)
                    && (0..=256).contains(&effect.concentration),
                "invalid BGI glyph glow"
            );
            let rgba = glyph.color.map(|c| (c.clamp(0., 1.) * 255.).round() as u8);
            let mut ink = ImageSurface::filled(cell.width, cell.height, [0; 4])?;
            for (p, a) in ink.rgba.chunks_exact_mut(4).zip(&cell.coverage) {
                let alpha = (a.clamp(0., 1.) * 255.).round() as u8;
                if alpha > 0 {
                    p.copy_from_slice(&[rgba[0], rgba[1], rgba[2], alpha]);
                }
            }
            let (dx, dy) = (effect.dx as u32, effect.dy as u32);
            let width = cell.width + 2 * dx;
            let height = cell.height + 2 * dy;
            ensure!(
                u64::from(width)
                    * u64::from(height)
                    * u64::from(2 * dx + 1)
                    * u64::from(2 * dy + 1)
                    <= 50_000_000,
                "BGI text glow work budget exceeded"
            );
            let mut glow = ImageSurface::filled(width, height, [0; 4])?;
            for y in 0..height as i32 {
                for x in 0..width as i32 {
                    let mut sum = 0u32;
                    for sy in (y - 2 * dy as i32)..=y {
                        for sx in (x - 2 * dx as i32)..=x {
                            if (0..cell.width as i32).contains(&sx)
                                && (0..cell.height as i32).contains(&sy)
                            {
                                sum += u32::from(
                                    ink.rgba
                                        [((sy as u32 * cell.width + sx as u32) * 4 + 3) as usize],
                                );
                            }
                        }
                    }
                    let at = ((y as u32 * width + x as u32) * 4) as usize;
                    glow.rgba[at..at + 4].copy_from_slice(&[
                        (effect.rgb >> 16) as u8,
                        (effect.rgb >> 8) as u8,
                        effect.rgb as u8,
                        sum.min(255) as u8,
                    ]);
                }
            }
            let mut record = ImageSurface::filled(width, height, [0; 4])?;
            crysirror_image::packed::blit(
                &mut record,
                2,
                &glow,
                2,
                0,
                0,
                1,
                256 - effect.concentration,
            )?;
            crysirror_image::packed::blit(&mut record, 2, &ink, 2, dx as i32, dy as i32, 0, 0)?;
            // The original record-to-XRGB kernel intentionally writes fourth-byte
            // payload, not display coverage. Normalize only when publishing.
            crysirror_image::packed::blit(
                out,
                1,
                &record,
                2,
                cell.origin[0],
                cell.origin[1],
                0,
                0,
            )?;
        } else {
            out.draw_coverage(
                cell.origin,
                cell.width,
                cell.height,
                &cell.coverage,
                glyph.color,
                glyph.bold,
                glyph.italic,
                glyph.clip,
            )?;
        }
        Ok(())
    }
    pub fn publish(
        &mut self,
        client: ClientId,
        plan: &FramePlan,
        host: &mut dyn HostServices,
    ) -> Result<FrameStats> {
        let mut image = self.compose(plan)?;
        let stats = FrameStats {
            width: plan.width,
            height: plan.height,
            image_draws: plan
                .draws
                .iter()
                .filter(|d| matches!(d, FrameDraw::Image(_)))
                .count(),
            glyph_draws: plan
                .draws
                .iter()
                .filter(|d| matches!(d, FrameDraw::Glyph(_)))
                .count(),
            framebuffer_hash: hash(&image),
        };
        // The BGI screen is an opaque XRGB display. Its byte 3 may hold bitmap
        // kernel payload and must not become sprite coverage in the host.
        for p in image.rgba.chunks_exact_mut(4) {
            p[3] = 255;
        }
        let path = host
            .present_surface(client, 0, image.width, image.height, &image.rgba)
            .map_err(|e| anyhow::anyhow!("present shared BGI composition: {e}"))?;
        let batch = serde_json::json!({"layers":[{"op":"put","layer":{"id":0,"rect":[0.,0.,image.width as f32,image.height as f32],"paint":{"kind":"image","asset":path.relative()}}}]});
        host.submit_engine(client, 1, &serde_json::to_vec(&batch)?)
            .map_err(|e| anyhow::anyhow!("publish BGI composed layer: {e}"))?;
        Ok(stats)
    }
}
fn hash(image: &ImageSurface) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in image
        .width
        .to_le_bytes()
        .into_iter()
        .chain(image.height.to_le_bytes())
        .chain(image.rgba.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{path::VfsPath, service::FileSystem, state::Command};
    use ethornell_app::crysirror_frame::ImageDraw;
    use std::collections::BTreeMap;
    fn plan() -> FramePlan {
        FramePlan {
            width: 4,
            height: 3,
            images: BTreeMap::from([(
                "decoded".into(),
                ethornell_app::crysirror_frame::FrameImage {
                    width: 2,
                    height: 1,
                    rgba: vec![200, 20, 0, 255, 0, 200, 20, 128],
                },
            )]),
            draws: vec![FrameDraw::Image(ImageDraw {
                key: "decoded".into(),
                destination: [1., 1., 2., 1.],
                source: [0., 0., 2., 1.],
                opacity: 0.5,
                ignore_source_alpha: false,
                blend_mode: 128,
                destination_quad: None,
                linear_sampling: false,
                clip: None,
                native_priority: 0,
            })],
        }
    }
    #[test]
    fn decoded_native_items_execute_the_shared_cpu_compositor() {
        let bridge = FrameBridge {
            text: TextSystem::default(),
        };
        let mut plan = plan();
        let image = bridge.compose(&plan).unwrap();
        assert_eq!(&image.rgba[20..28], &[100, 10, 0, 255, 0, 50, 5, 255]);
        assert_eq!(
            &image.rgba[..16],
            &[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]
        );
        let FrameDraw::Image(draw) = &mut plan.draws[0] else {
            unreachable!()
        };
        draw.blend_mode = 0x47;
        assert!(bridge
            .compose(&plan)
            .unwrap_err()
            .to_string()
            .contains("unsupported BGI display blend"));
    }
    #[derive(Default)]
    struct Host {
        surface: Vec<u8>,
        batch: Vec<u8>,
        client: Option<ClientId>,
    }
    impl FileSystem for Host {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            Err(crysirror_core::Error::Unsupported("unused test read"))
        }
        fn write_atomic(
            &mut self,
            _: ClientId,
            _: &VfsPath,
            _: &[u8],
        ) -> crysirror_core::Result<()> {
            Err(crysirror_core::Error::Unsupported("unused test write"))
        }
    }
    impl HostServices for Host {
        fn now_ms(&self) -> u64 {
            0
        }
        fn submit(&mut self, _: ClientId, _: &[Command]) -> crysirror_core::Result<()> {
            Ok(())
        }
        fn present_surface(
            &mut self,
            client: ClientId,
            slot: u32,
            width: u32,
            height: u32,
            rgba: &[u8],
        ) -> crysirror_core::Result<VfsPath> {
            assert_eq!((slot, width, height), (0, 4, 3));
            self.client = Some(client);
            self.surface = rgba.to_vec();
            VfsPath::new(crysirror_core::path::Mount::Asset, "__surface/0")
        }
        fn submit_engine(
            &mut self,
            client: ClientId,
            schema: u32,
            data: &[u8],
        ) -> crysirror_core::Result<()> {
            assert_eq!(self.client, Some(client));
            assert_eq!(schema, 1);
            self.batch = data.to_vec();
            Ok(())
        }
    }
    #[test]
    fn publication_is_client_scoped_and_xrgb_payload_is_not_display_alpha() {
        let mut bridge = FrameBridge {
            text: TextSystem::default(),
        };
        let mut plan = plan();
        let FrameDraw::Image(draw) = &mut plan.draws[0] else {
            unreachable!()
        };
        draw.blend_mode = 0xf0;
        let mut host = Host::default();
        let stats = bridge.publish(ClientId(7), &plan, &mut host).unwrap();
        assert_eq!((stats.image_draws, stats.glyph_draws), (1, 0));
        assert_eq!(host.client, Some(ClientId(7)));
        assert!(host.surface.chunks_exact(4).all(|p| p[3] == 255));
        let batch: serde_json::Value = serde_json::from_slice(&host.batch).unwrap();
        assert_eq!(batch["layers"][0]["layer"]["paint"]["asset"], "__surface/0");
    }
}
