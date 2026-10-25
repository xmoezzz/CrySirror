// SPDX-License-Identifier: GPL-3.0-or-later
// Additional bounded layout/compression ports from the same pinned source.
use super::{bitmap, util::*};
use anyhow::{bail, ensure, Context, Result};
use crysirror_image::{rgba_len, ImageDecodeContext, ImageSurface};
mod active;
mod fc01;
pub fn decode(id: &str, b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    match id {
        "active-soft/ed8" => active::ed8(b),
        "active-soft/edt" => active::edt(b),
        "fc01/acd" => fc01::acd(b),
        "fc01/mcg" => fc01::mcg(b, ctx),
        "eagls/gr" => gr(b),
        "french-bread/ex3" => ex3(b),
        "ism/isg" => isg(b),
        "ivory/prs" => yb(b, false),
        "triangle/yb" => yb(b, true),
        "mebius/mcg" => mcg(b),
        "pajamas/ep" => ep(b),
        "silky/akb" => akb(b, ctx),
        "twilight-frontier/pak2-gfx" => twilight(b, ctx, false),
        "twilight-frontier/tfbm" => twilight(b, ctx, true),
        _ => bail!("unsupported extended format"),
    }
}
fn gr(b: &[u8]) -> Result<ImageSurface> {
    let (&seed, data) = b.split_last().context("empty GR")?;
    let mut data = data.to_vec();
    let mut state = i64::from(u32::from(seed) ^ 0x75bd924);
    for v in data.iter_mut().take(0x174b) {
        let hi = state / 44488;
        let lo = state % 44488;
        state = 48271 * lo - 3399 * hi;
        if state < 0 {
            state += 2147483647;
        }
        let idx = (state as f64 * 4.656612875245797e-10 * 256.) as usize;
        *v ^= b"EAGLS_SYSTEM"[idx % 12];
    }
    let header = super::util::lzss(&data, 30, 0xfee, false)?;
    let offset = u32le(&header, 10)? as usize;
    let w = u32le(&header, 18)? as usize;
    let h = u32le(&header, 22)? as usize;
    let depth = usize::from(u16le(&header, 28)?);
    let size = offset
        .checked_add(
            w.div_ceil(4)
                .checked_mul(4)
                .and_then(|v| v.checked_mul(h))
                .and_then(|v| v.checked_mul(depth / 8))
                .context("GR dimensions overflow")?,
        )
        .context("GR size overflow")?;
    bitmap::bmp(&lzss(&data, size, 0xfee, false)?)
}
fn ex3(b: &[u8]) -> Result<ImageSurface> {
    let mut r = Reader::new(b, 4);
    let mut stack = r.take(64)?.to_vec();
    let mut out = Vec::new();
    let mut c = r.byte()?;
    while r.p < b.len() {
        let mut first: [u8; 256] = std::array::from_fn(|i| i as u8);
        let mut second = [0u8; 256];
        let mut offset = 0usize;
        loop {
            if c > 127 {
                offset += usize::from(c) - 127;
                c = 0;
            }
            ensure!(offset <= 256, "EX3 dictionary skip out of range");
            if offset == 256 {
                break;
            }
            for _ in 0..=c {
                ensure!(offset < 256, "EX3 dictionary out of range");
                first[offset] = r.byte()?;
                if first[offset] as usize != offset {
                    second[offset] = r.byte()?;
                }
                offset += 1;
            }
            if offset == 256 {
                break;
            }
            c = r.byte()?;
        }
        let mut left = usize::from(r.be16()?);
        let mut sp = 0;
        loop {
            let c = if sp > 0 {
                sp -= 1;
                ensure!(sp < 60, "EX3 dictionary stack overflow");
                stack[sp]
            } else {
                if left == 0 {
                    break;
                }
                left -= 1;
                r.byte()?
            };
            if first[c as usize] == c {
                out.push(c);
                limit(out.len())?;
            } else {
                ensure!(sp < 58, "EX3 dictionary cycle/stack overflow");
                stack[sp] = second[c as usize];
                stack[sp + 1] = first[c as usize];
                sp += 2;
            }
        }
        if r.p < b.len() {
            c = r.byte()?;
        }
    }
    bitmap::bmp(&out)
}
fn isg(b: &[u8]) -> Result<ImageSurface> {
    let version = *b.get(16).context("short ISG")?;
    let w = u32::from(u16le(b, 29)?);
    let h = u32::from(u16le(b, 31)?);
    let n = rgba_len(w, h)? / 4;
    let count = *b.get(35).context("short ISG palette")?;
    let count = if count == 0 { 256 } else { usize::from(count) };
    let mut r = Reader::new(b, 48);
    let mut palette = Vec::with_capacity(count);
    for _ in 0..count {
        let v = r.take(4)?;
        palette.push([v[2], v[1], v[0], 255]);
    }
    let mut out = Vec::with_capacity(n);
    let mut flags = 0u16;
    let mut dict = [0u8; 2048];
    let mut dp = 0x7f7;
    while out.len() < n {
        if version == 0x10 {
            flags >>= 1;
            if flags & 0x100 == 0 {
                flags = u16::from(r.byte()?) | 0xff00;
            }
            let v = r.byte()?;
            let count = if flags & 1 != 0 {
                usize::from(r.byte()?) + 2
            } else {
                1
            };
            out.extend(std::iter::repeat_n(v, count.min(n - out.len())));
        } else if version == 0x21 {
            flags <<= 1;
            if flags & 0x80 == 0 {
                flags = (u16::from(r.byte()?) << 8) | 255;
            }
            if flags & 0x8000 != 0 {
                let hi = r.byte()?;
                let lo = r.byte()?;
                let mut src = (usize::from(hi & 7) << 8) | usize::from(lo);
                let count = usize::from(hi >> 3) + 3;
                for _ in 0..count.min(n - out.len()) {
                    let v = dict[src];
                    src = (src + 1) & 2047;
                    out.push(v);
                    dict[dp] = v;
                    dp = (dp + 1) & 2047;
                }
            } else {
                let v = r.byte()?;
                out.push(v);
                dict[dp] = v;
                dp = (dp + 1) & 2047;
            }
        } else {
            bail!("unsupported ISG version");
        }
    }
    let mut image = indexed(w, h, &out, &palette)?;
    flip(&mut image);
    Ok(image)
}
fn yb(b: &[u8], triangle: bool) -> Result<ImageSurface> {
    let flags = *b.get(2).context("short YB")?;
    let channels = usize::from(*b.get(3).context("short YB")?);
    ensure!(
        !triangle || flags & 64 == 0,
        "unsupported triangle YB flag 0x40"
    );
    ensure!(
        if triangle {
            channels == 4
        } else {
            channels == 3
        },
        "unsupported YB version/channel count"
    );
    let w = u32::from(u16le(b, 12)?);
    let h = u32::from(u16le(b, 14)?);
    let n = rgba_len(w, h)? / 4 * channels;
    let mut r = Reader::new(b, 16);
    if !triangle {
        slice(b, 16, u32le(b, 4)? as usize)?;
    }
    let mut out = Vec::with_capacity(n);
    let mut control = 0u16;
    while out.len() < n {
        control <<= 1;
        if control & 128 == 0 {
            control = (u16::from(r.byte()?) << 8) | 255;
        }
        if control & 0x8000 == 0 {
            out.push(r.byte()?);
            continue;
        }
        let a = r.byte()?;
        let (dist, count) = if a & 128 != 0 {
            let c = r.byte()?;
            if a & 64 != 0 {
                let d = r.byte()?;
                let count = match d {
                    255 => 4096,
                    254 => 1024,
                    253 => 256,
                    _ => usize::from(d) + 3,
                };
                (usize::from(c) + ((usize::from(a) & 63) << 8) + 1, count)
            } else {
                (
                    usize::from(c >> 4) + ((usize::from(a) & 63) << 4) + 1,
                    usize::from(c & 15) + 3,
                )
            }
        } else if a & 3 == 3 {
            let count = usize::from(a >> 2) + 9;
            ensure!(count <= n - out.len(), "YB literal run overflow");
            out.extend(r.take(count)?);
            continue;
        } else {
            (usize::from(a >> 2) + 1, usize::from(a & 3) + 2)
        };
        let count = count.min(n - out.len());
        copy_back(&mut out, dist, count, n)?;
    }
    if (triangle && flags & 128 != 0) || (!triangle && flags > 0) {
        for i in channels..out.len() {
            out[i] = out[i].wrapping_add(out[i - channels]);
        }
    }
    raw(w, h, &out, channels)
}
fn mcg(b: &[u8]) -> Result<ImageSurface> {
    let kind = *b.get(3).context("short MCG")?;
    ensure!(kind < 8, "unsupported MCG type");
    let mut r = Reader::new(b, 4);
    let tw = u32::from(r.be16()?);
    let th = u32::from(r.be16()?);
    let x = u32::from(r.be16()?);
    let y = u32::from(r.be16()?);
    let w = u32::from(r.be16()?);
    let h = u32::from(r.be16()?);
    let n = rgba_len(w, h)? / 4;
    if kind >= 6 {
        return Ok(ImageSurface {
            width: w,
            height: h,
            rgba: [0, 0, 0, 255].repeat(n),
        });
    }
    let channels = match kind {
        0 | 2 => 3,
        1 | 3 => 4,
        _ => 1,
    };
    let mut data = if [2, 3, 5].contains(&kind) {
        let marker = r.byte()?;
        let mut out = Vec::with_capacity(n * channels);
        while out.len() < n * channels {
            let c = r.byte()?;
            if c == marker {
                let count = usize::from(r.byte()?);
                let v = r.byte()?;
                ensure!(count <= n * channels - out.len(), "MCG RLE overflow");
                out.extend(std::iter::repeat_n(v, count));
            } else {
                out.push(c);
            }
        }
        out
    } else {
        r.take(n * channels)?.to_vec()
    };
    if [0, 2, 3].contains(&kind) {
        let old = data.clone();
        for c in 0..channels {
            for p in 0..n {
                data[p * channels + c] = old[c * n + p];
            }
        }
    }
    let image = raw(w, h, &data, channels)?;
    let mut image = offset(image, x, y, Some((tw, th)))?;
    flip(&mut image);
    Ok(image)
}
fn ep(b: &[u8]) -> Result<ImageSurface> {
    let mode = *b.get(3).context("short EP")?;
    let kind = u32le(b, 4)?;
    let channels = match kind {
        1 => 3,
        2 => 4,
        3 => 2,
        _ => bail!("unsupported EP color type"),
    };
    let w = u32le(b, 8)?;
    let h = u32le(b, 12)?;
    let n = rgba_len(w, h)? / 4;
    let mut r = Reader::new(b, 16);
    let (x, y) = if mode == 2 {
        (r.le32()?, r.le32()?)
    } else {
        (0, 0)
    };
    let wv = i64::from(w);
    let offsets = [
        0,
        1,
        wv,
        wv + 1,
        2,
        wv - 1,
        wv * 2,
        3,
        (wv + 1) * 2,
        wv + 2,
        wv * 2 + 1,
        wv * 2 - 1,
        (wv - 1) * 2,
        wv - 2,
        wv * 3,
        4,
    ];
    let mut data = Vec::with_capacity(n * channels);
    while data.len() < n * channels {
        let flag = r.byte()?;
        if flag & 0xf0 != 0 {
            let count = if flag & 8 != 0 {
                usize::from(r.byte()?) + (usize::from(flag & 7) << 8)
            } else {
                usize::from(flag & 7)
            };
            let distance =
                usize::try_from(offsets[usize::from(flag >> 4)]).context("invalid EP offset")?;
            copy_back(&mut data, distance, count, n * channels)?;
        } else {
            let count = usize::from(flag).min(n * channels - data.len());
            ensure!(count > 0, "EP empty run");
            data.extend(r.take(count)?);
        }
    }
    let old = data.clone();
    for c in 0..channels {
        for p in 0..n {
            data[p * channels + c] = old[c * n + p];
        }
    }
    offset(raw(w, h, &data, channels)?, x, y, None)
}
fn akb(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let plus = b.starts_with(b"AKB+");
    let w = u32::from(u16le(b, 4)?);
    let h = u32::from(u16le(b, 6)?);
    let len = rgba_len(w, h)?;
    let channels = if u32be(b, 8)? & 64 != 0 { 3 } else { 4 };
    let bg = slice(b, 12, 4)?;
    let x1 = u32le(b, 16)? as i32;
    let y1 = u32le(b, 20)? as i32;
    let x2 = u32le(b, 24)? as i32;
    let y2 = u32le(b, 28)? as i32;
    ensure!(x2 >= x1 && y2 >= y1, "invalid AKB rectangle");
    let cw = u32::try_from(i64::from(x2) - i64::from(x1))?;
    let ch = u32::try_from(i64::from(y2) - i64::from(y1))?;
    let mut p = 32;
    let mut base = None;
    if plus {
        let name = slice(b, 32, 32)?;
        p += 32;
        let name = name.split(|v| *v == 0).next().unwrap_or_default();
        let name = match std::str::from_utf8(name) {
            Ok(name) => std::borrow::Cow::Borrowed(name),
            Err(_) => {
                let (name, errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(name);
                ensure!(!errors, "invalid AKB Shift-JIS filename");
                name
            }
        };
        if !name.is_empty() {
            let file = name.rsplit(['/', '\\']).next().unwrap_or(&name);
            let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
            // Original VFS searches by stem across extensions. Capability VFS
            // has no global enumeration, so try the authored sibling first,
            // then the conventional native AKB/BMP names, never another root.
            let candidates = [
                file.to_owned(),
                format!("{stem}.akb"),
                format!("{stem}.AKB"),
                format!("{stem}.bmp"),
                format!("{stem}.BMP"),
            ];
            for (index, candidate) in candidates.iter().enumerate() {
                if candidates[..index].contains(candidate) {
                    continue;
                }
                if let Some(data) = ctx.try_read_relative(candidate)? {
                    ensure!(
                        data.starts_with(b"AKB ") || data.starts_with(b"AKB+"),
                        "AKB base has invalid signature"
                    );
                    base = Some(ctx.decode_embedded(candidate, &data)?);
                    break;
                }
            }
        }
    }

    let mut image = base.unwrap_or_else(|| ImageSurface {
        width: w,
        height: h,
        rgba: [bg[2], bg[1], bg[0], bg[3]].repeat(len / 4),
    });
    ensure!(
        image.width == w && image.height == h,
        "AKB base dimensions mismatch"
    );
    if cw == 0 || ch == 0 {
        return Ok(image);
    }
    let n = rgba_len(cw, ch)? / 4 * channels;
    let mut data = lzss(slice(b, p, b.len().saturating_sub(p))?, n, 0xfee, false)?;
    let stride = cw as usize * channels;
    for y in 0..ch as usize / 2 {
        for x in 0..stride {
            data.swap(y * stride + x, (ch as usize - 1 - y) * stride + x);
        }
    }
    for x in channels..stride {
        data[x] = data[x].wrapping_add(data[x - channels]);
    }
    for y in 1..ch as usize {
        for x in 0..stride {
            let i = y * stride + x;
            data[i] = data[i].wrapping_add(data[i - stride]);
        }
    }
    let overlay = raw(cw, ch, &data, channels)?;
    for sy in 0..ch {
        for sx in 0..cw {
            let dx = i64::from(x1) + i64::from(sx);
            let dy = i64::from(y1) + i64::from(sy);
            if dx < 0 || dy < 0 || dx >= i64::from(w) || dy >= i64::from(h) {
                continue;
            }
            let src = (sy as usize * cw as usize + sx as usize) * 4;
            let v = &overlay.rgba[src..src + 4];
            if v[3] == 0 || (plus && v[..3] == [0, 255, 0]) {
                continue;
            }
            let dst = (dy as usize * w as usize + dx as usize) * 4;
            image.rgba[dst..dst + 4].copy_from_slice(v);
        }
    }
    Ok(image)
}
fn twilight(b: &[u8], ctx: &ImageDecodeContext<'_>, tfbm: bool) -> Result<ImageSurface> {
    let mut r = Reader::new(b, if tfbm { 5 } else { 0 });
    let depth = r.byte()?;
    let w = r.le32()?;
    let h = r.le32()?;
    let stride = r.le32()?;
    ensure!(stride >= w, "image stride smaller than width");
    let aux = r.le32()?;
    let data = if tfbm {
        inflate(r.take(aux as usize)?)?
    } else {
        r.take(b.len() - r.p)?.to_vec()
    };
    let mut source = Reader::new(&data, 0);
    let mut palette: Vec<[u8; 4]> = (0..256)
        .map(|i| {
            let v = (i * 255 / 256) as u8;
            [v, v, v, 255]
        })
        .collect();
    if depth == 8 {
        let name = if tfbm {
            "palette000.bmp".to_owned()
        } else {
            format!("palette{aux:03}.pal")
        };
        if let Some(bytes) = ctx.try_read_relative(&name)? {
            let pal = if tfbm {
                ensure!(bytes.starts_with(b"TFPA\0"), "invalid TFPA palette");
                inflate(slice(&bytes, 9, u32le(&bytes, 5)? as usize)?)?
            } else {
                slice(&bytes, 1, 512)?.to_vec()
            };
            for (i, c) in palette.iter_mut().enumerate() {
                *c = color16(u16le(&pal, i * 2)?, true);
            }
        }
    }
    let mut rgba = Vec::with_capacity(rgba_len(w, h)?);
    ensure!(
        u64::from(stride) * u64::from(h) <= crysirror_image::MAX_DECODED_BYTES as u64,
        "image stride exceeds limit"
    );
    for _ in 0..h {
        for x in 0..stride {
            let c = match depth {
                8 => palette[usize::from(source.byte()?)],
                16 if tfbm => {
                    let c = source.le16()?;
                    [
                        ((c >> 8) & 248) as u8,
                        ((c >> 3) & 252) as u8,
                        ((c << 3) & 248) as u8,
                        255,
                    ]
                }
                24 if !tfbm => {
                    let p = source.take(4)?;
                    [p[2], p[1], p[0], p[3]]
                }
                32 => {
                    let p = source.take(4)?;
                    [p[2], p[1], p[0], p[3]]
                }
                _ => bail!("unsupported twilight image depth"),
            };
            if x < w {
                rgba.extend(c);
            }
        }
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
