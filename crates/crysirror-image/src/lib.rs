// SPDX-License-Identifier: MPL-2.0
//! CPU-only image surfaces and decoder dispatch. No GPU, window or VM policy.
#![forbid(unsafe_code)]
pub mod block_compressed;
pub mod label_image;
pub mod matte_span;
pub mod lens;
pub mod radial_phase;
pub mod row_shift;
pub mod streak_field;
pub mod tile_compose;
pub mod png_rows;
pub mod pna;
pub mod psd;
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, io::Cursor, sync::Arc};
mod exhibit_gyu;
mod fixed_blend;
mod formats;
mod standard_encoding;
pub use formats::{ImageDecodeContext, ImageFormat, ImageSource};

pub const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;
pub mod nvsg;
pub mod sqz;
pub mod padding;
pub mod pixels;
mod png;
pub mod wipe_mask;
pub use png::{encode_png_uncompressed, png_uncompressed_len};
pub mod alpha_clip;
pub mod layer_ex;
pub mod resample_byte256;
pub mod resample_byte255;
/// Longest decoded image edge. Original sprite sheets exceed 8192 (Little
/// Busters! `EF_RPW_RP08.g00` is 1200x12000); `MAX_DECODED_BYTES` bounds memory
/// and the renderer validates uploads against the actual device limit.
pub const MAX_IMAGE_DIMENSION: u32 = 16384;
pub fn rgba_len(width: u32, height: u32) -> Result<usize> {
    ensure!(
        width > 0 && height > 0 && width <= MAX_IMAGE_DIMENSION && height <= MAX_IMAGE_DIMENSION,
        "image dimensions must be 1..{MAX_IMAGE_DIMENSION}"
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
pub mod affine_blend;
pub mod associated;
pub mod bitmap;
pub mod box_blur_clamped;
pub mod feedback_box_blur;
pub mod box_blur_span;
pub mod nearest_cell_sample;
pub mod box_downscale;
pub mod tiled_transform;
pub mod tile_scatter;
pub mod color;
pub mod composition;
pub mod crx;
pub mod crxd;
pub mod kiss_tex;
pub mod cursor;
pub mod cximage;
mod dds;
pub mod dual_texture;
pub mod effects;
pub mod nine_patch;
pub mod coverage_bitmap;
pub mod coverage_blur;
pub mod fixed_mix;
pub mod fixed_raster;
pub mod fixed_affine;
pub mod byte_raster;
pub mod row_blit;
pub mod packed_color_mask;
pub mod mask_field_pair;
pub mod axis_box_sample;
pub mod repeat_scale_sample;
pub mod stored_gradient_spans;
pub mod stored_word_spans;
pub mod linear_mask_offset;
pub mod spatial_color_band;
pub mod packed_radial_field;
pub mod radial_wave_pair;
pub mod packed_dual_mask;
pub mod fixed_scanlines;
pub mod masked_block_sample;
pub mod elliptic_paint;
pub mod indexed_plane;
pub mod clipped_scanline_sample;
pub mod floating_lines;
pub mod coverage_bounds;
pub mod neighbor_relief;
pub mod native_box_blur;
pub mod row_tint_blend;
pub mod masked_backdrop_shift;
pub mod masked_clip_blit;
pub mod packed_pair_blend;
pub mod integer_wipe;
pub mod axis_projection;
pub mod triangle_raster;
pub mod scanline_affine;
pub mod tile_reveal;
pub mod radial_angle;
pub mod fixed_projection;
pub mod gradient_overlay;
pub mod linear_gradient;
pub mod linear_alpha_lookup;
pub mod sequential_box_blur;
pub mod integer_raster;
pub mod masked_blur;
pub mod packed;
pub mod packed_bitmap;
pub mod procedural_instances;
pub mod pyramid;
pub mod resize_byte;
pub mod rgb_lut;
pub mod row_copy;
pub mod sprite_alpha;
pub mod coverage_key;
pub mod cyclic_cell_sample;
pub mod stored_mask;
pub mod temporal_pattern;
pub mod transition;
pub mod vector;
pub mod integer_lines;
pub mod vector_field;
pub mod vector_scene;
pub mod warp;
pub mod wave_displace;
/// Pixel size from a KiriKiri TLG5/TLG6 header, directly or in a TLG0 `sds` wrapper.
/// Lets a caller size a surface before a shared decoder job runs.
pub fn tlg_size(bytes: &[u8]) -> Option<[u32; 2]> {
    const MAGIC: usize = 11;
    let bytes = match bytes.get(..MAGIC)? {
        b"TLG0.0\0sds\x1a" => bytes.get(MAGIC + 4..)?,
        _ => bytes,
    };
    let start = match bytes.get(..MAGIC)? {
        b"TLG5.0\0raw\x1a" => MAGIC + 1,
        b"TLG6.0\0raw\x1a" => MAGIC + 4,
        _ => return None,
    };
    let word = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    Some([word(start)?, word(start + 4)?])
}

#[cfg(test)]
mod tlg_size_tests {
    #[test]
    fn reads_plain_and_wrapped_headers() {
        let mut tlg5 = b"TLG5.0\0raw\x1a\x04".to_vec();
        tlg5.extend(7u32.to_le_bytes());
        tlg5.extend(9u32.to_le_bytes());
        assert_eq!(super::tlg_size(&tlg5), Some([7, 9]));
        let mut tlg6 = b"TLG6.0\0raw\x1a\x04\0\0\x08".to_vec();
        tlg6.extend(3u32.to_le_bytes());
        tlg6.extend(5u32.to_le_bytes());
        assert_eq!(super::tlg_size(&tlg6), Some([3, 5]));
        let mut wrapped = b"TLG0.0\0sds\x1a\0\0\0\0".to_vec();
        wrapped.extend(&tlg5);
        assert_eq!(super::tlg_size(&wrapped), Some([7, 9]));
        assert_eq!(super::encoded_dimensions_hint(&wrapped), Some([7, 9]));
        assert_eq!(super::tlg_size(b"\x89PNG"), None);
    }
}

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
        this.formats.insert(exhibit_gyu::ID, Arc::new(exhibit_gyu::Format));
        this.formats.insert("circus/crx", Arc::new(crx::Format));
        this.formats.insert(kiss_tex::ID, Arc::new(kiss_tex::Format));
        this.formats
            .insert("engine/packed-bitmap", Arc::new(packed_bitmap::Format));
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
        assert!(rgba_len(MAX_IMAGE_DIMENSION + 1, 1).is_err());
        assert_eq!(rgba_len(1200, 12000).unwrap(), 1200 * 12000 * 4);
        assert!(rgba_len(MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION).is_err());
        assert!(
            ImageSurface {
                width: 1,
                height: 1,
                rgba: vec![0; 3]
            }
            .validate()
            .is_err()
        );
        assert!(
            DecoderRegistry::default()
                .decode("native.g00", &[])
                .is_err()
        );
    }
    #[test]
    fn channels_and_registration() {
        let mut p = [1, 2, 3, 4];
        swap_red_blue(&mut p).unwrap();
        assert_eq!(p, [3, 2, 1, 4]);
        assert!(swap_red_blue(&mut [0; 3]).is_err());
        assert!(
            DecoderRegistry::default()
                .register("png", Arc::new(Standard))
                .is_err()
        );
    }
}

/// Encode a checked CPU surface as baseline JPEG; alpha is dropped.
pub fn encode_jpeg(surface: &ImageSurface, quality: u8) -> Result<Vec<u8>> {
    surface.validate()?;
    let rgb: Vec<u8> = surface
        .rgba
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode(
        &rgb,
        surface.width,
        surface.height,
        image::ColorType::Rgb8,
    )?;
    Ok(out)
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
pub mod gdi_region;
pub mod alpha_cells;
pub mod bmfont;
pub mod layer_stack;
pub mod channel_matte;
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

pub mod byte_sampling;
pub mod cosine_lens;
pub mod coverage_sum;
/// Grouped fixed-point luminance with exact integer colour mixing.
pub mod fixed_luma;
pub mod integer_color;
pub mod luma_lookup;
pub mod tonal_balance;
pub mod row_color;
pub mod shift_rows;
pub mod masked_add;
pub mod rect_resample;
pub mod shape_mask;
pub mod triangular_blur;

pub mod piecewise_resample;

pub mod progress_curve;

pub mod masked_pair;

pub mod bmp_encode;
pub mod wcg_encode;
pub mod cel_convert;
pub mod paged;

pub mod packed_pixels;

/// Header-only allocation hint for built-in image encodings. Unknown custom
/// formats return None so the caller retains its conservative byte cap.
/// Call on an I/O/CPU worker: parsing a container header can seek internally.
pub fn encoded_dimensions_hint(bytes: &[u8]) -> Option<[u32; 2]> {
    if let Some([w, h]) = tlg_size(bytes) {
        return (w > 0 && h > 0).then_some([w, h]);
    }
    // Tiled QLIE DPNG: signature, version, tile count, then canvas width/height
    // (crysirror-formats-arc qlie/dpng). Without this, admission falls back
    // to the maximum decoded size and over-reserves shared decode memory.
    if bytes.len() >= 20 && &bytes[..4] == b"DPNG" {
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let (width, height) = (word(12), word(16));
        if width > 0 && height > 0 {
            return Some([width, height]);
        }
    }
    if let Ok(info) = cbg::probe_image(bytes) {
        if let (Some(width), Some(height)) = (info.width, info.height) {
            return Some([width, height]);
        }
    }
    let reader = image::io::Reader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if reader.format().is_none() {
        return None;
    }
    let (width, height) = reader.into_dimensions().ok()?;
    Some([width, height])
}

pub mod motion;

pub mod motion_shake;

pub mod alpha_planes;

/// Shared fixed-point sprite sampling and integer particle-plane composition.
pub mod particle;

pub mod alpha_stamps;
pub mod packed_stamps;
pub mod row_strip;

pub mod alpha_mask;
pub mod block_average;
pub mod box_reduce;
pub mod random_runs;
pub mod disc_copy;
pub mod truncated_row_blur;
pub mod canvas_margins;

pub mod rect_map;
pub mod transparency_planes;

pub mod row_edge_lift;

pub mod circular_offset;

pub mod affine_sample_blend;
pub mod linear_channel_transfer;

pub mod radial_twist_sample;

pub mod spectrum_graph;

pub mod png_header;
pub mod linear_byte_storage;
pub mod radial_noise_atlas;
pub mod alpha_brushes;
pub mod packed_repeat_rows;
pub mod radial_rgb_brushes;
pub mod packed_pair_batch;
pub mod fragment_grid;

pub mod floating_polygon_fragments;
pub mod overlap_resample;
pub mod keyed_over;
