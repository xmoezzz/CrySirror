// SPDX-License-Identifier: MPL-2.0
//! Stored RGB resampling with 8.8 coordinates and one truncating shift.
use crate::{ImageSurface, effects::MAX_EFFECT_PIXELS, rgba_len};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Sample a rectangle using the integer parts of each output coordinate and
/// its successor. The adjacent tap may cross the window edge, within the
/// source image. Both axes use weights summing to 255; output alpha is opaque.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RectResampleByte255 {
    /// Source x, y, width and height in pixels.
    pub source: [u32; 4],
    /// Output width and height in pixels.
    pub size: [u32; 2],
}

impl RectResampleByte255 {
    pub fn validate(self) -> Result<()> {
        rgba_len(self.size[0], self.size[1])?;
        ensure!(
            u64::from(self.size[0]) * u64::from(self.size[1]) <= MAX_EFFECT_PIXELS,
            "byte255 output pixel budget"
        );
        ensure!(
            self.source[2] != 0 && self.source[3] != 0,
            "byte255 empty window"
        );
        Ok(())
    }

    pub fn output_size(self, input: [u32; 2]) -> Result<[u32; 2]> {
        self.validate()?;
        rgba_len(input[0], input[1])?;
        for axis in 0..2 {
            ensure!(
                self.source[axis]
                    .checked_add(self.source[axis + 2])
                    .is_some_and(|end| end <= input[axis]),
                "byte255 window outside immutable input"
            );
        }
        Ok(self.size)
    }

    pub fn apply(
        self,
        input: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        input.validate()?;
        let source_size = [input.width, input.height];
        let [width, height] = self.output_size(source_size)?;
        check()?;
        let mut rgba = vec![255; rgba_len(width, height)?];
        let axis = |axis: usize, index: u32| {
            let origin = self.source[axis];
            let extent = u64::from(self.source[axis + 2]);
            let output = u64::from(self.size[axis]);
            let position = (u64::from(index) * extent * 256 / output) as u32;
            let first = origin + (position >> 8);
            let next = origin + (u64::from(index + 1) * extent / output) as u32;
            let second = if next == source_size[axis] {
                first
            } else {
                next
            };
            (first, second, position & 255)
        };
        let pixel = |x: u32, y: u32| {
            let start = ((y * input.width + x) * 4) as usize;
            &input.rgba[start..start + 3]
        };
        for (y, row) in rgba.chunks_exact_mut(width as usize * 4).enumerate() {
            check()?;
            let (top, bottom, wy) = axis(1, y as u32);
            for (x, out) in row.chunks_exact_mut(4).enumerate() {
                let (left, right, wx) = axis(0, x as u32);
                let [a, b, c, d] = [
                    pixel(left, top),
                    pixel(right, top),
                    pixel(left, bottom),
                    pixel(right, bottom),
                ];
                for channel in 0..3 {
                    let mix_x =
                        |left: u8, right: u8| (255 - wx) * u32::from(left) + wx * u32::from(right);
                    out[channel] = (((255 - wy) * mix_x(a[channel], b[channel])
                        + wy * mix_x(c[channel], d[channel]))
                        >> 16) as u8;
                }
            }
        }
        check()?;
        Ok(ImageSurface {
            width,
            height,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successor_taps_include_adjacent_pixels_without_rounding_the_intermediate() -> Result<()> {
        let input = ImageSurface {
            width: 4,
            height: 3,
            rgba: [0, 40, 80, 120, 20, 60, 100, 140, 40, 80, 120, 160]
                .into_iter()
                .flat_map(|value| [value, 255, 0, 13])
                .collect(),
        };
        let spec = RectResampleByte255 {
            source: [1, 0, 2, 2],
            size: [4, 4],
        };
        let output = spec.apply(&input, &|| Ok(()))?;
        assert_eq!(
            output
                .rgba
                .chunks_exact(4)
                .map(|pixel| pixel[0])
                .collect::<Vec<_>>(),
            [
                39, 59, 79, 99, 49, 69, 89, 109, 59, 79, 99, 119, 69, 89, 109, 129
            ]
        );
        assert!(
            output
                .rgba
                .chunks_exact(4)
                .all(|pixel| pixel[1..] == [253, 0, 255])
        );

        let edge = RectResampleByte255 {
            source: [2, 1, 2, 2],
            ..spec
        };
        let output = edge.apply(&input, &|| Ok(()))?;
        assert_eq!(&output.rgba[60..64], &[158, 253, 0, 255]);
        let invalid = RectResampleByte255 {
            source: [u32::MAX, 0, 2, 2],
            ..spec
        };
        assert!(invalid.apply(&input, &|| Ok(())).is_err());
        assert!(spec.apply(&input, &|| anyhow::bail!("cancelled")).is_err());
        Ok(())
    }
}
