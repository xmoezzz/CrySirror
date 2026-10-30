//! Bounded, device-free Circus CRXG raster decoding.
//!
//! Format reference: GARbro b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0,
//! ArcFormats/Circus/ImageCRX.cs, Copyright (C) 2015-2016 by morkt.
//! Its MIT permission notice is preserved in crx.LICENSE.
use crate::{rgba_len, ImageSurface};
use anyhow::{bail, ensure, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrxImage {
    pub offset: [i16; 2],
    pub surface: ImageSurface,
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).context("CRX input overflow")?;
        let value = self.bytes.get(self.pos..end).context("truncated CRX")?;
        self.pos = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
}

/// Decode a top-left straight-RGBA surface, retaining native origin metadata.
/// All allocation/decompression is bounded by the engine's image limits.
pub fn decode(bytes: &[u8]) -> Result<CrxImage> {
    ensure!(
        bytes.len() <= crate::MAX_DECODED_BYTES,
        "CRX encoded byte limit"
    );
    let mut input = Reader { bytes, pos: 0 };
    ensure!(input.take(4)? == b"CRXG", "CRX signature");
    let offset = [input.u16()? as i16, input.u16()? as i16];
    let width = u32::from(input.u16()?);
    let height = u32::from(input.u16()?);
    let length = rgba_len(width, height)?;
    let compression = input.u16()?;
    ensure!(
        (1..=3).contains(&compression),
        "unsupported CRX compression {compression}"
    );
    let flags = input.u16()?;
    let colors = input.u16()?;
    let mode = input.u16()?;
    ensure!(mode <= 2, "unsupported CRX pixel mode {mode}");
    let pixel = match colors {
        0 => 3,
        1 => 4,
        2..=258 => 1,
        _ => bail!("CRX palette size"),
    };
    let row = width as usize * pixel;
    let stride = (row + 3) & !3;
    let raw_len = stride * height as usize;
    ensure!(raw_len <= crate::MAX_DECODED_BYTES, "CRX raster limit");
    let palette = if pixel == 1 {
        let count = usize::from(colors.min(256));
        let step = if colors == 258 { 4 } else { 3 };
        let data = input.take(count * step)?;
        data.chunks_exact(step)
            .map(|p| {
                let g = if p[0] == 255 && p[1] == 0 && p[2] == 255 {
                    255
                } else {
                    p[1]
                };
                [p[0], g, p[2], 255]
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if compression == 3 {
        let count = input.u32()? as usize;
        input.take(count.checked_mul(16).context("CRX block table overflow")?)?;
    }
    if flags & 0x10 != 0 {
        let packed = input.u32()? as usize;
        ensure!(
            packed <= bytes.len() - input.pos,
            "truncated CRX packed payload"
        );
    }
    let mut raster = vec![0u8; raw_len];
    if compression == 1 {
        let mut window = vec![0u8; 65536];
        let mut win = 0usize;
        let mut dst = 0usize;
        let mut flag = 0u32;
        while dst < raw_len {
            flag >>= 1;
            if flag & 0x100 == 0 {
                flag = u32::from(input.byte()?) | 0xff00;
            }
            let (count, distance) = if flag & 1 != 0 {
                let b = input.byte()?;
                raster[dst] = b;
                window[win] = b;
                dst += 1;
                win = (win + 1) & 65535;
                continue;
            } else {
                let c = input.byte()?;
                if c >= 0xc0 {
                    (
                        4 + usize::from((c >> 2) & 15),
                        (usize::from(c & 3) << 8) | usize::from(input.byte()?),
                    )
                } else if c & 0x80 != 0 {
                    let d = usize::from(c & 31);
                    (
                        2 + usize::from((c >> 5) & 3),
                        if d == 0 {
                            usize::from(input.byte()?)
                        } else {
                            d
                        },
                    )
                } else if c == 0x7f {
                    (2 + usize::from(input.u16()?), usize::from(input.u16()?))
                } else {
                    (usize::from(c) + 4, usize::from(input.u16()?))
                }
            };
            let mut src = win.wrapping_sub(distance) & 65535;
            for _ in 0..count.min(raw_len - dst) {
                let b = window[src];
                raster[dst] = b;
                window[win] = b;
                src = (src + 1) & 65535;
                win = (win + 1) & 65535;
                dst += 1;
            }
        }
    } else {
        // Every filtered pixel consumes at most two bytes (planar RLE), plus
        // one row control. Inflation must never allocate an unbounded stream.
        let bound = (row * 2 + 1)
            .checked_mul(height as usize)
            .context("CRX inflate limit overflow")?;
        let decoded =
            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&bytes[input.pos..], bound)
                .map_err(|e| anyhow::anyhow!("CRX zlib: {e:?}"))?;
        let mut src = Reader {
            bytes: &decoded,
            pos: 0,
        };
        for y in 0..height as usize {
            let start = y * stride;
            if pixel == 1 {
                raster[start..start + row].copy_from_slice(src.take(row)?);
                continue;
            }
            let control = src.byte()?;
            ensure!(control <= 4, "unsupported CRX row filter {control}");
            ensure!(
                y > 0 || matches!(control, 0 | 4),
                "CRX row filter references absent previous row"
            );
            match control {
                0 | 2 => {
                    raster[start..start + pixel].copy_from_slice(src.take(pixel)?);
                    for x in pixel..row {
                        let previous = if control == 0 {
                            start + x - pixel
                        } else {
                            start - stride + x - pixel
                        };
                        raster[start + x] = src.byte()?.wrapping_add(raster[previous]);
                    }
                }
                1 => {
                    for x in 0..row {
                        raster[start + x] = src.byte()?.wrapping_add(raster[start - stride + x]);
                    }
                }
                3 => {
                    for x in 0..row - pixel {
                        raster[start + x] =
                            src.byte()?.wrapping_add(raster[start - stride + x + pixel]);
                    }
                    raster[start + row - pixel..start + row].copy_from_slice(src.take(pixel)?);
                }
                4 => {
                    for channel in 0..pixel {
                        let mut x = 0;
                        let mut value = src.byte()?;
                        loop {
                            raster[start + x * pixel + channel] = value;
                            x += 1;
                            if x == width as usize {
                                break;
                            }
                            let next = src.byte()?;
                            if next == value {
                                let count = usize::from(src.byte()?);
                                ensure!(count <= width as usize - x, "CRX planar run exceeds row");
                                for _ in 0..count {
                                    raster[start + x * pixel + channel] = value;
                                    x += 1;
                                }
                                if x == width as usize {
                                    break;
                                }
                                value = src.byte()?;
                            } else {
                                value = next;
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        ensure!(
            src.pos == decoded.len(),
            "trailing CRX filtered raster bytes"
        );
    }
    let mut rgba = vec![0u8; length];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let p = &raster[y * stride + x * pixel..][..pixel];
            let value = match pixel {
                1 => *palette
                    .get(usize::from(p[0]))
                    .context("CRX palette index")?,
                3 => [p[2], p[1], p[0], 255],
                4 if mode == 1 => [p[2], p[1], p[0], p[3]],
                4 => [p[3], p[2], p[1], p[0] ^ if mode == 2 { 0 } else { 255 }],
                _ => unreachable!(),
            };
            rgba[(y * width as usize + x) * 4..][..4].copy_from_slice(&value);
        }
    }
    Ok(CrxImage {
        offset,
        surface: ImageSurface {
            width,
            height,
            rgba,
        },
    })
}

pub(crate) struct Format;
impl crate::ImageFormat for Format {
    fn id(&self) -> &'static str {
        "circus/crx"
    }
    fn recognize(&self, _: &str, bytes: &[u8]) -> bool {
        bytes.starts_with(b"CRXG")
    }
    fn decode(&self, bytes: &[u8], _: &crate::ImageDecodeContext<'_>) -> Result<ImageSurface> {
        Ok(decode(bytes)?.surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image(width: u16, height: u16, colors: u16, mode: u16, raw: &[u8]) -> Vec<u8> {
        let mut data = b"CRXG".to_vec();
        for n in [(-3i16) as u16, 7, width, height, 2, 0, colors, mode] {
            data.extend(n.to_le_bytes());
        }
        data.extend(miniz_oxide::deflate::compress_to_vec_zlib(raw, 6));
        data
    }
    #[test]
    fn filtered_rows_and_native_origin_are_engine_resources() {
        // BGR rows: [1,2,3],[4,5,6]; [7,8,9],[10,11,12].
        for filter in 0..=4 {
            let mut raw = vec![0, 1, 2, 3, 3, 3, 3];
            raw.extend(match filter {
                0 => vec![0, 7, 8, 9, 3, 3, 3],
                1 => vec![1, 6, 6, 6, 6, 6, 6],
                2 => vec![2, 7, 8, 9, 9, 9, 9],
                3 => vec![3, 3, 3, 3, 10, 11, 12],
                4 => vec![4, 7, 10, 8, 11, 9, 12],
                _ => unreachable!(),
            });
            let bytes = image(2, 2, 0, 0, &raw);
            let got = decode(&bytes).unwrap();
            assert_eq!(got.offset, [-3, 7]);
            assert_eq!(
                got.surface.rgba,
                [3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255],
                "filter {filter}"
            );
            assert_eq!(
                crate::DecoderRegistry::default()
                    .decode("renamed.data", &bytes)
                    .unwrap(),
                got.surface
            );
        }
    }
    #[test]
    fn alpha_layouts_palette_and_planar_runs() {
        let mut literal = image(1, 1, 0, 0, &[0, 30, 20, 10]);
        literal[12..14].copy_from_slice(&1u16.to_le_bytes());
        literal.truncate(20);
        literal.extend([15, 30, 20, 10, 0]);
        assert_eq!(decode(&literal).unwrap().surface.rgba, [10, 20, 30, 255]);
        let mut blocks = image(1, 1, 0, 0, &[0, 30, 20, 10]);
        blocks[12..14].copy_from_slice(&3u16.to_le_bytes());
        blocks.splice(20..20, 0u32.to_le_bytes());
        assert_eq!(decode(&blocks).unwrap().surface.rgba, [10, 20, 30, 255]);
        for (mode, pixel) in [
            (0, [200, 30, 20, 10]),
            (1, [30, 20, 10, 55]),
            (2, [55, 30, 20, 10]),
        ] {
            let mut raw = vec![0];
            raw.extend(pixel);
            assert_eq!(
                decode(&image(1, 1, 1, mode, &raw)).unwrap().surface.rgba,
                [10, 20, 30, 55]
            );
        }
        let mut data = image(1, 1, 2, 0, &[1]);
        data.splice(20..20, [255, 0, 255, 10, 20, 30]);
        assert_eq!(decode(&data).unwrap().surface.rgba, [10, 20, 30, 255]);
        // Each channel explicitly writes 2 followed by run marker 2/count 3.
        assert_eq!(
            decode(&image(4, 1, 0, 0, &[4, 2, 2, 3, 2, 2, 3, 2, 2, 3]))
                .unwrap()
                .surface
                .rgba,
            [2, 2, 2, 255].repeat(4)
        );
    }
    #[test]
    fn malformed_streams_and_decompression_bombs_fail() {
        assert!(decode(&image(1, 1, 0, 0, &[1, 0, 0, 0])).is_err());
        assert!(decode(&image(1, 1, 0, 0, &[5])).is_err());
        assert!(decode(&image(2, 1, 0, 0, &[4, 2, 2, 9])).is_err());
        assert!(decode(&image(1, 1, 0, 0, &[0; 10000])).is_err());
        let mut bytes = image(1, 1, 0, 0, &[0, 1, 2, 3]);
        bytes.truncate(22);
        assert!(decode(&bytes).is_err());
    }
    #[test]
    #[ignore = "requires owner-provided local Circus game; no assets distributed"]
    fn installed_crx_corpus() {
        fn visit(path: &std::path::Path, count: &mut usize) {
            for item in std::fs::read_dir(path).unwrap() {
                let p = item.unwrap().path();
                if p.is_dir() && p.file_name().unwrap() != ".DAV" {
                    visit(&p, count);
                } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("crx")) {
                    let b = std::fs::read(&p).unwrap();
                    if b.starts_with(b"CRXG") {
                        decode(&b).unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
                        *count += 1;
                    }
                }
            }
        }
        let root = std::env::var_os("CRYSIRROR_CIRCUS_GRP").expect("CRYSIRROR_CIRCUS_GRP");
        let mut count = 0;
        visit(std::path::Path::new(&root), &mut count);
        assert!(count > 0);
        eprintln!("decoded {count} installed CRXG images");
    }
}
