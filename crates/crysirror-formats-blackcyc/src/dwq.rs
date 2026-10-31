// SPDX-License-Identifier: MIT
// DWQ layouts/row coding derived from GARbro ImageDWQ.cs, morkt 2015-2017.
use crate::{header, u16at, u32at};
use anyhow::{bail, ensure, Context, Result};
use crysirror_image::{rgba_len, ImageDecodeContext, ImageFormat, ImageSurface, MAX_DECODED_BYTES};
pub struct Dwq;
#[derive(Clone, Debug)]
pub struct DwqInfo {
    pub size: [u32; 2],
    pub pack: u16,
    pub alpha_suffix: bool,
    pub png_color_type: Option<u8>,
    pub png_transparency: bool,
}
impl Dwq {
    pub fn metadata(d: &[u8]) -> Result<DwqInfo> {
        let h = header(d)?;
        let size = Self::dimensions(d)?;
        let (mut png_color_type, mut png_transparency) = (None, false);
        if h.pack == 8 {
            let png = d.get(64..).context("DWQ PNG absent")?;
            ensure!(
                png.get(..8) == Some(b"\x89PNG\r\n\x1a\n") && png.get(12..16) == Some(b"IHDR"),
                "DWQ PNG header"
            );
            let kind = *png.get(25).context("DWQ PNG colour type absent")?;
            ensure!(matches!(kind, 0 | 2 | 3 | 4 | 6), "DWQ PNG colour type");
            png_color_type = Some(kind);
            let mut at = 8usize;
            for _ in 0..65536 {
                let header = png
                    .get(at..at + 8)
                    .context("DWQ PNG chunk header truncated")?;
                let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
                let end = at
                    .checked_add(12)
                    .and_then(|at| at.checked_add(len))
                    .context("DWQ PNG chunk overflow")?;
                ensure!(end <= png.len(), "DWQ PNG chunk outside payload");
                if &header[4..8] == b"tRNS" {
                    png_transparency = true;
                }
                if (&header[4..8] == b"IDAT" || &header[4..8] == b"IEND") {
                    break;
                }
                at = end;
            }
        }
        Ok(DwqInfo {
            size,
            pack: h.pack,
            alpha_suffix: h.alpha,
            png_color_type,
            png_transparency,
        })
    }
    /// Bounded header metadata for worker admission; the decoder still checks
    /// embedded payload dimensions before any resource can be published.
    pub fn dimensions(d: &[u8]) -> Result<[u32; 2]> {
        let h = header(d)?;
        let body = d.get(64..).context("DWQ body absent")?;
        let if_bmp = d.starts_with(b"IF PACKTYPE==");
        let size = if if_bmp {
            ensure!(
                d.get(13..15) == Some(b"0 ")
                    && d.get(44..48) == Some(b"BMP ")
                    && matches!(h.pack, 0 | 1),
                "unsupported DWQ IF-BMP profile"
            );
            let w = u32at(body, 18)? as i32;
            let ht = u32at(body, 22)? as i32;
            ensure!(w > 0 && ht != i32::MIN, "DWQ IF-BMP dimensions");
            [w as u32, ht.unsigned_abs()]
        } else {
            [u32at(d, 36)?, u32at(d, 40)?]
        };
        rgba_len(size[0], size[1])?;
        Ok(size)
    }
}
struct Bitmap {
    width: u32,
    height: u32,
    bpp: u16,
    stride: usize,
    palette: Vec<[u8; 3]>,
    bytes: Vec<u8>,
}
fn bitmap(d: &[u8], size: [u32; 2], packed: bool, cx: &ImageDecodeContext<'_>) -> Result<Bitmap> {
    ensure!(d.get(..2) == Some(b"BM"), "DWQ embedded BMP mark");
    ensure!(u32at(d, 14)? == 40, "unsupported DWQ BMP DIB profile");
    let width = u32at(d, 18)? as i32;
    let height = u32at(d, 22)? as i32;
    ensure!(
        width > 0 && height != i32::MIN && height != 0,
        "DWQ BMP signed dimensions"
    );
    ensure!(
        [width as u32, height.unsigned_abs()] == size,
        "DWQ embedded BMP dimensions"
    );
    if !packed {
        ensure!(
            height > 0,
            "unsupported DWQ plain BMP signed-height profile"
        );
    }
    rgba_len(size[0], size[1])?;
    ensure!(u16at(d, 26)? == 1, "DWQ BMP planes");
    let bpp = u16at(d, 28)?;
    ensure!(
        matches!(bpp, 8 | 16 | 24 | 32),
        "unsupported DWQ BMP depth {bpp}"
    );
    let row = size[0] as usize * (bpp / 8) as usize;
    let stride = if packed { row } else { (row + 3) & !3 };
    let mut start = 54usize;
    let mut palette = Vec::new();
    if bpp == 8 {
        let mut count = u32at(d, 46)? as usize;
        if packed && count == 0 {
            count = 256;
        }
        ensure!(
            (1..=256).contains(&count),
            "unsupported DWQ BMP palette count"
        );
        let colors = d
            .get(start..start + count * 4)
            .context("DWQ BMP palette bounds")?;
        palette = colors.chunks_exact(4).map(|p| [p[2], p[1], p[0]]).collect();
        start += count * 4;
    }
    let offset = u32at(d, 10)? as usize;
    if packed {
        ensure!(
            offset >= start && offset <= d.len(),
            "DWQ packed pixel offset"
        );
        start = offset;
    } else {
        ensure!(offset == start, "unsupported DWQ plain BMP pixel offset");
    }
    let mut bytes = vec![0; stride * size[1] as usize];
    if !packed {
        let len = bytes.len();
        bytes.copy_from_slice(
            d.get(start..start + len)
                .context("DWQ plain BMP pixel bounds")?,
        );
    } else {
        let mut cursor = start;
        let mut commands = 0usize;
        for y in 0..size[1] as usize {
            cx.check_cancelled()?;
            let mut x = 0;
            while x < stride {
                let byte = *d.get(cursor).context("DWQ packed row truncated")?;
                cursor += 1;
                if byte != 0 {
                    bytes[y * stride + x] = byte;
                    x += 1;
                } else {
                    let run = usize::from(*d.get(cursor).context("DWQ zero-run truncated")?);
                    cursor += 1;
                    ensure!(run <= stride - x, "DWQ zero-run crosses row");
                    x += run;
                }
                commands += 1;
                if commands % 4096 == 0 {
                    cx.check_cancelled()?;
                }
            }
            if y > 0 {
                for x in 0..stride {
                    bytes[y * stride + x] ^= bytes[(y - 1) * stride + x];
                }
            }
        }
    }
    Ok(Bitmap {
        width: size[0],
        height: size[1],
        bpp,
        stride,
        palette,
        bytes,
    })
}
fn color(b: &Bitmap, packed: bool, cx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut rgba = vec![0; rgba_len(b.width, b.height)?];
    for y in 0..b.height as usize {
        cx.check_cancelled()?;
        for x in 0..b.width as usize {
            let at = y * b.stride + x * (b.bpp / 8) as usize;
            let rgb = match b.bpp {
                8 => *b
                    .palette
                    .get(b.bytes[at] as usize)
                    .context("DWQ color palette index")?,
                16 => {
                    let v = u16::from_le_bytes([b.bytes[at], b.bytes[at + 1]]);
                    let r = ((v >> 11) & 31) as u8;
                    let g = ((v >> 5) & 63) as u8;
                    let z = (v & 31) as u8;
                    [
                        (r << 3) | (r >> 2),
                        (g << 2) | (g >> 4),
                        (z << 3) | (z >> 2),
                    ]
                }
                _ if packed => [b.bytes[at + 2], b.bytes[at + 1], b.bytes[at]],
                _ => [b.bytes[at], b.bytes[at + 1], b.bytes[at + 2]],
            };
            let dst = (y * b.width as usize + x) * 4;
            rgba[dst..dst + 3].copy_from_slice(&rgb);
            rgba[dst + 3] = 255;
        }
    }
    Ok(ImageSurface {
        width: b.width,
        height: b.height,
        rgba,
    })
}
impl ImageFormat for Dwq {
    fn id(&self) -> &'static str {
        "black-cyc/dwq"
    }
    fn recognize(&self, path: &str, d: &[u8]) -> bool {
        (path
            .rsplit('.')
            .next()
            .is_some_and(|e| e.eq_ignore_ascii_case("dwq"))
            || [
                b"PNG ".as_slice(),
                b"JPEG",
                b"BMP ",
                b"PACK",
                b"IF P",
                b"BMP+",
            ]
            .iter()
            .any(|s| d.starts_with(s)))
            && header(d).is_ok()
    }
    fn decode(&self, d: &[u8], cx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
        cx.check_cancelled()?;
        ensure!(d.len() <= MAX_DECODED_BYTES, "DWQ encoded byte limit");
        let h = header(d)?;
        let body = d.get(64..).context("DWQ body absent")?;
        let if_bmp = d.starts_with(b"IF PACKTYPE==");
        let size = Self::dimensions(d)?;
        let packed = match h.pack {
            0 | 5 | 8 => body.len(),
            2 | 3 | 7 => u32at(d, 32)? as usize,
            1 if if_bmp => body.len(),
            _ => bail!("unsupported DWQ PACKTYPE {}", h.pack),
        };
        let input = body.get(..packed).context("DWQ packed color span")?;
        let mut image = match h.pack {
            5 | 7 => cx.decode_embedded("payload.jpg", input)?,
            8 => cx.decode_embedded("payload.png", input)?,
            0 | 2 => color(&bitmap(input, size, false, cx)?, false, cx)?,
            1 | 3 => color(&bitmap(input, size, true, cx)?, true, cx)?,
            _ => unreachable!(),
        };
        ensure!(
            [image.width, image.height] == size,
            "DWQ embedded codec dimension mismatch"
        );
        if matches!(h.pack, 2 | 3 | 7) && (h.alpha || matches!(h.pack, 3 | 7)) {
            let mask = body
                .get(packed..)
                .filter(|d| !d.is_empty())
                .context("DWQ declared alpha mask absent")?;
            let alpha = bitmap(mask, size, true, cx)?;
            ensure!(alpha.bpp == 8, "DWQ alpha mask must be indexed8");
            for y in 0..size[1] as usize {
                cx.check_cancelled()?;
                for x in 0..size[0] as usize {
                    let index = alpha.bytes[y * alpha.stride + x] as usize;
                    let rgb = alpha
                        .palette
                        .get(index)
                        .context("DWQ alpha palette index")?;
                    image.rgba[(y * size[0] as usize + x) * 4 + 3] =
                        ((u16::from(rgb[0]) + u16::from(rgb[1]) + u16::from(rgb[2])) / 3) as u8;
                }
            }
        }
        image.validate()?;
        Ok(image)
    }
}
