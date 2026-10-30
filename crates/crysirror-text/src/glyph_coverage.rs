// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded glyph-mask effects, independent of layout, renderer and native VM.
use anyhow::{Result, ensure};

pub struct Coverage {
    pub pixels: Vec<u8>,
    pub size: [usize; 2],
    pub origin: [i32; 2],
}
fn validate(src: &[u8], width: usize, height: usize, radius: u8) -> Result<()> {
    ensure!(radius <= 16, "glyph coverage radius exceeds 16");
    ensure!(
        width <= 8192
            && height <= 8192
            && width.checked_mul(height) == Some(src.len())
            && src.len() <= 4 * 1024 * 1024,
        "glyph coverage dimensions/budget"
    );
    ensure!(
        (width + 2 * usize::from(radius))
            .checked_mul(height + 2 * usize::from(radius))
            .is_some_and(|n| n <= 8 * 1024 * 1024),
        "glyph effect output budget"
    );
    Ok(())
}
/// Circular maximum coverage, sharing the rich-text outline raster kernel.
pub fn disk_outline(src: &[u8], width: usize, height: usize, radius: u8) -> Result<Coverage> {
    validate(src, width, height, radius)?;
    if width == 0 || height == 0 {
        return Ok(Coverage {
            pixels: vec![],
            size: [0, 0],
            origin: [0, 0],
        });
    }
    let result = crate::raster::dilate(
        &crate::raster::GlyphMask {
            x: 0,
            y: 0,
            width,
            height,
            alpha: src.to_vec(),
        },
        usize::from(radius),
    );
    Ok(Coverage {
        pixels: result.alpha,
        size: [result.width, result.height],
        origin: [result.x, result.y],
    })
}
/// Union repeated source-over copies at every (x,y) in 0..=extent. The
/// truncating union a+b-floor(a*b/255) preserves native coverage arithmetic.
/// Policy reference: siglus_rs 5f358ef text_render.rs draw_glyph_bitmap_face;
/// its window/font defaults and pass placement remain in the native adapter.
pub fn square_face(src: &[u8], width: usize, height: usize, extent: u8) -> Result<Coverage> {
    validate(src, width, height, extent)?;
    if width == 0 || height == 0 {
        return Ok(Coverage {
            pixels: vec![],
            size: [0, 0],
            origin: [0, 0],
        });
    }
    let n = usize::from(extent) + 1;
    ensure!(
        src.len()
            .checked_mul(n * n)
            .is_some_and(|work| work <= 64 * 1024 * 1024),
        "glyph face work budget"
    );
    let (w, h) = (width + usize::from(extent), height + usize::from(extent));
    let mut pixels = vec![0u8; w * h];
    for oy in 0..n {
        for ox in 0..n {
            for y in 0..height {
                for x in 0..width {
                    let source = u32::from(src[y * width + x]);
                    let dest = &mut pixels[(y + oy) * w + x + ox];
                    *dest = (source + u32::from(*dest) - source * u32::from(*dest) / 255) as u8;
                }
            }
        }
    }
    Ok(Coverage {
        pixels,
        size: [w, h],
        origin: [0, 0],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outline_and_face_have_distinct_shape_origin_and_fractional_alpha_contracts() {
        let outline = disk_outline(&[128], 1, 1, 1).unwrap();
        assert_eq!(outline.origin, [-1, -1]);
        assert_eq!(outline.pixels, [0, 128, 0, 128, 128, 128, 0, 128, 0]);
        let face = square_face(&[128, 128], 2, 1, 1).unwrap();
        assert_eq!(face.origin, [0, 0]);
        assert_eq!(face.size, [3, 2]);
        assert_eq!(face.pixels, [128, 192, 128, 128, 192, 128]);
        assert!(square_face(&[0; 4], 1, 1, 1).is_err());
        assert!(disk_outline(&[1], 1, 1, 17).is_err());
        assert!(square_face(&[1], 1, 1, 17).is_err());
        assert!(square_face(&[], 0, 0, 2).unwrap().pixels.is_empty());
    }
}
