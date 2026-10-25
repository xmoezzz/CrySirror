// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU image ports from arc_unpacker 456834ecf2e5686813802c37efd829310485c57d.
//! JPEG/PNG/WebP entropy codecs use CrySirror's bounded standard decoders.
mod bitmap;
mod dds;
mod extended;
mod util;
use anyhow::{ensure, Context, Result};
use crysirror_image::{
    decode_standard, rgba_len, ImageDecodeContext, ImageSurface, MAX_DECODED_BYTES,
};
use util::*;
pub const FORMATS: &[&str] = &[
    "crowd/cwd",
    "crowd/cwl",
    "crowd/cwp",
    "crowd/zbm",
    "almond-collective/teyl",
    "malie/mgf",
    "minato-soft/fil",
    "nekopack/masked-bmp",
    "riddle-soft/cmp",
    "rpgmaker/xyz",
    "scene-player/pmp",
    "sysadv/pga",
    "west-vision/syg",
    "yuris/ycg",
    "yumemiru/epf",
    "gsd/bmz",
    "kiss/custom-png",
    "google/webp",
    "jpeg/jpeg",
    "png/png",
    "microsoft/bmp",
    "microsoft/dds",
    "truevision/tga",
    "active-soft/ed8",
    "active-soft/edt",
    "eagls/gr",
    "fc01/acd",
    "fc01/mcg",
    "french-bread/ex3",
    "ism/isg",
    "ivory/prs",
    "mebius/mcg",
    "pajamas/ep",
    "silky/akb",
    "triangle/yb",
    "twilight-frontier/pak2-gfx",
    "twilight-frontier/tfbm",
];
fn ext(path: &str, e: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, x)| x.eq_ignore_ascii_case(e))
}
pub fn probe(id: &str, path: &str, b: &[u8]) -> bool {
    match id {
        "active-soft/ed8" => b.starts_with(b".8Bit\x8d]\x8c\xcb\0"),
        "active-soft/edt" => b.starts_with(b".TRUE\x8d]\x8c\xcb\0"),
        "eagls/gr" => ext(path, "gr"),
        "fc01/acd" => b.starts_with(b"ACD 1.00"),
        "fc01/mcg" => b.starts_with(b"MCG "),
        "french-bread/ex3" => b.starts_with(b"LLIF"),
        "ism/isg" => b.starts_with(b"ISM IMAGEFILE\0"),
        "ivory/prs" => b.starts_with(b"YB") && b.get(3) == Some(&3),
        "triangle/yb" => b.starts_with(b"YB") && b.get(3) == Some(&4),
        "mebius/mcg" => b.starts_with(b"MCG") && b.get(3).is_some_and(|&v| v < 8),
        "pajamas/ep" => b.starts_with(b"EP"),
        "silky/akb" => b.starts_with(b"AKB ") || b.starts_with(b"AKB+"),
        "twilight-frontier/pak2-gfx" => ext(path, "cv2"),
        "twilight-frontier/tfbm" => b.starts_with(b"TFBM\0"),
        "crowd/cwd" => b.starts_with(b"cwd format  - version 1.00 -"),
        "crowd/cwl" => b.starts_with(b"SZDD") && ext(path, "cwl"),
        "crowd/zbm" => b.starts_with(b"SZDD") && ext(path, "zbm"),
        "crowd/cwp" => b.starts_with(b"CWDP"),
        "almond-collective/teyl" => b.starts_with(b"TEYL"),
        "malie/mgf" => b.starts_with(b"MalieGF\0"),
        "minato-soft/fil" => {
            b.len() >= 8
                && u32le(b, 0)
                    .ok()
                    .zip(u32le(b, 4).ok())
                    .is_some_and(|(w, h)| {
                        w > 0 && h > 0 && 8 + u64::from(w) * u64::from(h) == b.len() as u64
                    })
        }
        "nekopack/masked-bmp" => ext(path, "alp"),
        "scene-player/pmp" => ext(path, "pmp"),
        "truevision/tga" => ext(path, "tga"),
        "riddle-soft/cmp" => b.starts_with(b"CMP1"),
        "rpgmaker/xyz" => b.starts_with(b"XYZ1"),
        "sysadv/pga" => b.starts_with(b"PGAPGAH\x0a"),
        "west-vision/syg" => b.starts_with(b"$SYG"),
        "yuris/ycg" => b.starts_with(b"YCG\0"),
        "yumemiru/epf" => b.starts_with(b"yga\0"),
        "gsd/bmz" => b.starts_with(b"ZLC3"),
        "kiss/custom-png" => {
            b.starts_with(b"\x89PNG") && png_chunk(b, b"xPAL").ok().flatten().is_some()
        }
        "png/png" => b.starts_with(b"\x89PNG"),
        "jpeg/jpeg" => b.starts_with(b"\xff\xd8\xff"),
        "google/webp" => b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP"),
        "microsoft/bmp" => b.starts_with(b"BM") && b.get(6..10) == Some(&[0, 0, 0, 0]),
        "microsoft/dds" => b.starts_with(b"DDS "),
        _ => false,
    }
}
pub fn decode(id: &str, b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    ensure!(b.len() <= MAX_DECODED_BYTES, "encoded image exceeds 64 MiB");
    // The path-only formats deliberately still validate their complete payload below.
    ensure!(probe(id, ctx.path, b), "image does not match {id}");
    let image = match id {
        "png/png" => decode_png(b)?,
        "jpeg/jpeg" | "google/webp" => decode_standard(b)?,
        "microsoft/bmp" => bitmap::bmp(b)?,
        "microsoft/dds" => dds::decode(b)?,
        "truevision/tga" => bitmap::tga(b)?,
        "crowd/cwd" => cwd(b)?,
        "crowd/cwl" | "crowd/zbm" => {
            let size = u32le(b, 10)? as usize;
            let mut data = lzss(
                slice(b, 14, b.len().saturating_sub(14))?,
                size,
                0xff0,
                false,
            )?;
            if id == "crowd/cwl" {
                cwd(&data)?
            } else {
                for v in data.iter_mut().take(100) {
                    *v ^= 255;
                }
                bitmap::bmp(&data)?
            }
        }
        "almond-collective/teyl" => raw(
            u32le(b, 4)?,
            u32le(b, 8)?,
            slice(b, 12, b.len().saturating_sub(12))?,
            4,
        )?,
        "west-vision/syg" => raw(
            u32le(b, 16)?,
            u32le(b, 20)?,
            slice(b, 32, b.len().saturating_sub(32))?,
            3,
        )?,
        "minato-soft/fil" => raw(u32le(b, 0)?, u32le(b, 4)?, slice(b, 8, b.len() - 8)?, 1)?,
        "malie/mgf" => {
            let mut png = b.to_vec();
            png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
            decode_png(&png)?
        }
        "sysadv/pga" => {
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            png.extend(slice(b, 11, b.len().saturating_sub(11))?);
            decode_png(&png)?
        }
        "crowd/cwp" => {
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            png.extend(slice(b, 4, 17)?);
            png.extend(slice(b, 21, 4)?);
            png.extend(b"IDAT");
            png.extend(slice(b, 25, b.len().checked_sub(26).context("short CWP")?)?);
            png.extend(b"\0\0\0\0IEND\xaeB`\x82");
            let mut image = decode_png(&png)?;
            for p in image.rgba.chunks_exact_mut(4) {
                p.swap(0, 2);
                p[3] = 255;
            }
            image
        }
        "gsd/bmz" => {
            let data = inflate(slice(b, 8, b.len().saturating_sub(8))?)?;
            ensure!(data.len() == u32le(b, 4)? as usize, "BMZ size mismatch");
            bitmap::bmp(&data)?
        }
        "scene-player/pmp" => {
            let data: Vec<u8> = b.iter().map(|v| v ^ 0x21).collect();
            bitmap::bmp(&inflate(&data)?)?
        }
        "riddle-soft/cmp" => {
            let data = lzss(
                slice(b, 12, u32le(b, 8)? as usize)?,
                u32le(b, 4)? as usize,
                2031,
                true,
            )?;
            bitmap::bmp(&data)?
        }
        "yumemiru/epf" => {
            ensure!(u32le(b, 12)? == 1, "unsupported EPF version");
            let w = u32le(b, 4)?;
            let h = u32le(b, 8)?;
            let size = rgba_len(w, h)?;
            ensure!(u32le(b, 16)? as usize == size, "EPF size mismatch");
            raw(
                w,
                h,
                &lzss(slice(b, 24, u32le(b, 20)? as usize)?, size, 0xfee, false)?,
                4,
            )?
        }
        "rpgmaker/xyz" => {
            let w = u32::from(u16le(b, 4)?);
            let h = u32::from(u16le(b, 6)?);
            let size = rgba_len(w, h)? / 4;
            let data = inflate(slice(b, 8, b.len().saturating_sub(8))?)?;
            ensure!(data.len() == 768 + size, "XYZ size mismatch");
            let mut rgba = Vec::with_capacity(size * 4);
            for &i in &data[768..] {
                let p = usize::from(i) * 3;
                rgba.extend([data[p], data[p + 1], data[p + 2], 255]);
            }
            ImageSurface {
                width: w,
                height: h,
                rgba,
            }
        }
        "yuris/ycg" => {
            let w = u32le(b, 4)?;
            let h = u32le(b, 8)?;
            ensure!(u32le(b, 12)? == 32, "unsupported YCG depth");
            ensure!(u32le(b, 16)? == 1, "unsupported YCG compression");
            let expected = rgba_len(w, h)?;
            let n1 = u32le(b, 36)? as usize;
            let n2 = u32le(b, 52)? as usize;
            let mut data = inflate(slice(b, 56, n1)?)?;
            ensure!(
                data.len() == u32le(b, 32)? as usize,
                "YCG first size mismatch"
            );
            let second = inflate(slice(b, 56 + n1, n2)?)?;
            ensure!(
                second.len() == u32le(b, 48)? as usize,
                "YCG second size mismatch"
            );
            ensure!(
                data.len().checked_add(second.len()) == Some(expected),
                "YCG pixel size mismatch"
            );
            data.extend(second);
            raw(w, h, &data, 4)?
        }
        "nekopack/masked-bmp" => {
            let file = ctx.path.rsplit('/').next().unwrap_or(ctx.path);
            let stem = file.rsplit_once('.').context("missing mask extension")?.0;
            let mut image = ctx.decode_relative(&format!("{stem}.bmp"))?;
            ensure!(b.len() == image.rgba.len() / 4, "mask size mismatch");
            for (p, a) in image.rgba.chunks_exact_mut(4).zip(b) {
                p[3] = *a;
            }
            image
        }
        "kiss/custom-png" => {
            let pal = png_chunk(b, b"xPAL")?.context("missing xPAL")?;
            ensure!(pal.len() == 1024, "invalid xPAL palette");
            let mut image = decode_png(b)?;
            for p in image.rgba.chunks_exact_mut(4) {
                let i = usize::from(p[0]) * 4;
                p[..3].copy_from_slice(&[pal[i + 2], pal[i + 1], pal[i]]);
            }
            image
        }
        _ => extended::decode(id, b, ctx)?,
    };
    image.validate()?;
    Ok(image)
}
fn cwd(b: &[u8]) -> Result<ImageSurface> {
    ensure!(
        b.starts_with(b"cwd format  - version 1.00 -"),
        "invalid CWD signature"
    );
    let key = u32::from(*b.get(52).context("short CWD")?) + 0x259a;
    let w = u32le(b, 44)?.wrapping_add(key);
    let h = u32le(b, 48)?.wrapping_add(key);
    raw(w, h, slice(b, 56, b.len().saturating_sub(56))?, 2)
}
fn png_chunk<'a>(b: &'a [u8], name: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    let mut p = 8;
    let mut found = None;
    while p < b.len() {
        let n = u32be(b, p)? as usize;
        let kind = slice(b, p + 4, 4)?;
        let data = slice(b, p + 8, n)?;
        slice(b, p + 8 + n, 4)?;
        if kind == name {
            found = Some(data);
        }
        p = p.checked_add(12 + n).context("PNG chunk overflow")?;
        if kind == b"IEND" {
            break;
        }
    }
    Ok(found)
}

// Preserve libpng PNG_TRANSFORM_STRIP_16. image's generic to_rgba8 instead
// rescales/rounds all 16 bits; the original decoder discards the low byte.
fn decode_png(bytes: &[u8]) -> Result<ImageSurface> {
    if bytes.get(24) != Some(&16) {
        return decode_standard(bytes);
    }
    use image::ImageDecoder;
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some((2 * MAX_DECODED_BYTES) as u64);
    let decoder = image::codecs::png::PngDecoder::with_limits(std::io::Cursor::new(bytes), limits)?;
    let (w, h) = decoder.dimensions();
    let len = rgba_len(w, h)?;
    let channels = match decoder.color_type() {
        image::ColorType::L16 => 1,
        image::ColorType::La16 => 2,
        image::ColorType::Rgb16 => 3,
        image::ColorType::Rgba16 => 4,
        _ => anyhow::bail!("unexpected PNG16 output layout"),
    };
    let size = usize::try_from(decoder.total_bytes())?;
    ensure!(
        size == len / 4 * channels * 2,
        "PNG16 decoder length mismatch"
    );
    let mut raw = vec![0; size];
    decoder.read_image(&mut raw)?;
    let mut rgba = Vec::with_capacity(len);
    for pixel in raw.chunks_exact(channels * 2) {
        let mut c = [0; 4];
        for (i, sample) in pixel.chunks_exact(2).enumerate() {
            c[i] = (u16::from_ne_bytes([sample[0], sample[1]]) >> 8) as u8;
        }
        rgba.extend(match channels {
            1 => [c[0], c[0], c[0], 255],
            2 => [c[0], c[0], c[0], c[1]],
            3 => [c[0], c[1], c[2], 255],
            _ => c,
        });
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
