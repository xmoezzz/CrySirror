//! Explicit packed-pixel interchange, independent of an archive or VM.
use crate::{ImageSurface, rgba_len};
use anyhow::{Result, ensure};
use crysirror_core::image_processing::PackedPixelLayout;
pub fn decode(
    size: [u32; 2],
    layout: PackedPixelLayout,
    pixels: &[u8],
    check: &dyn Fn() -> Result<()>,
) -> Result<ImageSurface> {
    let len = rgba_len(size[0], size[1])?;
    let required = len / 4 * layout.bytes_per_pixel();
    ensure!(pixels.len() == required, "packed-pixel source length");
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(len)?;
    for (index, px) in pixels.chunks_exact(layout.bytes_per_pixel()).enumerate() {
        if index % 16384 == 0 {
            check()?;
        }
        let value = match layout {
            PackedPixelLayout::Bgr555 => {
                let word = u16::from_le_bytes([px[0], px[1]]);
                let channel = |shift: u16| (((word >> shift) & 0x1f) << 3) as u8;
                [channel(10), channel(5), channel(0), 255]
            }
            PackedPixelLayout::Bgr8 | PackedPixelLayout::Bgrx8 => [px[2], px[1], px[0], 255],
            PackedPixelLayout::Bgra8 => [px[2], px[1], px[0], px[3]],
            PackedPixelLayout::Red8 => [px[0], 0, 0, 255],
        };
        rgba.extend_from_slice(&value);
    }
    Ok(ImageSurface {
        width: size[0],
        height: size[1],
        rgba,
    })
}
pub fn encode(
    size: [u32; 2],
    rgba: &[u8],
    layout: PackedPixelLayout,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<u8>> {
    let len = rgba_len(size[0], size[1])?;
    ensure!(len == rgba.len(), "packed-pixel RGBA length");
    let mut out = Vec::new();
    out.try_reserve_exact(len / 4 * layout.bytes_per_pixel())?;
    for (index, pixel) in rgba.chunks_exact(4).enumerate() {
        if index % 16384 == 0 {
            check()?;
        }
        let [r, g, b, a]: [u8; 4] = pixel.try_into().unwrap();
        match layout {
            PackedPixelLayout::Bgr555 => {
                let word = (u16::from(r >> 3) << 10) | (u16::from(g >> 3) << 5) | u16::from(b >> 3);
                out.extend_from_slice(&word.to_le_bytes());
            }
            PackedPixelLayout::Bgr8 => out.extend_from_slice(&[b, g, r]),
            PackedPixelLayout::Bgrx8 => out.extend_from_slice(&[b, g, r, 0]),
            PackedPixelLayout::Bgra8 => out.extend_from_slice(&[b, g, r, a]),
            PackedPixelLayout::Red8 => out.push(r),
        }
    }
    Ok(out)
}
