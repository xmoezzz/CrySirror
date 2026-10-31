//! Windows cursor resources: `.cur`/`.ico` icon directories and `.ani`
//! animated cursors (RIFF `ACON`). Decodes to straight RGBA frames with a
//! hotspot and a frame schedule; no window-system or VM policy.
//!
//! Layout references: the ICONDIR/ICONDIRENTRY and BITMAPINFOHEADER formats
//! documented for `LoadImage`/`CreateIconFromResourceEx`, and the `anih`,
//! `rate`, `seq ` and `LIST fram` chunks read by `LoadCursorFromFile` for
//! animated cursors (display rates are in jiffies, 1/60 s).
use crate::ImageSurface;
use anyhow::{Context, Result, bail, ensure};
mod sprite;
pub use sprite::{SpriteCursorLayout, SpriteHotspot, cursor_from_sprite, sprite_source_size};

/// Largest cursor edge accepted; Windows cursors are at most 256 pixels.
pub const MAX_CURSOR_EDGE: u32 = 256;
/// Upper bound on frames/steps of one animated cursor.
pub const MAX_CURSOR_FRAMES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorFrame {
    pub image: ImageSurface,
    /// Hotspot in image pixels; `.ico` images use their centre-less default (0,0).
    pub hotspot: [u32; 2],
}

/// A decoded cursor: one frame for `.cur`, the step sequence for `.ani`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorImage {
    pub frames: Vec<CursorFrame>,
    /// Display order as indices into `frames`, with each step's duration in
    /// milliseconds. A still cursor has one step of duration 0.
    pub steps: Vec<(usize, u32)>,
}

impl CursorImage {
    pub fn is_animated(&self) -> bool {
        self.steps.len() > 1
    }
}

/// Decode a `.cur`, `.ico` or `.ani` resource by its magic.
pub fn decode_cursor(bytes: &[u8]) -> Result<CursorImage> {
    if bytes.starts_with(b"CUR16M") {
        return decode_cur16m(bytes);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"ACON" {
        return decode_ani(bytes);
    }
    let frame = decode_icon_dir(bytes)?;
    Ok(CursorImage {
        frames: vec![frame],
        steps: vec![(0, 0)],
    })
}

/// AVG32 CUR16M BGR/mask cursor resources. The native 150-ms frame policy is
/// represented as data in the shared cursor schedule, without a VM painter.
/// Layout/colour-key provenance: siglus_rs 5f358ef1360b0439e92aed2b749e8f25d8721954,
/// crates/avg32/src/cursor.rs and system.rs, Mozilla Public License 2.0.
pub fn decode_cur16m(bytes: &[u8]) -> Result<CursorImage> {
    const EDGE: usize = 32;
    const COLORS: usize = EDGE * EDGE * 3;
    const FRAME: usize = COLORS + EDGE * EDGE / 8;
    ensure!(
        bytes.len() >= 40 && bytes.starts_with(b"CUR16M"),
        "not a CUR16M cursor"
    );
    let data = &bytes[40..];
    ensure!(
        data.len() % FRAME == 0 && (1..=MAX_CURSOR_FRAMES).contains(&(data.len() / FRAME)),
        "CUR16M frame count/length"
    );
    let hotspot = [32, 36]
        .map(|i| i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()).clamp(0, 31) as u32);
    let frames = data
        .chunks_exact(FRAME)
        .map(|frame| {
            let (bgr, mask) = frame.split_at(COLORS);
            let mut rgba = Vec::with_capacity(EDGE * EDGE * 4);
            for (i, pixel) in bgr.chunks_exact(3).enumerate() {
                let rgb = [pixel[2], pixel[1], pixel[0]];
                let alpha = if mask[i / 8] & (0x80 >> (i % 8)) == 0 && rgb != [255, 0, 0] {
                    255
                } else {
                    0
                };
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
            }
            CursorFrame {
                image: ImageSurface {
                    width: 32,
                    height: 32,
                    rgba,
                },
                hotspot,
            }
        })
        .collect::<Vec<_>>();
    let steps = (0..frames.len()).map(|i| (i, 150)).collect();
    Ok(CursorImage { frames, steps })
}

fn u16_at(b: &[u8], o: usize) -> Result<u16> {
    let s = b.get(o..o + 2).context("truncated cursor data")?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    let s = b.get(o..o + 4).context("truncated cursor data")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One image of an ICONDIR (type 1 = icon, 2 = cursor). The largest entry
/// is chosen, preferring higher bit depth, as the shell does for a cursor
/// without a requested size.
pub fn decode_icon_dir(bytes: &[u8]) -> Result<CursorFrame> {
    ensure!(u16_at(bytes, 0)? == 0, "not an icon directory");
    let kind = u16_at(bytes, 2)?;
    ensure!(
        kind == 1 || kind == 2,
        "icon directory type {kind} is not icon/cursor"
    );
    let count = usize::from(u16_at(bytes, 4)?);
    ensure!(
        (1..=MAX_CURSOR_FRAMES).contains(&count),
        "icon directory has {count} images"
    );
    let mut best: Option<(u64, usize)> = None;
    for i in 0..count {
        let e = 6 + i * 16;
        let w = u32::from(*bytes.get(e).context("truncated icon entry")?);
        let h = u32::from(bytes[e + 1]);
        let (w, h) = (if w == 0 { 256 } else { w }, if h == 0 { 256 } else { h });
        let bits = if kind == 1 {
            u64::from(u16_at(bytes, e + 6)?)
        } else {
            0
        };
        let score = u64::from(w * h) << 8 | bits;
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, i));
        }
    }
    let e = 6 + best.context("empty icon directory")?.1 * 16;
    let hotspot = if kind == 2 {
        [
            u32::from(u16_at(bytes, e + 4)?),
            u32::from(u16_at(bytes, e + 6)?),
        ]
    } else {
        [0, 0]
    };
    let size = u32_at(bytes, e + 8)? as usize;
    let offset = u32_at(bytes, e + 12)? as usize;
    let data = bytes
        .get(offset..offset.checked_add(size).context("icon entry size")?)
        .context("icon entry outside the file")?;
    let image = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        let img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
            .context("PNG cursor image")?
            .to_rgba8();
        ImageSurface {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        }
    } else {
        decode_dib_icon(data)?
    };
    ensure!(
        image.width <= MAX_CURSOR_EDGE && image.height <= MAX_CURSOR_EDGE,
        "cursor image exceeds {MAX_CURSOR_EDGE} pixels"
    );
    let hotspot = [
        hotspot[0].min(image.width - 1),
        hotspot[1].min(image.height - 1),
    ];
    Ok(CursorFrame { image, hotspot })
}

/// A BITMAPINFOHEADER icon image: XOR colour bitmap followed by the 1-bpp
/// AND mask; the header height counts both. Bottom-up rows, 4-byte aligned.
fn decode_dib_icon(data: &[u8]) -> Result<ImageSurface> {
    let header = u32_at(data, 0)? as usize;
    ensure!(
        header >= 40 && header <= data.len(),
        "unsupported icon bitmap header"
    );
    let width = u32_at(data, 4)? as i32;
    let height2 = u32_at(data, 8)? as i32;
    let bpp = u16_at(data, 14)?;
    let compression = u32_at(data, 16)?;
    ensure!(
        compression == 0,
        "compressed icon bitmaps are not supported"
    );
    ensure!(
        width > 0 && height2 > 0 && height2 % 2 == 0,
        "invalid icon bitmap size"
    );
    let (w, h) = (width as u32, (height2 / 2) as u32);
    ensure!(
        w <= MAX_CURSOR_EDGE && h <= MAX_CURSOR_EDGE,
        "icon bitmap too large"
    );
    let palette_len = match bpp {
        1 | 4 | 8 => {
            let used = u32_at(data, 32)? as usize;
            if used == 0 {
                1usize << bpp
            } else {
                used.min(1 << bpp)
            }
        }
        24 | 32 => 0,
        _ => bail!("icon bitmap depth {bpp} is not supported"),
    };
    let palette = data
        .get(header..header + palette_len * 4)
        .context("truncated icon palette")?;
    let xor_stride = (w as usize * usize::from(bpp)).div_ceil(32) * 4;
    let and_stride = (w as usize).div_ceil(32) * 4;
    let xor_at = header + palette_len * 4;
    let and_at = xor_at + xor_stride * h as usize;
    let xor = data
        .get(xor_at..and_at)
        .context("truncated icon colour bitmap")?;
    // A 32-bpp image may omit its AND mask; treat it as fully opaque mask.
    let and = data.get(and_at..and_at + and_stride * h as usize);
    ensure!(and.is_some() || bpp == 32, "truncated icon mask");
    let mut rgba = vec![0u8; crate::rgba_len(w, h)?];
    let mut any_alpha = false;
    for y in 0..h as usize {
        let src_row = h as usize - 1 - y;
        let row = &xor[src_row * xor_stride..][..xor_stride];
        for x in 0..w as usize {
            let px = &mut rgba[(y * w as usize + x) * 4..][..4];
            match bpp {
                32 => {
                    let p = &row[x * 4..][..4];
                    px.copy_from_slice(&[p[2], p[1], p[0], p[3]]);
                    any_alpha |= p[3] != 0;
                }
                24 => {
                    let p = &row[x * 3..][..3];
                    px.copy_from_slice(&[p[2], p[1], p[0], 255]);
                }
                _ => {
                    let per = 8 / usize::from(bpp);
                    let byte = row[x / per];
                    let shift = 8 - usize::from(bpp) * (x % per + 1);
                    let index = usize::from(byte >> shift) & ((1 << bpp) - 1);
                    let p = palette
                        .get(index * 4..index * 4 + 4)
                        .context("icon palette index")?;
                    px.copy_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
        }
    }
    // The AND mask gives transparency unless a 32-bpp image carries alpha.
    if let Some(and) = and.filter(|_| !(bpp == 32 && any_alpha)) {
        for y in 0..h as usize {
            let row = &and[(h as usize - 1 - y) * and_stride..][..and_stride];
            for x in 0..w as usize {
                let transparent = row[x / 8] >> (7 - x % 8) & 1 == 1;
                let px = &mut rgba[(y * w as usize + x) * 4..][..4];
                if transparent {
                    // XOR-inverting pixels (mask 1, colour non-black) have
                    // no RGBA equivalent; they become transparent.
                    px.copy_from_slice(&[0, 0, 0, 0]);
                } else {
                    px[3] = 255;
                }
            }
        }
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}

/// RIFF `ACON`: `anih` header, optional `rate`/`seq ` tables and a
/// `LIST fram` of `icon` chunks, each a complete icon directory.
fn decode_ani(bytes: &[u8]) -> Result<CursorImage> {
    let riff_len = u32_at(bytes, 4)? as usize;
    let end = (8 + riff_len).min(bytes.len());
    let mut at = 12;
    let mut header = None;
    let mut rates = None;
    let mut seq = None;
    let mut frames = Vec::new();
    while at + 8 <= end {
        let id = &bytes[at..at + 4];
        let len = u32_at(bytes, at + 4)? as usize;
        let body = bytes
            .get(at + 8..at + 8 + len)
            .context("truncated ANI chunk")?;
        match id {
            b"anih" => {
                ensure!(len >= 36, "short anih chunk");
                let frames = u32_at(body, 4)? as usize;
                let steps = u32_at(body, 8)? as usize;
                let rate = u32_at(body, 28)?;
                let flags = u32_at(body, 32)?;
                ensure!(flags & 1 == 1, "raw-bitmap ANI frames are not supported");
                header = Some((frames, steps, rate));
            }
            b"rate" => rates = Some(read_u32s(body)?),
            b"seq " => seq = Some(read_u32s(body)?),
            b"LIST" if body.get(0..4) == Some(b"fram") => {
                let mut f = 4;
                while f + 8 <= body.len() {
                    let flen = u32_at(body, f + 4)? as usize;
                    let fbody = body
                        .get(f + 8..f + 8 + flen)
                        .context("truncated ANI frame")?;
                    if &body[f..f + 4] == b"icon" {
                        ensure!(frames.len() < MAX_CURSOR_FRAMES, "too many ANI frames");
                        frames.push(decode_icon_dir(fbody)?);
                    }
                    f += 8 + flen + (flen & 1);
                }
            }
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let (nframes, nsteps, rate) = header.context("ANI cursor without anih")?;
    ensure!(
        !frames.is_empty() && frames.len() == nframes,
        "ANI frame count mismatch"
    );
    let nsteps = if nsteps == 0 { nframes } else { nsteps };
    ensure!(nsteps <= MAX_CURSOR_FRAMES, "too many ANI steps");
    let order = seq.unwrap_or_else(|| (0..nsteps as u32).collect());
    ensure!(order.len() >= nsteps, "short ANI seq table");
    let mut steps = Vec::with_capacity(nsteps);
    for (i, &index) in order.iter().take(nsteps).enumerate() {
        ensure!(
            (index as usize) < frames.len(),
            "ANI seq index out of range"
        );
        let jiffies = rates
            .as_ref()
            .and_then(|r| r.get(i).copied())
            .unwrap_or(rate);
        steps.push((index as usize, jiffies.max(1).saturating_mul(1000) / 60));
    }
    Ok(CursorImage { frames, steps })
}

fn read_u32s(body: &[u8]) -> Result<Vec<u32>> {
    ensure!(body.len() / 4 <= MAX_CURSOR_FRAMES, "ANI table too long");
    (0..body.len() / 4).map(|i| u32_at(body, i * 4)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 1-bpp cursor: palette black/white, hotspot (1,0), the top-left
    /// pixel masked out.
    pub(crate) fn tiny_cur() -> Vec<u8> {
        let mut dib = Vec::new();
        for v in [40u32, 2, 4] {
            dib.extend(v.to_le_bytes());
        }
        dib.extend(1u16.to_le_bytes()); // planes
        dib.extend(1u16.to_le_bytes()); // bpp
        dib.extend([0u8; 24]); // compression .. important colours
        dib.extend([0, 0, 0, 0, 255, 255, 255, 0]); // palette
        // XOR rows bottom-up: bottom row = white,black; top row = white,white
        dib.extend([0b1000_0000, 0, 0, 0, 0b1100_0000, 0, 0, 0]);
        // AND rows bottom-up: bottom opaque; top-left transparent
        dib.extend([0, 0, 0, 0, 0b1000_0000, 0, 0, 0]);
        let mut f = vec![0, 0, 2, 0, 1, 0];
        f.extend([2, 2, 0, 0]);
        f.extend(1u16.to_le_bytes());
        f.extend(0u16.to_le_bytes());
        f.extend((dib.len() as u32).to_le_bytes());
        f.extend(22u32.to_le_bytes());
        f.extend(dib);
        f
    }

    #[test]
    fn decodes_a_monochrome_cursor_with_mask_and_hotspot() {
        let c = decode_cursor(&tiny_cur()).unwrap();
        assert!(!c.is_animated());
        let f = &c.frames[0];
        assert_eq!((f.image.width, f.image.height, f.hotspot), (2, 2, [1, 0]));
        assert_eq!(
            f.image.rgba,
            [
                0, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255
            ]
        );
    }

    #[test]
    fn decodes_an_animated_cursor_schedule() {
        let cur = tiny_cur();
        let mut fram = b"fram".to_vec();
        for _ in 0..2 {
            fram.extend(b"icon");
            fram.extend((cur.len() as u32).to_le_bytes());
            fram.extend(&cur);
            if cur.len() % 2 == 1 {
                fram.push(0);
            }
        }
        let mut anih = Vec::new();
        for v in [36u32, 2, 3, 0, 0, 0, 0, 6, 1] {
            anih.extend(v.to_le_bytes());
        }
        let mut body = b"ACON".to_vec();
        for (id, data) in [
            (&b"anih"[..], anih),
            (
                b"seq ",
                [0u32, 1, 0].iter().flat_map(|v| v.to_le_bytes()).collect(),
            ),
            (b"LIST", fram),
        ] {
            body.extend(id);
            body.extend((data.len() as u32).to_le_bytes());
            body.extend(data);
        }
        let mut riff = b"RIFF".to_vec();
        riff.extend((body.len() as u32).to_le_bytes());
        riff.extend(body);
        let c = decode_cursor(&riff).unwrap();
        assert!(c.is_animated());
        assert_eq!(c.frames.len(), 2);
        assert_eq!(c.steps, vec![(0, 100), (1, 100), (0, 100)]);
    }

    #[test]
    fn rejects_non_cursor_data() {
        assert!(decode_cursor(b"\x89PNG\r\n\x1a\n").is_err());
    }

    #[test]
    fn cur16m_preserves_native_mask_colour_key_hotspot_and_frame_schedule() {
        let mut bytes = b"CUR16M\0\0".to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(6408u32.to_le_bytes());
        bytes.extend([0; 16]);
        bytes.extend(1i32.to_le_bytes());
        bytes.extend(30i32.to_le_bytes());
        let mut frame = vec![0u8; 3200];
        frame[..9].copy_from_slice(&[0x61, 0xad, 0xff, 0, 0, 255, 9, 8, 7]);
        frame[3072] = 0x20; // Third pixel masked; pure red second pixel is also transparent.
        bytes.extend(&frame);
        bytes.extend(&frame);
        let image = decode_cursor(&bytes).unwrap();
        assert_eq!(image.steps, [(0, 150), (1, 150)]);
        assert_eq!(image.frames[0].hotspot, [1, 30]);
        assert_eq!(image.frames[0].image.width, 32);
        assert_eq!(
            &image.frames[0].image.rgba[..12],
            &[255, 173, 97, 255, 255, 0, 0, 0, 7, 8, 9, 0]
        );
        bytes.push(0);
        assert!(decode_cur16m(&bytes).is_err());
        assert!(decode_cur16m(b"CUR16M").is_err());
    }
}
