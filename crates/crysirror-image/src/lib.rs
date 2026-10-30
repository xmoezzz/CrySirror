// SPDX-License-Identifier: MPL-2.0
//! CPU-only image surfaces and decoder dispatch. No GPU, window or VM policy.
#![forbid(unsafe_code)]
use anyhow::{ensure, Result};
use std::{collections::BTreeMap, io::Cursor, sync::Arc};
mod formats;
pub use formats::{ImageDecodeContext, ImageFormat, ImageSource};

pub const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;
pub mod nvsg;
pub mod pixels;
pub mod padding;
pub mod wipe_mask;
mod png;
pub use png::{encode_png_uncompressed, png_uncompressed_len};
pub fn rgba_len(width: u32, height: u32) -> Result<usize> {
    ensure!(
        width > 0 && height > 0 && width <= 8192 && height <= 8192,
        "image dimensions must be 1..8192"
    );
    let n = u64::from(width) * u64::from(height) * 4;
    ensure!(
        n <= MAX_DECODED_BYTES as u64,
        "decoded image exceeds 64 MiB"
    );
    Ok(n as usize)
}
pub fn swap_red_blue(bytes: &mut [u8]) -> Result<()> {
    ensure!(
        bytes.len().is_multiple_of(4),
        "invalid four-channel image length"
    );
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSurface {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl ImageSurface {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            rgba_len(self.width, self.height)? == self.rgba.len(),
            "RGBA length mismatch"
        );
        Ok(())
    }
}
pub trait ImageDecoder: Send + Sync {
    /// Decode one ordinary top-left image. Native frame/pivot policies must be
    /// resolved explicitly by their frontend before using this generic surface.
    fn decode(&self, bytes: &[u8]) -> Result<ImageSurface>;
}
pub mod associated;
pub mod bitmap;
pub mod composition;
pub mod cursor;
pub mod color;
pub mod crx;
pub mod cximage;
mod dds;
pub mod effects;
pub mod sprite_alpha;
pub mod packed;
pub mod row_copy;
pub mod transition;
pub mod vector;
pub mod warp;
pub fn decode_standard(bytes: &[u8]) -> Result<ImageSurface> {
    ensure!(
        bytes.len() <= MAX_DECODED_BYTES,
        "encoded image exceeds 64 MiB"
    );
    if bytes.starts_with(b"DDS ") {
        if let Some(surface) = dds::decode(bytes)? {
            return Ok(surface);
        }
    }
    let mut reader = image::io::Reader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_DECODED_BYTES as u64);
    reader.limits(limits);
    let image = reader.decode()?.to_rgba8();
    let surface = ImageSurface {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    };
    surface.validate()?;
    Ok(surface)
}
struct Standard;
struct Pdt;
impl ImageDecoder for Pdt {
    fn decode(&self, bytes: &[u8]) -> Result<ImageSurface> {
        let p = pdt::decode_pdt(bytes)?;
        let mut image = ImageSurface {
            width: p.width,
            height: p.height,
            rgba: p.rgba,
        };
        // PDT stores mask-associated RGB. Normalize once in the shared codec,
        // so every Layer/Bitmap consumer receives conventional straight RGBA.
        if p.has_mask {
            for c in image.rgba.chunks_exact_mut(4) {
                let a = u32::from(c[3]);
                if a != 0 && a != 255 {
                    for x in &mut c[..3] {
                        *x = (u32::from(*x) * 255 / a).min(255) as u8;
                    }
                }
            }
        }
        image.validate()?;
        Ok(image)
    }
}
impl ImageDecoder for Standard {
    fn decode(&self, bytes: &[u8]) -> Result<ImageSurface> {
        decode_standard(bytes)
    }
}
#[derive(Clone)]
pub struct DecoderRegistry {
    decoders: BTreeMap<String, Arc<dyn ImageDecoder>>,
    formats: BTreeMap<&'static str, Arc<dyn ImageFormat>>,
}
impl Default for DecoderRegistry {
    fn default() -> Self {
        let mut this = Self {
            decoders: BTreeMap::new(),
            formats: BTreeMap::new(),
        };
        for extension in [
            "png", "jpg", "jpeg", "bmp", "dds", "tga", "webp", "gif", "tif", "tiff", "pnm", "ppm",
            "pgm", "pbm", "ico", "hdr",
        ] {
            this.decoders.insert(extension.into(), Arc::new(Standard));
        }
        this.decoders.insert("pdt".into(), Arc::new(Pdt));
        this.formats.insert("circus/crx", Arc::new(crx::Format));
        this
    }
}
impl DecoderRegistry {
    pub fn register(&mut self, extension: &str, decoder: Arc<dyn ImageDecoder>) -> Result<()> {
        ensure!(
            !extension.is_empty()
                && extension
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
            "invalid decoder extension"
        );
        ensure!(
            !self.decoders.contains_key(extension),
            "duplicate image decoder"
        );
        self.decoders.insert(extension.into(), decoder);
        Ok(())
    }
    pub fn decode(&self, path: &str, bytes: &[u8]) -> Result<ImageSurface> {
        self.decode_with(path, bytes, None, None, None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_surfaces_and_unknown_formats_fail() {
        assert!(rgba_len(u32::MAX, 2).is_err());
        assert!(ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![0; 3]
        }
        .validate()
        .is_err());
        assert!(DecoderRegistry::default()
            .decode("native.g00", &[])
            .is_err());
    }
    #[test]
    fn channels_and_registration() {
        let mut p = [1, 2, 3, 4];
        swap_red_blue(&mut p).unwrap();
        assert_eq!(p, [3, 2, 1, 4]);
        assert!(swap_red_blue(&mut [0; 3]).is_err());
        assert!(DecoderRegistry::default()
            .register("png", Arc::new(Standard))
            .is_err());
    }
}

/// Encode a checked CPU surface for the shared immutable resource store.
pub fn encode_png(surface: &ImageSurface) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    surface.validate()?;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(
        &surface.rgba,
        surface.width,
        surface.height,
        image::ColorType::Rgba8,
    )?;
    Ok(out)
}

/// CPU surfaces with explicit legacy /255 composition and affine sampling.
pub mod surface;

pub mod avg32;
pub mod g00;
/// Shared MPL PDT10/PDT11 image codec; native cut/pattern metadata stays in adapters.
pub mod pdt;
pub mod reallive_wipe;

pub mod cbg;

pub mod vector_geometry;

/// Engine image backing types. Native adapters share the established image
/// library rather than retaining a private implementation of pixel storage.
pub mod buffer {
    pub use image::{
        DynamicImage, GenericImageView, GrayAlphaImage, ImageBuffer, LumaA, Rgb, Rgba, RgbaImage,
    };
}
