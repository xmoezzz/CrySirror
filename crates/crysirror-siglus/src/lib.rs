// SPDX-License-Identifier: MPL-2.0
//! Siglus-specific resource frontend, ported from the pinned original source.
//! This crate does not register a VM until original execution/save semantics
//! are adapted. G00 support is not a claim that a Siglus game can run yet.
#![forbid(unsafe_code)]
use anyhow::{ensure, Result};
use crysirror_image::{ImageDecoder, ImageSurface};
pub mod g00;
mod jpeg;

/// Native metadata remains here; the shared surface does not interpret pivots.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub center_x: i32,
    pub center_y: i32,
    pub rgba: Vec<u8>,
}

pub struct SiglusImageDecoder;
impl ImageDecoder for SiglusImageDecoder {
    fn decode(&self, bytes: &[u8]) -> Result<ImageSurface> {
        // RealLive type-2 files carry several patterns in one bitmap: the
        // engine's shared atlas decoder returns that bitmap whole, and the
        // RealLive frontend selects patterns with `g00_regions`.
        if bytes.first() == Some(&2) {
            if let Ok((_, _, regions)) = crysirror_image::g00::g00_regions(bytes) {
                if regions.len() > 1 {
                    let atlas = crysirror_image::g00::decode_g00(bytes)?;
                    return Ok(ImageSurface {
                        width: u32::try_from(atlas.width)?,
                        height: u32::try_from(atlas.height)?,
                        rgba: atlas.rgba,
                    });
                }
            }
        }
        let mut decoded = g00::decode_g00(bytes)?;
        ensure!(
            decoded.frames.len() == 1,
            "G00 has multiple native cuts: select the frame in the Siglus frontend explicitly"
        );
        let frame = decoded.frames.pop().unwrap();
        ensure!(
            frame.center_x == 0
                && frame.center_y == 0
                && decoded.original_sizes[0] == (frame.width, frame.height),
            "G00 native pivot/cut metadata requires the Siglus object frontend"
        );
        Ok(ImageSurface {
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn type3_preserves_jpeg_pixels_through_shared_decoder() {
        let rgb = [220, 30, 20, 10, 180, 40];
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
            .encode(&rgb, 2, 1, image::ColorType::Rgb8)
            .unwrap();
        let expected = crysirror_image::decode_standard(&jpeg).unwrap();
        crate::jpeg::xor_type3_jpeg_in_place(&mut jpeg);
        let mut file = vec![3, 2, 0, 1, 0];
        file.extend(jpeg);
        assert_eq!(SiglusImageDecoder.decode(&file).unwrap(), expected);
    }
}
