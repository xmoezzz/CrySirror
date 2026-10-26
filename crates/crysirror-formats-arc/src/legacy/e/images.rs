// SPDX-License-Identifier: GPL-3.0-or-later
// Native image-archive records ported from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
use crysirror_image::bitmap::{BitmapRect, BlitOptions, PixelMath};
use crysirror_image::{encode_png_uncompressed, png_uncompressed_len, rgba_len, ImageSurface};
fn blit(dst: &mut ImageSurface, src: &ImageSurface, x: i32, y: i32) -> Result<()> {
    dst.blit(
        src,
        BlitOptions {
            source_rect: BitmapRect::new(0, 0, src.width, src.height),
            destination: [x, y],
            opacity: 255,
            mode: PixelMath::Copy,
            color_key: None,
        },
    )
}
fn output_size(w: u32, h: u32, cx: &DecodeContext) -> Result<u64> {
    let n = png_uncompressed_len(w, h)? as u64;
    ensure!(
        n <= cx.limits.max_entry_size,
        "PNG archive output byte limit"
    );
    Ok(n)
}
fn png_name(path: String) -> String {
    if let Some((stem, _)) = path.rsplit_once('.') {
        format!("{stem}.png")
    } else {
        format!("{path}.png")
    }
}
pub(super) fn egr(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let mut at = 0;
    let len = length(s)?;
    let mut rows = Vec::new();
    while at < len {
        limits(cx, rows.len() + 1)?;
        let h = bytes(s, at, 12, 12)?;
        let w = le32(&h, 0)?;
        let height = le32(&h, 4)?;
        let pixels = rgba_len(w, height)? / 4;
        ensure!(le32(&h, 8)? as usize == pixels, "EGR pixel count");
        let size = output_size(w, height, cx)?;
        let stored = 0x574 + pixels as u64;
        range(len, at + 12, stored)?;
        rows.push(Row::packed(
            format!("Image{:03}.png", rows.len()),
            at + 12,
            stored,
            size,
            decode(move |b, _| {
                ensure!(b.len() == 0x574 + pixels, "EGR length");
                let mut rgba = Vec::with_capacity(pixels * 4);
                for v in &b[0x574..] {
                    let i = *v as usize * 4;
                    rgba.extend([b[i + 2], b[i + 3], b[i + 1], 255]);
                }
                encode_png_uncompressed(&ImageSurface {
                    width: w,
                    height,
                    rgba,
                })
            }),
        ));
        at += 12 + stored;
    }
    Ok(rows)
}
pub(super) fn blue_images(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 16, 16)?;
    let mut c = Cursor::new(&h);
    let stored = c.le32()? as u64;
    let n = count(&mut c, cx)?;
    let at = c.le16()? as u64;
    let width = c.le16()? as u32;
    let height = c.le16()? as u32;
    let _kind = c.le16()?;
    ensure!(at == 16 + n as u64 * 40, "BlueArrow bitmap offset");
    ensure!(
        stored == rgba_len(width, height)? as u64,
        "BlueArrow bitmap size"
    );
    range(length(s)?, at, stored)?;
    let b = window(s, cx, at + stored)?;
    let mut names = Cursor::new(&b);
    let table = index(s, cx, 16, n as u64 * 40)?;
    let mut rows = Vec::new();
    for p in table.chunks_exact(40) {
        let f = |at| -> Result<f32> {
            let v = f32::from_le_bytes(p[at..at + 4].try_into()?);
            ensure!(v.is_finite(), "BlueArrow nonfinite coordinate");
            Ok(v)
        };
        let w = f(0)?;
        let h = f(4)?;
        ensure!(
            w > 0. && h > 0. && w <= 8192. && h <= 8192.,
            "BlueArrow crop dimensions"
        );
        let (w, h) = (w as u32, h as u32);
        let dx = (-f(8)? * width as f32) as i32;
        let dy = (-f(12)? * height as f32) as i32;
        let size = output_size(w, h, cx)?;
        let path = png_name(sjis(names.zero()?)?);
        rows.push(Row::packed(
            path,
            at,
            stored,
            size,
            decode(move |b, _| {
                let mut rgba = b.to_vec();
                crysirror_image::swap_red_blue(&mut rgba)?;
                let src = ImageSurface {
                    width,
                    height,
                    rgba,
                };
                let mut dst = ImageSurface::filled(w, h, [0; 4])?;
                blit(&mut dst, &src, dx, dy)?;
                encode_png_uncompressed(&dst)
            }),
        ));
    }
    Ok(rows)
}
struct Bits<'a> {
    b: &'a [u8],
    at: usize,
    lsb: bool,
}
impl Bits<'_> {
    fn bit(&mut self) -> Result<u32> {
        let b = *self
            .b
            .get(self.at / 8)
            .context("truncated image control bits")?;
        let shift = if self.lsb {
            self.at % 8
        } else {
            7 - self.at % 8
        };
        self.at += 1;
        Ok(((b >> shift) & 1) as u32)
    }
    fn gamma(&mut self) -> Result<usize> {
        let mut n = 0;
        while self.bit()? == 0 {
            n += 1;
            ensure!(n < 31, "gamma overflow");
        }
        let mut v = 1usize << n;
        for i in (0..n).rev() {
            v |= (self.bit()? as usize) << i;
        }
        Ok(v)
    }
}
pub(super) fn hg3(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"HG-3")?, "missing HG-3");
    let len = length(s)?;
    let mut at = 12;
    let mut rows = Vec::new();
    while at < len {
        limits(cx, rows.len() + 1)?;
        let h = bytes(s, at, 8, 8)?;
        let next = le32(&h, 0)? as u64;
        let stored = if next == 0 {
            len - at - 8
        } else {
            ensure!(next >= 8, "HG3 frame size");
            next - 8
        };
        range(len, at + 8, stored)?;
        let head = index(s, cx, at + 8, stored.min(cx.limits.max_index_size))?;
        let mut c = Cursor::new(&head);
        let mut dimensions = None;
        while c.left() > 8 {
            let name = c.take(8)?;
            c.take(4)?;
            let n = c.le32()? as usize;
            let data = c.take(n)?;
            if name == b"stdinfo\0" {
                ensure!(n >= 28, "HG3 stdinfo size");
                dimensions = Some((le32(data, 20)?, le32(data, 24)?));
                break;
            }
        }
        let (w, h) = dimensions.context("missing HG3 stdinfo")?;
        let size = output_size(w, h, cx)?;
        let bound = cx.limits.max_entry_size;
        rows.push(Row::packed(
            format!("{:03}.png", rows.len()),
            at + 8,
            stored,
            size,
            decode(move |b, _| {
                let mut c = Cursor::new(b);
                let mut stdinfo = None;
                let mut img = None;
                while c.left() > 8 {
                    let name = c.take(8)?;
                    c.take(4)?;
                    let n = c.le32()? as usize;
                    let p = c.take(n)?;
                    if name == b"stdinfo\0" {
                        stdinfo = Some(p);
                    }
                    if name == b"img0000\0" {
                        img = Some(p);
                    }
                    if name == b"img_jpg\0" {
                        bail!("HG3 img_jpg is unsupported by pinned upstream");
                    }
                }
                let info = stdinfo.context("missing HG3 stdinfo")?;
                let width = le32(info, 0)?;
                let height = le32(info, 4)?;
                let depth = le32(info, 8)?;
                ensure!(
                    depth == 32,
                    "HG3 img0000 depth must be 32 in pinned decoder"
                );
                let pixel_bytes = rgba_len(width, height)?;
                ensure!(pixel_bytes as u64 <= bound, "HG3 pixel limit");
                let x = le32(info, 12)? as i32;
                let y = le32(info, 16)? as i32;
                let data = img.context("HG3 image chunk absent")?;
                let mut c = Cursor::new(data);
                c.take(8)?;
                let dc = c.le32()? as usize;
                let ds = c.le32()? as u64;
                let cc = c.le32()? as usize;
                let cs = c.le32()? as u64;
                ensure!(ds <= bound && cs <= bound, "HG3 inflated chunk limit");
                let d = inflate_index(c.take(dc)?, ds)?;
                let ctl = inflate_index(c.take(cc)?, cs)?;
                ensure!(
                    d.len() as u64 == ds && ctl.len() as u64 == cs,
                    "HG3 chunk length mismatch"
                );
                let mut bits = Bits {
                    b: &ctl,
                    at: 0,
                    lsb: true,
                };
                let mut copy = bits.bit()? != 0;
                let size = bits.gamma()?;
                ensure!(
                    size == pixel_bytes && size % 4 == 0,
                    "HG3 image control output length"
                );
                let mut tmp = vec![0; size];
                let mut out = 0;
                let mut source = 0;
                while out < size {
                    let n = bits.gamma()?;
                    ensure!(n <= size - out, "HG3 run outside output");
                    if copy {
                        let part = d
                            .get(source..source + n)
                            .context("HG3 literal outside input")?;
                        tmp[out..out + n].copy_from_slice(part);
                        source += n;
                    }
                    out += n;
                    copy = !copy;
                }
                let mut rgba = vec![0; size];
                let plane = size / 4;
                for i in 0..plane {
                    let mut value = 0u32;
                    for p in 0..4 {
                        let v = tmp[p * plane + i] as u32;
                        let mut spread = 0;
                        for shift in [6, 4, 2, 0] {
                            spread = (spread << 6) | (v & (3 << shift));
                        }
                        value |= spread << (6 - p * 2);
                    }
                    for j in 0..4 {
                        let v = (value >> (j * 8)) as u8;
                        rgba[i * 4 + j] = if v & 1 != 0 { (v >> 1) ^ 255 } else { v >> 1 };
                    }
                }
                let stride = width as usize * 4;
                for x in 4..stride {
                    rgba[x] = rgba[x].wrapping_add(rgba[x - 4]);
                }
                for y in 1..height as usize {
                    for x in 0..stride {
                        let i = y * stride + x;
                        rgba[i] = rgba[i].wrapping_add(rgba[i - stride]);
                    }
                }
                crysirror_image::swap_red_blue(&mut rgba)?;
                let mut flipped = Vec::with_capacity(size);
                for row in rgba.chunks_exact(stride).rev() {
                    flipped.extend(row);
                }
                let src = ImageSurface {
                    width,
                    height,
                    rgba: flipped,
                };
                let mut dst = ImageSurface::filled(w, h, [0; 4])?;
                blit(&mut dst, &src, x, y)?;
                encode_png_uncompressed(&dst)
            }),
        ));
        at += 8 + stored;
    }
    Ok(rows)
}
pub(super) fn mca(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"MCA ")?, "missing MCA");
    let h = bytes(s, 16, 20, 20)?;
    let base = le32(&h, 0)? as u64;
    let n = le32(&h, 16)? as usize;
    limits(cx, n)?;
    let opt = cx
        .parameters
        .get("fc01/mca")
        .context("MCA requires caller key_id profile")?;
    let alias = opt
        .get("key_id")
        .and_then(|v| v.as_str())
        .context("MCA requires key_id")?;
    let key = cx
        .keys
        .key(cx.client, alias)?
        .with_bytes(|b| -> Result<u8> {
            ensure!(b.len() == 1, "MCA key must be one byte");
            Ok(b[0])
        })?;
    let table = index(s, cx, base, n as u64 * 4)?;
    let mut rows = Vec::new();
    for p in table.chunks_exact(4) {
        let at = le32(p, 0)? as u64;
        let h = bytes(s, at, 32, 32)?;
        let kind = le32(&h, 0)?;
        ensure!(kind <= 1, "unknown MCA encryption type {kind}");
        let width = le32(&h, 12)?;
        let height = le32(&h, 16)?;
        let stored = le32(&h, 20)? as u64;
        let orig = le32(&h, 24)? as u64;
        let size = output_size(width, height, cx)?;
        let stride = (width as usize * 3 + 3) & !3;
        ensure!(
            orig == stride as u64 * height as u64 && orig <= cx.limits.max_entry_size,
            "MCA stride/size mismatch"
        );
        rows.push(Row::packed(
            format!("{:03}.png", rows.len()),
            at + 32,
            stored,
            size,
            decode(move |b, _| {
                let mut key = key;
                let mut d = Vec::with_capacity(b.len());
                for (i, v) in b.iter().enumerate() {
                    d.push(v.rotate_left(1) ^ key);
                    key = key.wrapping_add((b.len() - i) as u8);
                }
                if kind == 1 {
                    d = fc_lzss(&d, orig as usize)?;
                }
                ensure!(d.len() == orig as usize, "MCA decompressed size");
                let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
                for row in d.chunks_exact(stride) {
                    for p in row[..width as usize * 3].chunks_exact(3) {
                        rgba.extend([p[2], p[1], p[0], 255]);
                    }
                }
                encode_png_uncompressed(&ImageSurface {
                    width,
                    height,
                    rgba,
                })
            }),
        ));
    }
    checked_rows(s, cx, rows)
}
pub(super) fn dzi(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let b = index(s, cx, 0, length(s)?)?;
    let text = std::str::from_utf8(&b)?;
    let mut lines = text.lines();
    ensure!(lines.next() == Some("DZI"), "missing DZI");
    let pair = |line: &str| -> Result<(u32, u32)> {
        let (a, b) = line.split_once(',').context("DZI dimension pair")?;
        Ok((a.parse()?, b.parse()?))
    };
    let (width, height) = pair(lines.next().context("DZI dimensions")?)?;
    let size = output_size(width, height, cx)?;
    let n = lines.next().context("DZI frame count")?.parse::<usize>()?;
    limits(cx, n)?;
    let mut rows = Vec::new();
    for i in 0..n {
        let (cols, rows_n) = pair(lines.next().context("DZI grid")?)?;
        ensure!(
            cols as u64 * rows_n as u64 <= cx.limits.max_entries as u64,
            "DZI texture count limit"
        );
        let mut textures = Vec::new();
        for y in 0..rows_n {
            let line = lines.next().context("DZI texture row")?;
            let names: Vec<_> = line.split(',').collect();
            ensure!(names.len() >= cols as usize, "short DZI row");
            for x in 0..cols {
                let name = names[x as usize];
                if name.is_empty() {
                    continue;
                }
                ensure!(
                    !name.contains('/') && !name.contains('\\'),
                    "DZI texture name must be a basename"
                );
                let reader = cx
                    .open_sibling(&format!("tex/{name}.webp"))
                    .or_else(|_| cx.open_sibling(&format!("tex/{name}.png")))?;
                textures.push((x * 256, y * 256, reader));
            }
        }
        let limit = cx.limits.max_entry_size;
        rows.push(Row::packed(
            format!("{i:03}.png"),
            0,
            0,
            size,
            decode(move |_, _| {
                let mut dst = ImageSurface::filled(width, height, [0; 4])?;
                for (x, y, reader) in &textures {
                    let data = crysirror_resource::read_all(reader.as_ref(), limit)?;
                    let src = crysirror_image::decode_standard(&data)?;
                    blit(&mut dst, &src, *x as i32, *y as i32)?;
                }
                encode_png_uncompressed(&dst)
            }),
        ));
    }
    Ok(rows)
}
/// Cyberworks images use a caller-provided header permutation/delimiters. The
/// original title lookup table stays outside the engine and repository.
pub(super) fn cyber_image(b: &[u8], profile: &serde_json::Value) -> Result<ImageSurface> {
    ensure!(!b.is_empty(), "empty Cyberworks image");
    if b[0] == b'c' {
        return crysirror_image::decode_standard(
            b.get(5..).context("short Cyberworks PNG wrapper")?,
        );
    }
    ensure!(b[0] == b'a', "unknown Cyberworks image type");
    let p = profile
        .get("image")
        .context("Cyberworks image profile missing")?;
    let delim = p
        .get("delimiters")
        .and_then(|v| v.as_array())
        .context("Cyberworks image delimiters missing")?;
    ensure!(delim.len() == 3, "Cyberworks image requires 3 delimiters");
    let mut d = [0u8; 3];
    for i in 0..3 {
        let v = delim[i]
            .as_u64()
            .context("Cyberworks image delimiter integer")?;
        ensure!(v <= 255, "Cyberworks delimiter range");
        d[i] = v as u8;
    }
    let order = p
        .get("header_order")
        .and_then(|v| v.as_array())
        .context("Cyberworks image header_order missing")?;
    ensure!(order.len() <= 64, "Cyberworks header parameter limit");
    let mut c = Cursor::new(b.get(2..).context("short Cyberworks image")?);
    let mut params = std::collections::BTreeMap::new();
    for v in order {
        let i = v.as_u64().context("Cyberworks image parameter integer")?;
        let mut low = c.u8()?;
        if low == d[2] {
            low = 0;
        }
        let mut mid = 0u32;
        let mut high = 0u32;
        loop {
            let byte = c.u8()?;
            if byte == d[1] {
                break;
            }
            if byte == d[0] {
                high += 1;
                ensure!(high <= 255, "Cyberworks parameter overflow");
            } else if byte == d[2] {
                mid = 0;
            } else {
                mid = byte as u32;
            }
        }
        let value = high * d[0] as u32 * d[0] as u32 + mid * d[0] as u32 + low as u32;
        params.insert(i, value);
    }
    let param = |i| -> Result<u32> {
        params
            .get(&i)
            .copied()
            .context("Cyberworks required image parameter absent")
    };
    let width = param(4)?;
    let height = param(3)?;
    let len = rgba_len(width, height)?;
    let pixels = len / 4;
    let flags = param(0)?;
    let mut rgba = vec![0; len];
    let stride = (width as usize * 3 + 3) & !3;
    if flags == 0 {
        let size = param(5)? as usize;
        if size == pixels * 3 {
            for p in rgba.chunks_exact_mut(4) {
                let b = c.take(3)?;
                p.copy_from_slice(&[b[2], b[1], b[0], 255]);
            }
        } else if size == pixels {
            for p in rgba.chunks_exact_mut(4) {
                let v = c.u8()?;
                p.copy_from_slice(&[v, v, v, 255]);
            }
        } else if size == stride * height as usize {
            for y in 0..height as usize {
                let line = c.take(stride)?;
                for x in 0..width as usize {
                    rgba[(y * width as usize + x) * 4..(y * width as usize + x + 1) * 4]
                        .copy_from_slice(&[line[x * 3 + 2], line[x * 3 + 1], line[x * 3], 255]);
                }
            }
        } else {
            bail!("Cyberworks raw image size mismatch");
        }
    } else if flags & 6 == 2 {
        bail!("Cyberworks type a2 unsupported by pinned upstream");
    } else if flags & 6 == 6 {
        let map_size = param(7)? as usize;
        if map_size == 0 {
            for y in 0..height as usize {
                let line = c.take(stride)?;
                for x in 0..width as usize {
                    rgba[(y * width as usize + x) * 4 + 3] = line[x * 3];
                }
            }
            for y in 0..height as usize {
                let line = c.take(stride)?;
                for x in 0..width as usize {
                    let p = (y * width as usize + x) * 4;
                    rgba[p..p + 3].copy_from_slice(&[
                        line[x * 3 + 2],
                        line[x * 3 + 1],
                        line[x * 3],
                    ]);
                }
            }
        } else {
            let rgb_map = c.take(map_size)?;
            let alpha_map = c.take(map_size)?;
            let alpha = c.take(param(6)? as usize)?;
            let mut ai = 0;
            for i in 0..pixels {
                let rgb =
                    rgb_map.get(i / 8).context("Cyberworks RGB map short")? & (1 << (i % 8)) != 0;
                let a = alpha_map.get(i / 8).context("Cyberworks alpha map short")?
                    & (1 << (i % 8))
                    != 0;
                if rgb || a {
                    let b = c.take(3)?;
                    let av = if a {
                        let v = *alpha.get(ai).context("Cyberworks alpha plane short")?;
                        ai += 3;
                        v
                    } else {
                        255
                    };
                    rgba[i * 4..i * 4 + 4].copy_from_slice(&[b[2], b[1], b[0], av]);
                }
            }
        }
    } else {
        bail!("unknown Cyberworks image flags {flags}");
    }
    if p.get("flip_vertical")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let stride = width as usize * 4;
        let mut flipped = Vec::with_capacity(len);
        for line in rgba.chunks_exact(stride).rev() {
            flipped.extend(line);
        }
        rgba = flipped;
    }
    Ok(ImageSurface {
        width,
        height,
        rgba,
    })
}
