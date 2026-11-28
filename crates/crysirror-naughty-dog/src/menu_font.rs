// SPDX-License-Identifier: MPL-2.0
//! Retail menu fonts. The metrics are loose BMFont descriptors while their
//! packed coverage textures live in misc1/debugrenderingf.pak.
use crate::{adapter::MAIN, package::Directory, texture::VramDesc};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{ClientStorage, HostServices},
    state::ClientId,
};
use crysirror_image::bmfont::PackedFont;
use crysirror_resource::read_all;
use serde_json::{Value, json};

struct Face {
    metrics: PackedFont,
    atlas: String,
}

/// The two root-page styles are calibrated to the visible BCAS20097 title in
/// a 1280 × 720 viewport. Submenus retain their independent layout.
#[derive(Clone, Copy)]
pub(super) enum LabelStyle {
    Menu(u16),
    PressStart,
    Legal,
}

impl LabelStyle {
    pub fn height(self) -> u16 {
        match self {
            Self::Menu(size) => size,
            Self::PressStart => 34,
            Self::Legal => 22,
        }
    }

    fn width(self) -> f32 {
        match self {
            Self::PressStart => 36.,
            _ => f32::from(self.height()),
        }
    }

    fn tracking(self) -> f32 {
        match self {
            Self::Menu(_) => 0.,
            Self::PressStart | Self::Legal => 1.,
        }
    }

    fn shadow(self) -> Option<(f32, u8)> {
        match self {
            Self::Menu(_) => None,
            Self::PressStart => Some((3.5, 105)),
            Self::Legal => Some((2.5, 48)),
        }
    }
}

#[derive(Default)]
pub(super) struct MenuFont(Vec<Face>);

impl MenuFont {
    pub fn load(
        host: &mut dyn HostServices,
        client: ClientId,
        storage: &dyn ClientStorage,
        language: u32,
    ) -> Result<Self> {
        let root = storage.open_asset(&VfsPath::new(
            Mount::Asset,
            &format!("{MAIN}/misc1/debugrenderingf.pak"),
        )?)?;
        let dir = Directory::read(root.as_ref())?;
        let textures = dir
            .items
            .iter()
            .filter(|i| i.kind == "VRAM_DESC")
            .map(|i| VramDesc::read(root.as_ref(), i.payload_offset))
            .collect::<Result<Vec<_>>>()?;
        let localized = match language {
            11 => Some("japanese"),
            12 => Some("korean"),
            16 => Some("chinese"),
            17 => Some("russian"),
            _ => None,
        };
        // The localized atlas contains only a subset of the Latin glyphs.
        // Prefer it when the chosen language supplies the requested scalar.
        let mut faces = Vec::new();
        for (i, name) in localized.into_iter().chain(["main"]).enumerate() {
            let descriptor = storage.open_asset(&VfsPath::new(
                Mount::Asset,
                &format!("PS3_GAME/USRDIR/data/fonts/{name}.fnt"),
            )?)?;
            let bytes = read_all(descriptor.as_ref(), 1 << 20)?;
            let metrics = PackedFont::parse(std::str::from_utf8(&bytes)?)?;
            let page = format!("/font/{}/", metrics.page);
            let texture = textures
                .iter()
                .find(|t| t.name.contains(&page))
                .with_context(|| format!("missing {name} font atlas"))?;
            let [w, h] = metrics.dimensions;
            ensure!(
                texture.width == w && texture.height == h,
                "font page dimensions differ"
            );
            let rgba = texture.decode(root.as_ref(), dir.texture_base)?;
            // This RSX atlas is stored bottom up; mask bits 1/2/4/8 select
            // A/R/G/B of VramDesc's RGBA output. The two-pixel blank border
            // in all 204 main.fnt glyph rectangles confirms the mapping.
            let planes = metrics.coverage_atlas(&rgba.rgba, [3, 0, 1, 2], true)?;
            let atlas = host
                .upload_rgba(client, 0x4e44_4600 + i as u32, w, h * 4, &planes)?
                .relative()
                .to_owned();
            faces.push(Face { metrics, atlas });
        }
        Ok(Self(faces))
    }

    pub fn layer(
        &self,
        id: u32,
        rect: [i32; 4],
        text: &str,
        color: [u8; 4],
        style: LabelStyle,
        center: bool,
    ) -> Option<Value> {
        let primary = self.0.first()?;
        let size = f32::from(style.height());
        let baseline = size * primary.metrics.base as f32 / primary.metrics.size as f32;
        let line_height = size * primary.metrics.line_height as f32 / primary.metrics.size as f32;
        let mut glyphs = Vec::new();
        let mut shadows = Vec::new();
        for (line, text) in text.lines().enumerate() {
            let mut x = 0.;
            let y = line as f32 * line_height;
            let mut previous = None;
            let mut placed = Vec::new();
            for ch in text.chars() {
                let (idx, glyph) = self
                    .0
                    .iter()
                    .enumerate()
                    .find_map(|(i, f)| f.metrics.glyph(ch).map(|g| (i, g)))?;
                let face = &self.0[idx];
                let scale = size / face.metrics.size as f32;
                let x_scale = style.width() / face.metrics.size as f32;
                if previous.is_some() {
                    x += style.tracking();
                }
                if let Some((i, left)) = previous
                    && i == idx
                {
                    x += x_scale * face.metrics.kerning(left, ch) as f32;
                }
                let [sx, sy, sw, sh] = glyph.texel.map(|v| v as f32);
                let [w, h] = face.metrics.dimensions.map(|v| v as f32);
                let v = sy + glyph.plane as f32 * h;
                if !ch.is_whitespace() && sw > 0. && sh > 0. {
                    placed.push((
                        ch,
                        face,
                        [
                            x + glyph.offset[0] as f32 * x_scale,
                            y + baseline + (glyph.offset[1] - face.metrics.base) as f32 * scale,
                            sw * x_scale,
                            sh * scale,
                        ],
                        [sx / w, v / (4. * h), (sx + sw) / w, (v + sh) / (4. * h)],
                    ));
                }
                x += glyph.advance as f32 * x_scale;
                previous = Some((idx, ch));
            }
            let left = if center {
                (rect[2] as f32 - x) * 0.5
            } else {
                0.
            };
            for (ch, face, mut dst, uv) in placed {
                dst[0] += left;
                let positioned = |rect: [f32; 4], tint: [u8; 4]| json!({"character":ch,"baseline":[rect[0],y+baseline],"image":{"asset":face.atlas,"rect":rect,"uv":uv,"tint":tint}});
                if let Some((offset, alpha)) = style.shadow() {
                    let mut shadow_rect = dst;
                    shadow_rect[1] += offset;
                    shadows.push(positioned(shadow_rect, [0, 0, 0, alpha]));
                }
                glyphs.push(positioned(dst, color));
            }
        }
        if !shadows.is_empty() {
            shadows.append(&mut glyphs);
            glyphs = shadows;
        }
        Some(
            json!({"op":"put","layer":{"id":id,"rect":rect,"paint":{"kind":"glyph_run","font":"default","size":size,"glyphs":glyphs}}}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kerned_glyphs_use_the_native_plane_bearing_and_center() -> Result<()> {
        let data = "info size=20\ncommon lineHeight=20 base=15 scaleW=4 scaleH=4 pages=1 packed=1\npage id=0 file=\"test.tga\"\nchars count=2\nchar id=65 x=0 y=0 width=2 height=3 xoffset=-1 yoffset=1 xadvance=10 page=0 chnl=4\nchar id=86 x=2 y=1 width=2 height=3 xoffset=1 yoffset=2 xadvance=10 page=0 chnl=1\nkernings count=1\nkerning first=65 second=86 amount=-2";
        let font = MenuFont(vec![Face {
            metrics: PackedFont::parse(data)?,
            atlas: "character_atlas".into(),
        }]);
        let command = font
            .layer(
                1,
                [10, 20, 60, 40],
                "AV",
                [4, 5, 6, 255],
                LabelStyle::Menu(20),
                true,
            )
            .unwrap();
        let paint: crysirror_scene::layer::Paint =
            serde_json::from_value(command["layer"]["paint"].clone())?;
        let crysirror_scene::layer::Paint::GlyphRun { glyphs, .. } = paint else {
            panic!("expected retained glyphs")
        };
        let first = glyphs[0].image.as_ref().unwrap();
        let second = glyphs[1].image.as_ref().unwrap();
        assert_eq!(first.rect, [20., 1., 2., 3.]);
        assert_eq!(first.uv, [0., 0.5, 0.5, 11. / 16.]);
        assert_eq!(first.tint, [4, 5, 6, 255]);
        assert_eq!(second.rect, [30., 2., 2., 3.]);
        assert_eq!(second.uv, [0.5, 1. / 16., 1., 4. / 16.]);
        assert!(
            font.layer(1, [0, 0, 1, 1], "X", [255; 4], LabelStyle::Menu(20), false)
                .is_none()
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires BCAS20097 01.00 via UNCHARTED2_ROOT"]
    fn installed_title_prompt_uses_disc_metrics_and_ink() -> Result<()> {
        use crysirror_core::service::MemoryAssetReader;
        let root = std::path::PathBuf::from(
            std::env::var_os("UNCHARTED2_ROOT").context("UNCHARTED2_ROOT")?,
        );
        let data = std::fs::read_to_string(root.join("PS3_GAME/USRDIR/data/fonts/main.fnt"))?;
        let font = PackedFont::parse(&data)?;
        let reader = MemoryAssetReader(
            std::fs::read(root.join(MAIN).join("misc1/debugrenderingf.pak"))?.into(),
        );
        let dir = Directory::read(&reader)?;
        let page = format!("/font/{}/", font.page);
        let desc = dir
            .items
            .iter()
            .filter(|i| i.kind == "VRAM_DESC")
            .map(|i| VramDesc::read(&reader, i.payload_offset))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .find(|d| d.name.contains(&page))
            .context("retail atlas")?;
        let surface = desc.decode(&reader, dir.texture_base)?;
        let image = font.coverage_atlas(&surface.rgba, [3, 0, 1, 2], true)?;
        let [w, h] = font.dimensions;
        assert_eq!((font.size, font.base, w, h), (42, 34, 256, 128));
        let mut ink = 0usize;
        let mut count = 0;
        for ch in (0..=0xffff).filter_map(char::from_u32) {
            let Some(g) = font.glyph(ch) else { continue };
            count += 1;
            let [x, y, gw, gh] = g.texel;
            for r in 0..gh {
                for c in 0..gw {
                    let idx = (g.plane as u32 * h + y + r) * w + x + c;
                    let alpha = image[idx as usize * 4 + 3];
                    if c == 0 || r == 0 || c + 1 == gw || r + 1 == gh {
                        assert_eq!(alpha, 0, "nonempty border for {ch:?}");
                    }
                    ink += usize::from(alpha > 0);
                }
            }
        }
        assert_eq!(count, 204);
        assert_eq!(ink, 41968);
        for (text, expected) in [
            ("Press the START button", 324),
            ("New Game", 160),
            ("AV", 42),
        ] {
            let chars = text.chars().collect::<Vec<_>>();
            let advance = chars
                .iter()
                .map(|c| font.glyph(*c).unwrap().advance)
                .sum::<i32>()
                + chars
                    .windows(2)
                    .map(|a| font.kerning(a[0], a[1]))
                    .sum::<i32>();
            assert_eq!(advance, expected);
        }
        let menu = MenuFont(vec![Face {
            metrics: font,
            atlas: "disc_font".into(),
        }]);
        let command = menu
            .layer(
                1,
                [0, 470, 1280, 50],
                "Press the START button",
                [200, 200, 195, 255],
                LabelStyle::PressStart,
                true,
            )
            .unwrap();
        let cmd: crysirror_scene::layer::Paint =
            serde_json::from_value(command["layer"]["paint"].clone())?;
        let crysirror_scene::layer::Paint::GlyphRun { glyphs, .. } = cmd else {
            panic!("expected retained glyphs")
        };
        let (shadows, ink) = glyphs.split_at(glyphs.len() / 2);
        assert_eq!(ink.len(), 19);
        assert_eq!(ink[0].character, 'P');
        assert_eq!(ink.last().unwrap().character, 'n');
        let first = ink[0].image.as_ref().unwrap();
        let last = ink.last().unwrap().image.as_ref().unwrap();
        assert!((first.rect[0] - 490.).abs() < 3.);
        assert!((last.rect[0] + last.rect[2] - 790.).abs() < 3.);
        for (sh, face) in shadows.iter().zip(ink) {
            let sh = sh.image.as_ref().unwrap();
            let face = face.image.as_ref().unwrap();
            assert_eq!(sh.uv, face.uv);
            assert_eq!(sh.rect[1], face.rect[1] + 3.5);
            assert_eq!(sh.tint, [0, 0, 0, 105]);
            assert_eq!(face.tint, [200, 200, 195, 255]);
        }
        Ok(())
    }
}
