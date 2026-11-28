// SPDX-License-Identifier: MPL-2.0
//! Ordinary indexed ExHibit DLT 0x0101–0x0103, including v1 mask planes.
//!
//! Resource decoding only: the frame interval and loop hint come from the
//! file header. Script modes, placement and animation clocks are not here.
use crate::lzss::decompress_exact_12_4;
use anyhow::{Context, Result, ensure};
use std::{ops::Range, sync::Arc};

const MAX_BYTES: usize = 128 << 20;
const MAX_PIXELS: usize = 16 << 20;
const MAX_DIMENSION: u32 = 4096;
const MAX_RECORDS: u32 = 16384;
const HEAD: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DltInfo {
    pub size: [u32; 2],
    /// Display steps, including an unchanged-frame step. The palette and
    /// terminator are not displayed.
    pub frames: usize,
    pub interval_ms: u32,
    pub loop_hint: bool,
}

#[derive(Debug)]
pub struct DltFrame {
    pub index: usize,
    pub rgba: Vec<u8>,
    /// Top-left coordinate and extent of this frame's encoded rectangle.
    /// A hold has `[0; 4]` and leaves the previous image intact.
    pub rect: [u32; 4],
}

#[derive(Debug)]
enum Delta {
    Paint {
        rect: [u32; 4],
        range: Range<usize>,
        mask: Option<Range<usize>>,
    },
    Hold,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Version {
    V101,
    V102,
    V103,
}

pub struct DltDecoder {
    encoded: Arc<[u8]>,
    info: DltInfo,
    palette: [[u8; 4]; 256],
    pixels: Vec<u8>,
    base: Range<usize>,
    mask_base: Option<Range<usize>>,
    mask_pixels: Vec<u8>,
    stride: usize,
    version: Version,
    deltas: Vec<Delta>,
    next: usize,
}

fn word(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .context("short ExHibit DLT word")?;
    Ok(u32::from_le_bytes(bytes.try_into()?))
}

fn take(data: &[u8], at: &mut usize, size: usize) -> Result<Range<usize>> {
    let end = at
        .checked_add(size)
        .context("ExHibit DLT extent overflow")?;
    ensure!(end <= data.len(), "truncated ExHibit DLT record");
    let range = *at..end;
    *at = end;
    Ok(range)
}

impl DltDecoder {
    pub fn open(encoded: Arc<[u8]>) -> Result<Self> {
        ensure!(
            (HEAD..=MAX_BYTES).contains(&encoded.len()),
            "invalid ExHibit DLT size"
        );
        let version = match word(&encoded, 0)? {
            0x0101 => Version::V101,
            0x0102 => Version::V102,
            0x0103 => Version::V103,
            _ => anyhow::bail!("unsupported ExHibit DLT version"),
        };
        let late = version == Version::V103;
        let [width, height] = [word(&encoded, 4)?, word(&encoded, 8)?];
        ensure!(
            (1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height),
            "invalid ExHibit DLT dimensions"
        );
        let count = width as usize * height as usize;
        ensure!(count <= MAX_PIXELS, "oversized ExHibit DLT frame");
        let stride = if late {
            width as usize
        } else {
            (width as usize).next_multiple_of(4)
        };
        let records = word(&encoded, 12)?;
        ensure!(
            ((2 + u32::from(late))..=MAX_RECORDS).contains(&records),
            "invalid ExHibit DLT record count"
        );
        let format = word(&encoded, 16)?;
        let masked = version == Version::V101 && matches!(format, 0x0802 | 0x0803);
        ensure!(
            if late {
                format == 0x0800
            } else {
                matches!(format, 0x0800 | 0x0801) || masked
            },
            "unsupported ExHibit DLT pixel profile {format:#x}"
        );
        let interval_ms = word(&encoded, 20)?;
        ensure!(
            (1..=60_000).contains(&interval_ms),
            "invalid ExHibit DLT frame interval"
        );
        ensure!(
            word(&encoded, 24)? == 0
                && word(&encoded, 28)?
                    == if late {
                        u32::from_le_bytes(*b"DLT\0")
                    } else {
                        0
                    },
            "unsupported ExHibit DLT header fields"
        );
        let mut at = HEAD;
        let p = take(&encoded, &mut at, 8)?;
        ensure!(
            word(&encoded, p.start)? == 0x0e02 && word(&encoded, p.start + 4)? == 1024,
            "missing ExHibit DLT palette record"
        );
        let entries = take(&encoded, &mut at, 1024)?;
        let mut palette = [[0; 4]; 256];
        for (dst, src) in palette.iter_mut().zip(encoded[entries].chunks_exact(4)) {
            *dst = [src[2], src[1], src[0], 255];
        }
        let b = take(&encoded, &mut at, 8)?;
        let base_len = word(&encoded, b.start + 4)? as usize;
        ensure!(
            if late {
                word(&encoded, b.start)? == 0x0d02 && base_len > 0
            } else {
                word(&encoded, b.start)? == if masked { 0x4f02 } else { 0x0f02 }
                    && base_len == stride * height as usize
            },
            "invalid ExHibit DLT full image record"
        );
        if masked {
            let header = take(&encoded, &mut at, 4)?;
            ensure!(
                word(&encoded, header.start)? as usize == base_len,
                "invalid ExHibit DLT mask extent"
            );
        }
        let base = take(&encoded, &mut at, base_len)?;
        let mask_base = masked
            .then(|| take(&encoded, &mut at, base_len))
            .transpose()?;
        // 0x0103 counts its terminator; 0x0101 does not.
        let physical_frames = records as usize - 1 - usize::from(late);
        let mut deltas = Vec::with_capacity(physical_frames - 1);
        for _ in 1..physical_frames {
            let flags = word(&encoded, at)?;
            if version != Version::V101 && flags == 0x0001_0004 {
                let hold = take(&encoded, &mut at, 8)?;
                ensure!(
                    word(&encoded, hold.start + 4)? == 0,
                    "invalid ExHibit DLT hold"
                );
                deltas.push(Delta::Hold);
                continue;
            }
            let header = take(&encoded, &mut at, 24)?;
            let ptr = header.start;
            ensure!(
                flags
                    == if masked {
                        0x4200
                    } else if version == Version::V101 {
                        0x0200
                    } else {
                        0x0001_0200
                    },
                "unsupported ExHibit DLT delta flags {flags:#x}"
            );
            let decode_len = word(&encoded, ptr + 4)? as usize;
            let [x, y, w, h] = [
                word(&encoded, ptr + 8)?,
                word(&encoded, ptr + 12)?,
                word(&encoded, ptr + 16)?,
                word(&encoded, ptr + 20)?,
            ];
            ensure!(
                w > 0
                    && h > 0
                    && x.checked_add(w).is_some_and(|r| r <= width)
                    && y.checked_add(h).is_some_and(|b| b <= height),
                "invalid ExHibit DLT delta rectangle"
            );
            ensure!(decode_len > 0, "empty ExHibit DLT delta");
            let mask_len = if masked {
                let header = take(&encoded, &mut at, 4)?;
                let len = word(&encoded, header.start)? as usize;
                ensure!(len > 0, "empty ExHibit DLT mask delta");
                Some(len)
            } else {
                None
            };
            let range = take(&encoded, &mut at, decode_len)?;
            let mask = mask_len
                .map(|len| take(&encoded, &mut at, len))
                .transpose()?;
            deltas.push(Delta::Paint {
                rect: [x, y, w, h],
                range,
                mask,
            });
        }
        let tail = take(&encoded, &mut at, 4)?;
        ensure!(
            word(&encoded, tail.start)? == 0x8000 && at == encoded.len(),
            "missing ExHibit DLT terminator or extra data"
        );
        let info = DltInfo {
            size: [width, height],
            frames: physical_frames,
            interval_ms,
            loop_hint: format & 1 != 0,
        };
        Ok(Self {
            encoded,
            info,
            palette,
            pixels: vec![0; count],
            base,
            mask_base,
            mask_pixels: if masked { vec![0; count] } else { Vec::new() },
            stride,
            version,
            deltas,
            next: 0,
        })
    }

    pub fn info(&self) -> DltInfo {
        self.info
    }

    /// Advance like the ordinary native looping reader. At 0x8000 it
    /// skips the palette and raw image again, preserving existing indices.
    pub fn next_looped_frame(&mut self) -> Result<Option<DltFrame>> {
        if self.next == self.info.frames && self.info.loop_hint {
            ensure!(!self.deltas.is_empty(), "empty ExHibit DLT loop");
            self.next = 1;
        }
        self.next_frame()
    }

    /// Advance one image in order. A corrupt delta never mutates the canvas.
    /// Reopening the resource is the current way to restart; timing is caller-owned.
    pub fn next_frame(&mut self) -> Result<Option<DltFrame>> {
        let frame = self.next;
        if frame >= self.info.frames {
            return Ok(None);
        }
        let [width, height] = self.info.size;
        let row = width as usize;
        let rect = if frame == 0 {
            let base = &self.encoded[self.base.clone()];
            let expanded;
            let source = if self.version == Version::V103 {
                expanded = decompress_exact_12_4(base, row * height as usize)
                    .context("ExHibit DLT compressed base")?;
                &expanded
            } else {
                base
            };
            for (destination, source) in self
                .pixels
                .chunks_exact_mut(row)
                .zip(source.chunks_exact(self.stride).rev())
            {
                destination.copy_from_slice(&source[..row]);
            }
            if let Some(range) = &self.mask_base {
                for (dst, src) in self
                    .mask_pixels
                    .chunks_exact_mut(row)
                    .zip(self.encoded[range.clone()].chunks_exact(self.stride).rev())
                {
                    dst.copy_from_slice(&src[..row]);
                }
            }
            [0, 0, width, height]
        } else if let Delta::Paint { rect, range, mask } = &self.deltas[frame - 1] {
            let [x, y, w, h] = rect.map(|n| n as usize);
            let packed = &self.encoded[range.clone()];
            let size = w * h;
            if self.version != Version::V103 {
                validate_lzss_prefix(packed, size.min(18))?;
            }
            let decoded = decompress_exact_12_4(packed, size)
                .with_context(|| format!("ExHibit DLT color frame {frame}"))?;
            // Decode both planes before mutating either retained canvas.
            let mask = mask
                .as_ref()
                .map(|range| {
                    let packed = &self.encoded[range.clone()];
                    validate_lzss_prefix(packed, size.min(18))?;
                    decompress_exact_12_4(packed, size)
                        .with_context(|| format!("ExHibit DLT mask frame {frame}"))
                })
                .transpose()?;
            for (i, update) in decoded.chunks_exact(w).enumerate() {
                let source_y = if self.version == Version::V101 {
                    i
                } else {
                    h - 1 - i
                };
                let start = (y + source_y) * row + x;
                for (pixel, new) in self.pixels[start..start + w].iter_mut().zip(update) {
                    if *new != 0 {
                        *pixel = *new;
                    }
                }
                if let Some(mask) = &mask {
                    for (pixel, &new) in self.mask_pixels[start..start + w]
                        .iter_mut()
                        .zip(&mask[i * w..(i + 1) * w])
                    {
                        if new != 0xfe {
                            *pixel = new;
                        }
                    }
                }
            }
            *rect
        } else {
            [0; 4]
        };
        self.next += 1;
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for (i, &index) in self.pixels.iter().enumerate() {
            let mut color = self.palette[index as usize];
            if let Some(mask) = self.mask_pixels.get(i) {
                color[3] = mask.saturating_mul(16);
            }
            rgba.extend_from_slice(&color);
        }
        Ok(Some(DltFrame {
            index: frame,
            rgba,
            rect,
        }))
    }
}

// The native routine clears [0, 4078) and starts writing at 4078. Before
// the first 18 outputs it cannot portably read the uninitialized tail.
fn validate_lzss_prefix(packed: &[u8], decisive: usize) -> Result<()> {
    let mut at = 0;
    let mut written = 0;
    while written < decisive {
        let flags = *packed.get(at).context("short ExHibit DLT LZSS flags")?;
        at += 1;
        for bit in 0..8 {
            if flags & (1 << bit) != 0 {
                packed.get(at).context("short ExHibit DLT LZSS literal")?;
                at += 1;
                written += 1;
            } else {
                let code = packed
                    .get(at..at + 2)
                    .context("short ExHibit DLT LZSS backreference")?;
                at += 2;
                let start = usize::from(code[0]) | (usize::from(code[1] & 0xf0) << 4);
                for i in 0..(usize::from(code[1] & 15) + 3) {
                    let source = (start + i) & 4095;
                    ensure!(
                        source < 4078 + written,
                        "undefined ExHibit DLT LZSS prefix reference"
                    );
                    written += 1;
                    if written >= decisive {
                        return Ok(());
                    }
                }
            }
            if written >= decisive {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fixture() -> Vec<u8> {
        fn words(output: &mut Vec<u8>, values: &[u32]) {
            for n in values {
                output.extend(n.to_le_bytes());
            }
        }
        let mut bytes = Vec::new();
        words(&mut bytes, &[0x0101, 3, 2, 3, 0x0801, 80, 0, 0]);
        words(&mut bytes, &[0x0e02, 1024]);
        for i in 0..=255_u8 {
            bytes.extend([i.wrapping_add(23), i.wrapping_add(9), i, 0]);
        }
        words(&mut bytes, &[0x0f02, 8]);
        bytes.extend([4, 5, 6, 255, 1, 2, 3, 255]);
        words(&mut bytes, &[0x0200, 5, 1, 0, 2, 2]);
        bytes.extend([0x0f, 0, 7, 8, 0]);
        words(&mut bytes, &[0x8000]);
        bytes
    }

    fn late_fixture() -> Vec<u8> {
        let palette = fixture()[32..1064].to_vec();
        let mut bytes: Vec<_> = [0x0103u32, 3, 2, 5, 0x0800, 33, 0, 0x0054_4c44]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        bytes.extend(palette);
        let mut record = |words: &[u32], packed: &[u8]| {
            bytes.extend(words.iter().flat_map(|w| w.to_le_bytes()));
            bytes.extend(packed);
        };
        record(&[0x0d02, 7], &[0x3f, 4, 5, 6, 1, 2, 3]);
        record(&[0x0001_0200, 5, 1, 0, 2, 2], &[0x0f, 8, 0, 0, 7]);
        record(&[0x0001_0004, 0], &[]);
        record(&[0x8000], &[]);
        bytes
    }

    #[test]
    fn late_compressed_base_bottom_up_delta_and_hold_keep_timing() -> Result<()> {
        let mut dlt = DltDecoder::open(late_fixture().into())?;
        assert_eq!(
            dlt.info(),
            DltInfo {
                size: [3, 2],
                frames: 3,
                interval_ms: 33,
                loop_hint: false
            }
        );
        let base = dlt.next_frame()?.unwrap();
        let pasted = dlt.next_frame()?.unwrap();
        let hold = dlt.next_frame()?.unwrap();
        assert_eq!([base.index, pasted.index, hold.index], [0, 1, 2]);
        assert_eq!(pasted.rect, [1, 0, 2, 2]);
        assert_eq!(hold.rect, [0, 0, 0, 0]);
        assert_eq!(hold.rgba, pasted.rgba);
        assert_eq!(
            base.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        assert_eq!(
            hold.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
            [1, 2, 7, 4, 8, 6]
        );
        assert!(dlt.next_looped_frame()?.is_none());
        Ok(())
    }

    #[test]
    fn midori_raw_padded_base_bottom_up_rect_and_repeat_skip_the_base() -> Result<()> {
        let mut bytes = late_fixture();
        bytes[0..32].copy_from_slice(
            &[0x0102u32, 3, 2, 4, 0x0801, 80, 0, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        bytes.splice(
            1064..1079,
            [0x0f02u32, 8]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .chain([4, 5, 6, 255, 1, 2, 3, 255]),
        );
        let mut dlt = DltDecoder::open(bytes.into())?;
        assert!(dlt.info().loop_hint);
        let base = dlt.next_looped_frame()?.unwrap();
        let paint = dlt.next_looped_frame()?.unwrap();
        let held = dlt.next_looped_frame()?.unwrap();
        assert_eq!([base.index, paint.index, held.index], [0, 1, 2]);
        assert_ne!(base.rgba, paint.rgba);
        assert_eq!(paint.rgba, held.rgba);
        assert_eq!(
            paint.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
            [1, 2, 7, 4, 8, 6]
        );
        let wrapped = dlt.next_looped_frame()?.unwrap();
        assert_eq!(wrapped.index, 1);
        assert_eq!(wrapped.rgba, held.rgba);
        Ok(())
    }

    #[test]
    fn late_album_layout_is_not_a_v1_alias_and_corrupt_base_cannot_advance() -> Result<()> {
        let source = late_fixture();
        for (offset, value) in [
            (0, 0x0101u32),
            (12, 4),
            (16, 0x0806),
            (28, 0),
            (1064, 0x0f02),
            (1079, 0x0200),
            (1112, 1),
        ] {
            let mut bytes = source.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(DltDecoder::open(bytes.into()).is_err(), "{offset}");
        }
        let mut corrupt_base = source;
        corrupt_base[1072] = 0;
        let mut dlt = DltDecoder::open(corrupt_base.into())?;
        assert!(dlt.next_frame().is_err());
        assert!(dlt.next_frame().is_err());
        assert_eq!(dlt.next, 0);
        assert_eq!(dlt.pixels, [0; 6]);
        Ok(())
    }

    #[test]
    fn indexed_bottom_up_base_and_zero_keep_rectangles() -> Result<()> {
        let mut dlt = DltDecoder::open(fixture().into())?;
        assert_eq!(
            dlt.info(),
            DltInfo {
                size: [3, 2],
                frames: 2,
                interval_ms: 80,
                loop_hint: true
            }
        );
        let a = dlt.next_frame()?.unwrap();
        let b = dlt.next_frame()?.unwrap();
        assert_eq!([a.index, b.index], [0, 1]);
        assert_eq!([a.rect, b.rect], [[0, 0, 3, 2], [1, 0, 2, 2]]);
        assert_eq!(
            a.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        assert_eq!(
            b.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
            [1, 2, 7, 4, 8, 6]
        );
        assert_eq!(&b.rgba[8..12], &[7, 16, 30, 255]);
        assert!(dlt.next_frame()?.is_none());
        Ok(())
    }

    #[test]
    fn looping_skips_raw_base_and_keeps_last_canvas() -> Result<()> {
        let mut bytes = fixture();
        bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
        bytes.truncate(bytes.len() - 4);
        for n in [0x0200u32, 2, 0, 0, 1, 1] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend([1, 9]);
        bytes.extend(0x8000u32.to_le_bytes());
        let mut dlt = DltDecoder::open(bytes.into())?;
        for expected in 0..3 {
            assert_eq!(dlt.next_looped_frame()?.unwrap().index, expected);
        }
        let repeated = dlt.next_looped_frame()?.unwrap();
        assert_eq!(repeated.index, 1);
        assert_eq!(repeated.rgba[0], 9);
        Ok(())
    }

    #[test]
    fn rejects_unsupported_profiles_oob_rectangles_and_broken_streams() -> Result<()> {
        let source = fixture();
        let expected_delta_start = 32 + 8 + 1024 + 8 + 8;
        for (offset, value) in [(0, 0x0102_u32), (16, 0x0803), (expected_delta_start + 8, 2)] {
            let mut bytes = source.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(DltDecoder::open(bytes.into()).is_err());
        }
        assert!(DltDecoder::open(source[..source.len() - 1].to_vec().into()).is_err());
        let mut corrupt = source;
        corrupt[expected_delta_start + 24] = 0;
        corrupt[expected_delta_start + 25] = 0xee;
        corrupt[expected_delta_start + 26] = 0xf0;
        let mut dlt = DltDecoder::open(corrupt.into())?;
        assert!(dlt.next_frame()?.is_some());
        assert!(dlt.next_frame().is_err());
        assert!(dlt.next_frame().is_err());
        assert_eq!(dlt.pixels, [1, 2, 3, 4, 5, 6]);
        let overlap = [0x01, 42, 0xee, 0xf0];
        validate_lzss_prefix(&overlap, 4)?;
        assert_eq!(decompress_exact_12_4(&overlap, 4)?, [42; 4]);
        Ok(())
    }

    #[test]
    fn v1_mask_planes_keep_only_fe_and_corruption_preserves_both_canvases() -> Result<()> {
        let mut src = fixture()[..1064].to_vec();
        src[16..20].copy_from_slice(&0x0803u32.to_le_bytes());
        src.extend([0x4f02u32, 8, 8].into_iter().flat_map(u32::to_le_bytes));
        src.extend([4, 5, 6, 0, 1, 2, 3, 0]);
        src.extend([0, 16, 1, 0, 8, 3, 15, 0]);
        src.extend(
            [0x4200u32, 5, 1, 0, 2, 2, 5]
                .into_iter()
                .flat_map(u32::to_le_bytes),
        );
        src.extend([0x0f, 0, 7, 8, 0]);
        let mask_start = src.len();
        src.extend([0x0f, 0, 0xfe, 0xfe, 16]);
        src.extend(0x8000u32.to_le_bytes());
        let mut decoder = DltDecoder::open(src.clone().into())?;
        let lanes = |frame: DltFrame, lane: usize| {
            frame
                .rgba
                .chunks_exact(4)
                .map(|p| p[lane])
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lanes(decoder.next_frame()?.unwrap(), 3),
            [128, 48, 240, 0, 255, 16]
        );
        assert_eq!(
            lanes(decoder.next_frame()?.unwrap(), 3),
            [128, 0, 240, 0, 255, 255]
        );
        assert_eq!(
            lanes(decoder.next_looped_frame()?.unwrap(), 0),
            [1, 2, 7, 4, 8, 6]
        );

        src[mask_start..mask_start + 3].copy_from_slice(&[0, 0xee, 0xf0]);
        let mut bad = DltDecoder::open(src.into())?;
        bad.next_frame()?;
        assert!(bad.next_frame().is_err());
        assert_eq!(bad.pixels, [1, 2, 3, 4, 5, 6]);
        assert_eq!(bad.mask_pixels, [8, 3, 15, 0, 16, 1]);
        assert_eq!(bad.next, 1);
        Ok(())
    }

    #[test]
    #[ignore = "requires the user's installed Famisupi 1180.dlt; no game run"]
    fn official_masked_clip_matches_independent_rgba_hash() -> Result<()> {
        let path = std::env::var_os("CRYSIRROR_EXHIBIT_DLT_MASKED")
            .context("set CRYSIRROR_EXHIBIT_DLT_MASKED to the local stock 1180.dlt")?;
        let bytes = std::fs::read(path)?;
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            "fe17d25044ef8851f6487b31f516844e49ae4c2cd369ed6c2c255afb4a9ba83e"
        );
        let mut decoder = DltDecoder::open(bytes.into())?;
        assert_eq!(
            decoder.info(),
            DltInfo {
                size: [400, 400],
                frames: 25,
                interval_ms: 80,
                loop_hint: true
            }
        );
        let mut digest = Sha256::new();
        while let Some(frame) = decoder.next_frame()? {
            digest.update(frame.rgba);
        }
        assert_eq!(
            format!("{:x}", digest.finalize()),
            "0d6aea2ba35be3f915b3dbd343162a2a17ca976ead05fa789df2c8b22e7dd52b"
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires a user-supplied stock ordinary DLT 1239 file; no game run"]
    fn official_opening_asset_matches_independent_pixel_hashes() -> Result<()> {
        let path = std::env::var_os("CRYSIRROR_EXHIBIT_DLT_FIXTURE")
            .context("set CRYSIRROR_EXHIBIT_DLT_FIXTURE to the local stock 1239.dlt")?;
        let bytes = std::fs::read(path)?;
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            "3f92bcd7e21c9169dd43175d119792e232526393341e7b6a12eb006a7e75a72c"
        );
        let mut dlt = DltDecoder::open(bytes.into())?;
        assert_eq!(
            dlt.info(),
            DltInfo {
                size: [800, 600],
                frames: 49,
                interval_ms: 80,
                loop_hint: true
            }
        );
        let mut all = Sha256::new();
        let mut count = 0;
        while let Some(frame) = dlt.next_frame()? {
            assert_eq!(frame.index, count);
            all.update(frame.rgba);
            count += 1;
        }
        assert_eq!(count, 49);
        assert_eq!(
            format!("{:x}", all.finalize()),
            "4e8890840c634b958bfa0c87bcc32710c7f2776eb00921142d1c8a4ecf917462"
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires the user's installed ordinary Stray Sheep v3 clips; no game run"]
    fn stray_sheep_ordinary_clips_match_independent_pixel_hashes() -> Result<()> {
        for (var, interval_ms, frames, input_hash, frame_hash) in [
            (
                "CRYSIRROR_EXHIBIT_DLT_STRAY_VOL1",
                33,
                64,
                "a5b018e808546b4a9ae839509f1598dedda454a2828057853193b318ab4fa036",
                "ad095683af5cd9b8efdf52b4f1c1488a400dc5e7a3055aad49f02e1385e879e7",
            ),
            (
                "CRYSIRROR_EXHIBIT_DLT_STRAY_VOL2",
                80,
                8,
                "f1402da73b348eca95ac9f59b8d5abd2f958c43a9aeca3fd83ed32366b70911f",
                "9bc9aa7b61befbff823ccac98f4be993dee06918d18ec8527c42d8fa630f1145",
            ),
        ] {
            let path = std::env::var_os(var).with_context(|| format!("set {var}"))?;
            let bytes = std::fs::read(path)?;
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), input_hash);
            let mut dlt = DltDecoder::open(bytes.into())?;
            assert_eq!(
                dlt.info(),
                DltInfo {
                    size: [1920, 1080],
                    frames,
                    interval_ms,
                    loop_hint: false
                }
            );
            let mut all = Sha256::new();
            let mut count = 0;
            let mut held = 0;
            while let Some(frame) = dlt.next_frame()? {
                assert_eq!(frame.index, count);
                held += usize::from(frame.rect == [0; 4]);
                all.update(frame.rgba);
                count += 1;
            }
            assert_eq!(count, frames);
            assert_eq!(held, if frames == 8 { 2 } else { 0 });
            assert_eq!(format!("{:x}", all.finalize()), frame_hash);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires the user's installed ordinary Midori v3 clip; no game run"]
    fn midori_ordinary_clip_matches_independent_pixel_hash() -> Result<()> {
        let path = std::env::var_os("CRYSIRROR_EXHIBIT_DLT_MIDORI")
            .context("set CRYSIRROR_EXHIBIT_DLT_MIDORI")?;
        let bytes = std::fs::read(path)?;
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            "085d3c1cd331c991c5de2f0f51439fd0b1bf7238a74d8fc94f808f422c8de2d1"
        );
        let mut dlt = DltDecoder::open(bytes.into())?;
        assert_eq!(
            dlt.info(),
            DltInfo {
                size: [1024, 576],
                frames: 49,
                interval_ms: 80,
                loop_hint: true
            }
        );
        let mut all = Sha256::new();
        for index in 0..49 {
            let frame = dlt.next_frame()?.context("missing Midori frame")?;
            assert_eq!(frame.index, index);
            assert_eq!(frame.rect == [0; 4], index == 48);
            all.update(frame.rgba);
        }
        assert!(dlt.next_frame()?.is_none());
        assert_eq!(
            format!("{:x}", all.finalize()),
            "96b785cb83fa56afb136d3c192d2c7d2876a528c8bf3020dc2d927c69bbeb58a"
        );
        Ok(())
    }
}
