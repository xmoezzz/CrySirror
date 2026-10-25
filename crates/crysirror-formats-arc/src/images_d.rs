// SPDX-License-Identifier: GPL-3.0-or-later
// Layouts/algorithms translated from pinned arc_unpacker, Copyright (C) 2016 rr-.
use anyhow::{bail, ensure, Context, Result};
use crysirror_image::{rgba_len, ImageDecodeContext, ImageSurface, MAX_DECODED_BYTES};
use std::collections::BTreeMap;
pub const FORMATS: &[&str] = &[
    "glib/pgx",
    "glib/jpeg-pgx",
    "playstation/gim",
    "playstation/gtf",
    "cri/xtx",
];
fn take(b: &[u8], p: usize, n: usize) -> Result<&[u8]> {
    b.get(p..p.checked_add(n).context("image offset overflow")?)
        .context("truncated image")
}
fn le16(b: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(take(b, p, 2)?.try_into()?))
}
fn le32(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(take(b, p, 4)?.try_into()?))
}
fn be16(b: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(take(b, p, 2)?.try_into()?))
}
fn be32(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(take(b, p, 4)?.try_into()?))
}
fn pgx_start(b: &[u8]) -> bool {
    (|| -> Result<bool> {
        if !b.starts_with(b"\xff\xd8\xff\xe0") {
            return Ok(false);
        }
        let n = be16(b, 4)? as usize;
        ensure!(n >= 2, "JPEG marker length");
        let at = 4 + n;
        Ok(take(b, at, 2)? == b"\xff\xe3"
            && be16(b, at + 2)? >= 6
            && take(b, at + 4, 4)? == b"PGX\0")
    })()
    .unwrap_or(false)
}
fn xtx_start(b: &[u8]) -> Option<usize> {
    if b.starts_with(b"xtx\0") {
        Some(0)
    } else {
        let n = le32(b, 0).ok()? as usize;
        (n < 4096 && b.get(n..n + 4) == Some(b"xtx\0".as_slice())).then_some(n)
    }
}
pub fn probe(id: &str, _: &str, b: &[u8]) -> bool {
    match id {
        "glib/pgx" => b.starts_with(b"PGX\0"),
        "glib/jpeg-pgx" => pgx_start(b),
        "playstation/gim" => b.starts_with(b"MIG.00.1PSP\0"),
        "playstation/gtf" => be32(b, 4)
            .ok()
            .zip(be32(b, 16).ok())
            .is_some_and(|(n, h)| u64::from(n) + u64::from(h) == b.len() as u64),
        "cri/xtx" => xtx_start(b).is_some(),
        _ => false,
    }
}
pub fn decode(id: &str, b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    ensure!(
        b.len() <= MAX_DECODED_BYTES,
        "encoded D-group image exceeds 64 MiB"
    );
    ensure!(probe(id, ctx.path, b), "image does not match {id}");
    let surface = match id {
        "glib/pgx" => pgx(b, false),
        "glib/jpeg-pgx" => pgx(b, true),
        "playstation/gim" => gim(b),
        "playstation/gtf" => gtf(b, ctx),
        "cri/xtx" => xtx(b, ctx),
        _ => bail!("unsupported image {id}"),
    }?;
    surface.validate()?;
    Ok(surface)
}
fn lzss(b: &[u8], n: usize) -> Result<(Vec<u8>, usize)> {
    ensure!(n <= MAX_DECODED_BYTES, "PGX decoded byte limit");
    let mut dict = [0u8; 4096];
    let mut pos = 0xfee;
    let mut at = 0;
    let mut control = 0u16;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        control >>= 1;
        if control & 0x100 == 0 {
            control = u16::from(*take(b, at, 1)?.first().unwrap()) | 0xff00;
            at += 1;
        }
        if control & 1 != 0 {
            let v = take(b, at, 1)?[0];
            at += 1;
            out.push(v);
            dict[pos] = v;
            pos = (pos + 1) & 4095;
        } else {
            let pair = take(b, at, 2)?;
            at += 2;
            let mut src = usize::from(pair[0]) | ((usize::from(pair[1]) & 0xf0) << 4);
            let count = usize::from(!pair[1] & 15) + 3;
            for _ in 0..count.min(n - out.len()) {
                let v = dict[src];
                src = (src + 1) & 4095;
                out.push(v);
                dict[pos] = v;
                pos = (pos + 1) & 4095;
            }
        }
    }
    Ok((out, at))
}
fn pgx(file: &[u8], jpeg: bool) -> Result<ImageSurface> {
    let mut collected = Vec::new();
    let b = if jpeg {
        ensure!(pgx_start(file), "invalid JPEG-PGX header");
        let mut at = 4 + usize::from(be16(file, 4)?);
        while file.get(at..at + 2) == Some(b"\xff\xe3".as_slice()) {
            let n = usize::from(be16(file, at + 2)?);
            ensure!(n >= 2, "invalid JPEG-PGX APP3 length");
            ensure!(
                collected
                    .len()
                    .checked_add(n - 2)
                    .is_some_and(|n| n <= MAX_DECODED_BYTES),
                "JPEG-PGX payload limit"
            );
            collected.extend_from_slice(take(file, at + 4, n - 2)?);
            at += n + 2;
        }
        collected.as_slice()
    } else {
        file
    };
    ensure!(b.starts_with(b"PGX\0"), "PGX magic");
    let w = le32(b, 8)?;
    let h = le32(b, 12)?;
    let alpha = le16(b, 16)? != 0;
    let n = rgba_len(w, h)?;
    let at = if jpeg {
        let mut at = 32;
        if !alpha {
            let x = le32(b, 40)?;
            let y = le32(b, 44)?;
            let extra = ((y & 0x00ff00ff) | (x & 0xff00ff00)) as usize;
            let (_, used) = lzss(take(b, 48, b.len().saturating_sub(48))?, extra)?;
            at = 48 + used;
        }
        at
    } else {
        b.len()
            .checked_sub(le32(b, 20)? as usize)
            .context("PGX source range")?
    };
    ensure!(at >= 24, "PGX pixel payload overlaps its header");
    let (mut rgba, _) = lzss(take(b, at, b.len().saturating_sub(at))?, n)?;
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
        if !alpha {
            p[3] = 255;
        }
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
fn pixel(b: &[u8], p: usize, format: u16) -> Result<[u8; 4]> {
    if format == 3 {
        return Ok(take(b, p, 4)?.try_into()?);
    }
    let v = le16(b, p)?;
    Ok(match format {
        0 => [
            ((v & 31) << 3) as u8,
            (((v >> 5) & 63) << 2) as u8,
            (((v >> 11) & 31) << 3) as u8,
            255,
        ],
        1 => [
            ((v & 31) << 3) as u8,
            (((v >> 5) & 31) << 3) as u8,
            (((v >> 10) & 31) << 3) as u8,
            if v & 0x8000 != 0 { 255 } else { 0 },
        ],
        2 => [
            ((v & 15) << 4) as u8,
            (((v >> 4) & 15) << 4) as u8,
            (((v >> 8) & 15) << 4) as u8,
            (((v >> 12) & 15) << 4) as u8,
        ],
        _ => bail!("unsupported GIM palette format {format}"),
    })
}
fn gim(b: &[u8]) -> Result<ImageSurface> {
    ensure!(b.starts_with(b"MIG.00.1PSP\0"), "GIM signature");
    let mut at = 0x30;
    let mut chunks = BTreeMap::new();
    while at < b.len() {
        let typ = le32(b, at)?;
        let n = le32(b, at + 4)? as usize;
        ensure!(
            n >= 12 && le32(b, at + 8)? as usize == n,
            "compressed or invalid GIM chunk"
        );
        take(b, at, n)?;
        chunks.insert(typ, (at, n));
        at = at.checked_add(n).context("GIM chunk overflow")?;
    }
    let &(chunk, size) = chunks.get(&4).context("GIM bitmap missing")?;
    let data = take(b, chunk, size)?;
    let format = le16(data, 0x14)?;
    let tiled = le16(data, 0x16)? == 1;
    let w = (u32::from(le16(data, 0x18)?) + 15) & !15;
    let h = (u32::from(le16(data, 0x1a)?) + 7) & !7;
    let len = rgba_len(w, h)?;
    let bits = match format {
        0..=2 => 16,
        3 => 32,
        4 => 4,
        5 => 8,
        6 => 16,
        7 => 32,
        _ => bail!("unsupported GIM pixel format"),
    };
    // The pinned decoder has no working indexed16/32 branch. Reject these
    // instead of dereferencing a null result as the original implementation can.
    ensure!(
        format < 6,
        "GIM indexed16/32 are unsupported by the pinned decoder"
    );
    let stride = w as usize * bits / 8;
    let n = h as usize * stride;
    let offset = 0x10usize
        .checked_add(le32(data, 0x2c)? as usize)
        .context("GIM data offset")?;
    let source = take(data, offset, n)?;
    let mut linear = source.to_vec();
    if tiled {
        ensure!(
            stride.is_multiple_of(16) && h.is_multiple_of(8),
            "GIM swizzle alignment"
        );
        for y in 0..h as usize {
            for x in 0..stride {
                let idx = (x / 16 + y / 8 * (stride / 16)) * 128 + x % 16 + y % 8 * 16;
                linear[y * stride + x] = *source.get(idx).context("GIM swizzle range")?;
            }
        }
    }
    let mut palette = Vec::new();
    if format >= 4 {
        let &(p, n) = chunks.get(&5).context("GIM palette missing")?;
        let data = take(b, p, n)?;
        let fmt = le16(data, 0x14)?;
        ensure!(fmt <= 3, "GIM palette encoding");
        let count = usize::from(le16(data, 0x18)?);
        ensure!(count <= 256 && count > 0, "GIM palette count");
        let step = if fmt == 3 { 4 } else { 2 };
        for i in 0..count {
            palette.push(pixel(data, 0x50 + i * step, fmt)?);
        }
    }
    let mut rgba = Vec::with_capacity(len);
    for i in 0..len / 4 {
        let p = if format < 4 {
            pixel(&linear, i * bits / 8, format)?
        } else {
            let index = if format == 4 {
                usize::from((linear[i / 2] >> ((i % 2) * 4)) & 15)
            } else {
                usize::from(linear[i])
            };
            *palette.get(index).context("GIM palette index")?
        };
        rgba.extend(p);
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
fn dxt(
    data: &[u8],
    w: u32,
    h: u32,
    kind: u8,
    ctx: &ImageDecodeContext<'_>,
) -> Result<ImageSurface> {
    rgba_len(w, h)?;
    let block = if kind == 1 { 8 } else { 16 };
    let n = ((w as usize + 3) / 4) * ((h as usize + 3) / 4) * block;
    take(data, 0, n)?;
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&h.to_le_bytes());
    bytes[16..20].copy_from_slice(&w.to_le_bytes());
    bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
    bytes[80..84].copy_from_slice(&4u32.to_le_bytes());
    bytes[84..88].copy_from_slice(match kind {
        1 => b"DXT1",
        3 => b"DXT3",
        5 => b"DXT5",
        _ => bail!("unsupported DXT format"),
    });
    bytes.extend_from_slice(&data[..n]);
    crate::images_b::decode("microsoft/dds", &bytes, ctx)
}
fn gtf(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    ensure!(probe("playstation/gtf", "", b), "GTF range header");
    ensure!(be32(b, 8)? == 1, "multi-texture GTF unsupported");
    ensure!(be16(b, 36)? == 1, "3D GTF unsupported");
    let format = take(b, 24, 1)?[0] & 0x9f;
    let kind = match format {
        0x86 => 1,
        0x87 => 3,
        0x88 => 5,
        _ => bail!("non-DXT GTF unsupported by pinned decoder"),
    };
    let offset = be32(b, 16)? as usize;
    let w = u32::from(be16(b, 32)?);
    let h = u32::from(be16(b, 34)?);
    dxt(
        take(b, offset, b.len().saturating_sub(offset))?,
        w,
        h,
        kind,
        ctx,
    )
}
fn tiled(i: usize, width: usize, level: usize) -> Result<(usize, usize)> {
    ensure!(width > 0 && width <= 8192, "XTX tile width");
    let a = (level >> 2) + ((level >> 1) >> (level >> 2));
    let v2 = i << a;
    let v3 = (v2 & 0x3f) + ((v2 >> 2) & 0x1c0) + ((v2 >> 3) & 0x1ffffe00);
    let columns = (width + 31) >> 5;
    let xlow = (((level << 3) - 1) & ((v3 >> 1) ^ ((v3 ^ (v3 >> 1)) & 15))) >> a;
    let xmajor = ((v2 >> 6) & 255) + ((v3 >> (a + 5)) & 0xfe);
    let x = xlow + (((xmajor & 3) + (((v3 >> (a + 7)) % columns) << 2)) << 3);
    let middle = (v3 & ((level << 6) - 1) & !31) + ((((v2 & 0x3f) + ((v2 >> 2) & 0xc0)) & 15) << 1);
    let ymajor = ((v2 >> 10) & 2) + ((v3 >> (a + 6)) & 1) + (((v3 >> (a + 7)) / columns) << 2);
    let y = ((v3 >> 4) & 1) + ((middle >> (a + 3)) & !1) + (ymajor << 3);
    Ok((x, y))
}
fn xtx(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let start = xtx_start(b).context("XTX signature")?;
    let b = &b[start..];
    let format = take(b, 4, 1)?[0];
    let aw = be32(b, 8)?;
    let ah = be32(b, 12)?;
    let w = be32(b, 16)?;
    let h = be32(b, 20)?;
    let output = rgba_len(w, h)?;
    rgba_len(aw, ah)?;
    ensure!(w <= aw && h <= ah, "XTX crop exceeds aligned canvas");
    let source = &b[32.min(b.len())..];
    if format == 2 {
        ensure!(
            aw.is_multiple_of(4) && ah.is_multiple_of(4),
            "XTX DXT alignment"
        );
        let tw = aw as usize / 4;
        let th = ah as usize / 4;
        let n = aw as usize * ah as usize;
        let source = take(source, 0, n)?;
        let mut linear = vec![0; n];
        for i in 0..tw * th {
            let (x, y) = tiled(i, tw, 16)?;
            let dest = (x + y * tw) * 16;
            let block = take(source, i * 16, 16)?;
            let dest = linear
                .get_mut(dest..dest + 16)
                .context("XTX DXT tile outside canvas")?;
            for j in (0..16).step_by(2) {
                dest[j] = block[j + 1];
                dest[j + 1] = block[j];
            }
        }
        let decoded = dxt(&linear, aw, ah, 5, ctx)?;
        let mut rgba = Vec::with_capacity(output);
        for row in 0..h as usize {
            rgba.extend_from_slice(
                &decoded.rgba[row * decoded.width as usize * 4
                    ..row * decoded.width as usize * 4 + w as usize * 4],
            );
        }
        return Ok(ImageSurface {
            width: w,
            height: h,
            rgba,
        });
    }
    ensure!(format <= 1, "unsupported XTX pixel format");
    let step = if format == 0 { 4 } else { 2 };
    let source = take(source, 0, aw as usize * ah as usize * step)?;
    // Native format1 starts with zero BGR565 pixels; even unfilled tiled
    // padding therefore converts to opaque black, unlike format0 BGRA zeros.
    let mut rgba = if format == 1 {
        [0, 0, 0, 255].repeat(output / 4)
    } else {
        vec![0; output]
    };
    for i in 0..aw as usize * ah as usize {
        let (x, y) = tiled(i, aw as usize, step)?;
        if x >= w as usize || y >= h as usize {
            continue;
        }
        let p = i * step;
        let dest = (x + y * w as usize) * 4;
        let pixel = if format == 0 {
            [source[p + 1], source[p + 2], source[p + 3], source[p]]
        } else {
            let v = u16::from_be_bytes([source[p], source[p + 1]]);
            [
                (((v >> 11) & 31) << 3) as u8,
                (((v >> 5) & 63) << 2) as u8,
                ((v & 31) << 3) as u8,
                255,
            ]
        };
        rgba[dest..dest + 4].copy_from_slice(&pixel);
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
