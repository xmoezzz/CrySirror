// SPDX-License-Identifier: MPL-2.0
//! Checked channel layout conversion for engine image resources.
//! Stored alpha association is preserved; this API does not premultiply RGB.
use anyhow::{ensure, Result};
#[derive(Debug, Clone, Copy)]
pub enum PixelLayout {
    Rgba,
    Bgra,
    Rgb,
    Bgr,
    Gray,
    GrayAlpha,
    Coverage,
}
impl PixelLayout {
    pub fn channels(self) -> usize {
        match self {
            Self::Rgba | Self::Bgra => 4,
            Self::Rgb | Self::Bgr => 3,
            Self::GrayAlpha => 2,
            _ => 1,
        }
    }
}
pub fn to_rgba8(size: [u32; 2], pixels: &[u8], layout: PixelLayout) -> Result<Vec<u8>> {
    let rgba_len = crate::rgba_len(size[0], size[1])?;
    let count = rgba_len / 4;
    let channels = layout.channels();
    ensure!(
        pixels.len() == count * channels,
        "pixel input length mismatch"
    );
    let mut out = Vec::with_capacity(rgba_len);
    for p in pixels.chunks_exact(channels) {
        out.extend_from_slice(&match layout {
            PixelLayout::Rgba => [p[0], p[1], p[2], p[3]],
            PixelLayout::Bgra => [p[2], p[1], p[0], p[3]],
            PixelLayout::Rgb => [p[0], p[1], p[2], 255],
            PixelLayout::Bgr => [p[2], p[1], p[0], 255],
            PixelLayout::Gray => [p[0], p[0], p[0], 255],
            PixelLayout::GrayAlpha => [p[0], p[0], p[0], p[1]],
            PixelLayout::Coverage => [255, 255, 255, p[0]],
        });
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layouts_keep_hidden_rgb_and_coverage_distinct_from_luminance() {
        assert_eq!(
            to_rgba8(
                [2, 1],
                &[17, 34, 51, 0, 68, 85, 102, 128],
                PixelLayout::Bgra
            )
            .unwrap(),
            [51, 34, 17, 0, 102, 85, 68, 128]
        );
        assert_eq!(
            to_rgba8([2, 1], &[0, 128], PixelLayout::Coverage).unwrap(),
            [255, 255, 255, 0, 255, 255, 255, 128]
        );
        assert_eq!(
            to_rgba8([2, 1], &[0, 128], PixelLayout::Gray).unwrap(),
            [0, 0, 0, 255, 128, 128, 128, 255]
        );
        assert!(to_rgba8([1, 1], &[0; 3], PixelLayout::Rgba).is_err());
        assert!(to_rgba8([u32::MAX, 1], &[], PixelLayout::Gray).is_err());
    }
}
