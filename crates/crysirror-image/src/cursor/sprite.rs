//! Sprite-sheet cursor preparation. Coordinates and animation policy are data;
//! this kernel has no native VM, window or renderer dependency.
use super::{CursorFrame, CursorImage, MAX_CURSOR_EDGE, MAX_CURSOR_FRAMES};
use crate::ImageSurface;
use anyhow::{Result, ensure};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpriteHotspot {
    Position([u32; 2]),
    /// Search the separate marker rectangle, in column-major order. Marker
    /// coordinates are relative to that rectangle; absence uses `fallback`.
    Marker {
        rect: [u32; 4],
        rgb: [u8; 3],
        fallback: [u32; 2],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteCursorLayout {
    /// Source rectangles `[x, y, width, height]`, in display order.
    pub frames: Vec<[u32; 4]>,
    pub hotspot: SpriteHotspot,
    /// A source pixel whose RGB identifies transparent pixels in every frame.
    pub color_key_at: Option<[u32; 2]>,
    pub frame_ms: u32,
}

/// Read only declared geometry from a bounded image header before admitting a
/// worker decode. Supported header profiles are PDT, RealLive G00, PNG and
/// BITMAPINFOHEADER BMP; this neither loads pixels nor validates the payload.
/// G00 type 2 requires its complete region table in the supplied header.
pub fn sprite_source_size(name: &str, header: &[u8]) -> Result<[u32; 2]> {
    ensure!(header.len() <= 65536, "sprite image header budget");
    let word = |at: usize| -> Result<u32> {
        let bytes: [u8; 4] = header
            .get(at..at + 4)
            .ok_or_else(|| anyhow::anyhow!("truncated sprite image header"))?
            .try_into()
            .unwrap();
        Ok(u32::from_le_bytes(bytes))
    };
    let size = if name.to_ascii_lowercase().ends_with(".g00") {
        let dimensions = header
            .get(1..5)
            .ok_or_else(|| anyhow::anyhow!("truncated sprite G00 geometry"))?;
        crate::rgba_len(
            u16::from_le_bytes([dimensions[0], dimensions[1]]) as u32,
            u16::from_le_bytes([dimensions[2], dimensions[3]]) as u32,
        )?;
        let (w, h, _) = crate::g00::g00_regions(header)?;
        [u32::try_from(w)?, u32::try_from(h)?]
    } else if header.starts_with(b"PDT10") || header.starts_with(b"PDT11") {
        [word(12)?, word(16)?]
    } else if header.starts_with(b"BM") {
        ensure!(word(14)? >= 40, "unsupported sprite BMP header");
        let (w, h) = (word(18)? as i32, word(22)? as i32);
        ensure!(
            w > 0 && h != 0 && h != i32::MIN,
            "invalid sprite BMP geometry"
        );
        [w as u32, h.unsigned_abs()]
    } else if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        ensure!(
            header.get(12..16) == Some(b"IHDR"),
            "missing sprite PNG IHDR"
        );
        [word(16)?.swap_bytes(), word(20)?.swap_bytes()]
    } else {
        anyhow::bail!("unsupported sprite image header profile");
    };
    crate::rgba_len(size[0], size[1])?;
    Ok(size)
}

pub fn cursor_from_sprite(
    image: &ImageSurface,
    layout: &SpriteCursorLayout,
) -> Result<CursorImage> {
    image.validate()?;
    ensure!(
        (1..=MAX_CURSOR_FRAMES).contains(&layout.frames.len()),
        "sprite cursor frame count"
    );
    let inside = |[x, y, w, h]: [u32; 4]| -> Result<()> {
        ensure!(
            w > 0
                && h > 0
                && x.checked_add(w).is_some_and(|v| v <= image.width)
                && y.checked_add(h).is_some_and(|v| v <= image.height),
            "sprite cursor rectangle outside image"
        );
        Ok(())
    };
    let pixel = |x: u32, y: u32| {
        let i = ((y * image.width + x) * 4) as usize;
        [image.rgba[i], image.rgba[i + 1], image.rgba[i + 2]]
    };
    let key = layout
        .color_key_at
        .map(|[x, y]| {
            inside([x, y, 1, 1])?;
            Ok::<_, anyhow::Error>(pixel(x, y))
        })
        .transpose()?;
    let hotspot = match layout.hotspot {
        SpriteHotspot::Position(p) => p,
        SpriteHotspot::Marker {
            rect: [x, y, w, h],
            rgb,
            fallback,
        } => {
            inside([x, y, w, h])?;
            (0..w)
                .find_map(|dx| {
                    (0..h)
                        .find(|&dy| pixel(x + dx, y + dy) == rgb)
                        .map(|dy| [dx, dy])
                })
                .unwrap_or(fallback)
        }
    };
    let mut frames = Vec::with_capacity(layout.frames.len());
    for &[x, y, w, h] in &layout.frames {
        inside([x, y, w, h])?;
        ensure!(
            w <= MAX_CURSOR_EDGE && h <= MAX_CURSOR_EDGE,
            "sprite cursor frame size"
        );
        ensure!(
            hotspot[0] < w && hotspot[1] < h,
            "sprite cursor hotspot outside frame"
        );
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for dy in 0..h {
            let start = (((y + dy) * image.width + x) * 4) as usize;
            for p in image.rgba[start..start + (w * 4) as usize].chunks_exact(4) {
                rgba.extend_from_slice(&[
                    p[0],
                    p[1],
                    p[2],
                    if key == Some([p[0], p[1], p[2]]) {
                        0
                    } else {
                        p[3]
                    },
                ]);
            }
        }
        frames.push(CursorFrame {
            image: ImageSurface {
                width: w,
                height: h,
                rgba,
            },
            hotspot,
        });
    }
    ensure!(
        frames.len() == 1 || layout.frame_ms > 0,
        "sprite cursor animation duration"
    );
    let steps = (0..frames.len())
        .map(|i| {
            (
                i,
                if frames.len() == 1 {
                    0
                } else {
                    layout.frame_ms
                },
            )
        })
        .collect();
    Ok(CursorImage { frames, steps })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_sprite_cursor_extracts_frames_key_and_marker() {
        let mut image = ImageSurface {
            width: 7,
            height: 6,
            rgba: vec![0; 7 * 6 * 4],
        };
        for p in image.rgba.chunks_exact_mut(4) {
            p.copy_from_slice(&[255, 0, 255, 255]);
        }
        let mut set = |x: usize, y: usize, p: [u8; 4]| {
            image.rgba[(y * 7 + x) * 4..(y * 7 + x + 1) * 4].copy_from_slice(&p)
        };
        set(1, 1, [10, 20, 30, 127]);
        set(4, 1, [40, 50, 60, 255]);
        set(2, 5, [255, 255, 255, 255]);
        let cursor = cursor_from_sprite(
            &image,
            &SpriteCursorLayout {
                frames: vec![[1, 1, 2, 2], [4, 1, 2, 2]],
                hotspot: SpriteHotspot::Marker {
                    rect: [1, 4, 2, 2],
                    rgb: [255; 3],
                    fallback: [0, 0],
                },
                color_key_at: Some([0, 0]),
                frame_ms: 53,
            },
        )
        .unwrap();
        assert_eq!(cursor.steps, [(0, 53), (1, 53)]);
        assert_eq!(cursor.frames[0].hotspot, [1, 1]);
        assert_eq!(
            &cursor.frames[0].image.rgba[..8],
            &[10, 20, 30, 127, 255, 0, 255, 0]
        );
        assert_eq!(&cursor.frames[1].image.rgba[..4], &[40, 50, 60, 255]);
    }
    #[test]
    fn sprite_cursor_rejects_outside_rectangles_and_hotspots() {
        let image = ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
        for (frames, hotspot) in [
            (vec![[u32::MAX, 0, 1, 1]], [0, 0]),
            (vec![[0, 0, 1, 1]], [1, 0]),
        ] {
            assert!(
                cursor_from_sprite(
                    &image,
                    &SpriteCursorLayout {
                        frames,
                        hotspot: SpriteHotspot::Position(hotspot),
                        color_key_at: None,
                        frame_ms: 0
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn source_geometry_is_header_only_and_unknown_layout_fails() {
        let mut pdt = vec![0; 32];
        pdt[..5].copy_from_slice(b"PDT10");
        pdt[12..16].copy_from_slice(&400u32.to_le_bytes());
        pdt[16..20].copy_from_slice(&88u32.to_le_bytes());
        assert_eq!(sprite_source_size("pointer.pdt", &pdt).unwrap(), [400, 88]);
        assert_eq!(
            sprite_source_size("pointer.g00", &[0, 144, 1, 88, 0, 0, 0, 0, 0]).unwrap(),
            [400, 88]
        );
        assert!(sprite_source_size("pointer.jpg", &[0xff, 0xd8]).is_err());
        assert!(sprite_source_size("pointer.pdt", &pdt[..16]).is_err());
        let mut empty_g00 = vec![0; 33];
        empty_g00[0] = 2;
        empty_g00[5] = 1;
        assert!(sprite_source_size("pointer.g00", &empty_g00).is_err());
    }
}
