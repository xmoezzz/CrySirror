// SPDX-License-Identifier: GPL-3.0-or-later
// Image archive semantics ported from arc_unpacker (C) 2016 rr-.
use super::*;
use crysirror_image::ImageSurface;

fn png_size(cx: &DecodeContext, w: u32, h: u32) -> Result<u64> {
    let n = crysirror_image::png_uncompressed_len(w, h)? as u64;
    sized(cx, n)?;
    Ok(n)
}
fn png(w: u32, h: u32, rgba: Vec<u8>) -> Result<Vec<u8>> {
    crysirror_image::encode_png_uncompressed(&ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
fn bgr(data: &[u8], w: u32, h: u32, ch: u32, flip: bool) -> Result<Vec<u8>> {
    ensure!(ch == 3 || ch == 4, "unsupported image channel count");
    let n = crysirror_image::rgba_len(w, h)?;
    ensure!(data.len() >= n / 4 * ch as usize, "truncated pixel data");
    let mut out = vec![255; n];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let src = ((if flip { h as usize - 1 - y } else { y }) * w as usize + x) * ch as usize;
            let dst = (y * w as usize + x) * 4;
            out[dst..dst + 3].copy_from_slice(&[data[src + 2], data[src + 1], data[src]]);
            if ch == 4 {
                out[dst + 3] = data[src + 3]
            }
        }
    }
    Ok(out)
}
struct Scan<'a> {
    s: &'a dyn AssetReader,
    cx: &'a DecodeContext,
    at: usize,
    read: u64,
}
impl Scan<'_> {
    fn take(&mut self, n: usize) -> Result<Vec<u8>> {
        self.read = self
            .read
            .checked_add(n as u64)
            .context("image index overflow")?;
        ensure!(
            self.read <= self.cx.limits.max_index_size,
            "image index byte limit"
        );
        let b = bytes(
            self.s,
            self.at as u64,
            n as u64,
            self.cx.limits.max_index_size,
        )?;
        self.at = self
            .at
            .checked_add(n)
            .context("image index offset overflow")?;
        Ok(b)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        range(length(self.s)?, self.at as u64, n as u64)?;
        self.at = self
            .at
            .checked_add(n)
            .context("image payload offset overflow")?;
        Ok(())
    }
    fn le32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn le16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
}
fn animation_prefix(c: &mut Scan<'_>, v: u16) -> Result<usize> {
    if v < 20 {
        c.take(16)?;
        let n = c.le16()? as usize;
        c.take(2 + n * 4)?;
    } else if v < 100 {
        let n = c.le16()?;
        c.take(2)?;
        for _ in 0..n {
            match c.u8()? {
                0 => {}
                1 => {
                    c.take(8)?;
                }
                2..=5 => {
                    c.take(4)?;
                }
                _ => anyhow::bail!("unsupported Kaguya animation control"),
            }
        }
        let n = c.le16()? as usize;
        c.take(n * 8)?;
        if v == 21 {
            ensure!(c.take(7)? == b"[PIC]10", "invalid AN21 picture magic");
        }
    }
    let n = c.le16()? as usize;
    if v >= 100 || (v >= 20 && n > 0) {
        c.take(16)?;
    }
    Ok(n)
}
pub(super) fn animation(s: &dyn AssetReader, cx: &DecodeContext, v: u16) -> Result<Vec<Row>> {
    // Sequential picture formats interleave headers and pixel payloads; index
    // scanning is bounded independently from decoded-entry allocations.
    let mut c = Scan {
        s,
        cx,
        at: 4,
        read: 4,
    };
    let n = animation_prefix(&mut c, v)?;
    limits(cx, n)?;
    let mut rows = Vec::new();
    if n == 0 {
        return Ok(rows);
    }
    let mut base = None;
    for i in 0..n {
        if i == 0 || !(v == 21 || v == 110) {
            let x = c.le32()?;
            let y = c.le32()?;
            let w = c.le32()?;
            let h = c.le32()?;
            let ch = if v == 0 { 4 } else { c.le32()? };
            ensure!(ch == 3 || ch == 4, "unsupported Kaguya channels");
            let raw = u64::from(w) * u64::from(h) * u64::from(ch);
            sized(cx, raw)?;
            let size = png_size(cx, w, h)?;
            let start = c.at;
            c.skip(raw as usize)?;
            base = Some((x, y, w, h, ch, start, raw));
            let row = Row::packed(
                format!("{i}.png"),
                start as u64,
                raw,
                size,
                decode(move |b, _| png(w, h, bgr(b, w, h, ch, true)?)),
            )
            .meta(serde_json::json!({"x":x,"y":y,"width":w,"height":h}));
            rows.push(row);
        } else {
            let (x, y, w, h, ch, start, raw) = base.unwrap();
            let bands = c.u8()?;
            ensure!(bands > 0, "zero Kaguya RLE bands");
            let comp = c.le32()? as usize;
            c.skip(comp)?;
            let end = c.at;
            let size = png_size(cx, w, h)?;
            let delta_size =
                u64::from(ch) * (u64::from(x) + u64::from(w)) * (u64::from(y) + u64::from(h));
            ensure!(
                delta_size == raw,
                "Kaguya delta dimensions exceed base pixels"
            );
            rows.push(
                Row::packed(
                    format!("{i}.png"),
                    start as u64,
                    (end - start) as u64,
                    size,
                    decode(move |b, _| {
                        let mut c = Cursor::new(b);
                        let mut out = c.take(raw as usize)?.to_vec();
                        for _ in 0..i {
                            let bands = c.u8()? as usize;
                            let comp = c.le32()? as usize;
                            let delta = rle(c.take(comp)?, raw as usize, bands)?;
                            for (a, b) in out.iter_mut().zip(delta) {
                                *a = a.wrapping_add(b)
                            }
                        }
                        png(w, h, bgr(&out, w, h, ch, true)?)
                    }),
                )
                .meta(serde_json::json!({"x":x,"y":y,"width":w,"height":h})),
            );
        }
    }
    Ok(rows)
}
fn rle(b: &[u8], size: usize, bands: usize) -> Result<Vec<u8>> {
    ensure!(bands > 0 && bands <= size, "Kaguya RLE bands");
    let mut c = Cursor::new(b);
    let mut out = vec![0; size];
    for band in 0..bands {
        let mut p = band;
        let mut last = c.u8()?;
        out[p] = last;
        p += bands;
        while p < size {
            let mut v = c.u8()?;
            out[p] = v;
            p += bands;
            if last == v {
                let mut n = c.u8()? as usize;
                if n >= 128 {
                    n = ((n & 127) << 8) + c.u8()? as usize + 128
                }
                for _ in 0..n {
                    if p >= size {
                        break;
                    }
                    out[p] = v;
                    p += bands;
                }
                if p < size {
                    v = c.u8()?;
                    out[p] = v;
                    p += bands;
                }
            }
            last = v;
        }
    }
    Ok(out)
}
pub(super) fn cz10(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 4)? as usize;
    limits(cx, n)?;
    let mut at = 8 + n as u64 * 16;
    let mut rows = Vec::new();
    for (i, r) in index(s, cx, 8, n as u64 * 16)?.chunks_exact(16).enumerate() {
        let w = u16::from_le_bytes(r[..2].try_into().unwrap()) as u32;
        let h = u16::from_le_bytes(r[2..4].try_into().unwrap()) as u32;
        let stored = u64::from(le32(r, 8)?);
        let ch = le32(r, 12)?;
        ensure!(ch == 4, "CZ10 requires four channels");
        let size = png_size(cx, w, h)?;
        rows.push(Row::packed(
            format!("{i}.png"),
            at,
            stored,
            size,
            decode(move |b, _| {
                let stride = w as usize * 4;
                let mut c = Cursor::new(b);
                let mut last = vec![0u8; stride];
                let mut rgba = vec![0; crysirror_image::rgba_len(w, h)?];
                for y in 0..h as usize {
                    let control = c.u8()?;
                    let mut line = if control == 0 {
                        c.take(stride)?.to_vec()
                    } else {
                        ensure!(control <= 3, "CZ10 scanline control");
                        let n = u16::from_be_bytes(c.take(2)?.try_into().unwrap()) as usize;
                        inflate(c.take(n)?, stride)?
                    };
                    if control == 2 {
                        for x in 1..stride {
                            line[x] = line[x - 1].wrapping_sub(line[x]);
                        }
                    } else if control == 3 {
                        for (a, b) in line.iter_mut().zip(&last) {
                            *a = b.wrapping_sub(*a)
                        }
                    }
                    for channel in 0..4 {
                        for x in 0..w as usize {
                            rgba[(y * w as usize + x) * 4
                                + if channel < 3 { 2 - channel } else { channel }] =
                                line[channel * w as usize + x]
                        }
                    }
                    last = line;
                }
                png(w, h, rgba)
            }),
        ));
        at += stored;
    }
    Ok(rows)
}
fn gray16(data: &[u8], w: u32, h: u32, bpp: u16, flip: bool) -> Result<Vec<u8>> {
    ensure!(bpp == 8 || bpp == 16, "unsupported Leaf image depth");
    let n = crysirror_image::rgba_len(w, h)?;
    ensure!(
        data.len() >= n / 4 * bpp as usize / 8,
        "truncated Leaf pixels"
    );
    let mut out = vec![255; n];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let from = (if flip { h as usize - 1 - y } else { y }) * w as usize + x;
            let dst = (y * w as usize + x) * 4;
            if bpp == 8 {
                out[dst..dst + 3].fill(data[from])
            } else {
                let v = u16::from_le_bytes(data[from * 2..from * 2 + 2].try_into().unwrap());
                let expand = |v: u16| ((v & 31) << 3) as u8;
                out[dst] = expand(v >> 10);
                out[dst + 1] = expand(v >> 5);
                out[dst + 2] = expand(v);
                out[dst + 3] = if v & 0x8000 != 0 { 0 } else { 255 };
            }
        }
    }
    Ok(out)
}
pub(super) fn pak2_image(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let total = length(s)?;
    let mut at = 0;
    let mut specs: Vec<(u64, u64, u32, u32, u16, Option<u64>)> = Vec::new();
    while at < total {
        let tag = u32_at(s, at + 4)?;
        at += 8;
        match tag {
            0x756df85f => {
                let h = bytes(s, at, 32, 32)?;
                let bpp = u16::from_le_bytes(h[18..20].try_into().unwrap());
                let w = u16::from_le_bytes(h[28..30].try_into().unwrap()) as u32;
                let height = u16::from_le_bytes(h[30..32].try_into().unwrap()) as u32;
                at += 32;
                let size = u64::from(w) * u64::from(height) * u64::from(bpp) / 8;
                sized(cx, size)?;
                if w > 0 && height > 0 {
                    specs.push((at, size, w, height, bpp, None));
                    limits(cx, specs.len())?;
                }
                at += size;
            }
            0xa60dc503 => {
                at += 28;
                let p = specs.last_mut().context("Leaf mask has no image")?;
                p.5 = Some(at);
                at += u64::from(p.2) * u64::from(p.3);
            }
            0x4e4df6af => {
                at += 12;
                ensure!(at == total, "data after Leaf end marker");
            }
            _ => anyhow::bail!("unknown Leaf image chunk"),
        };
        ensure!(at <= total, "Leaf image outside source");
    }
    specs
        .into_iter()
        .enumerate()
        .map(|(i, (offset, size, w, h, bpp, mask))| {
            let stored = mask.map_or(size, |m| m + u64::from(w) * u64::from(h) - offset);
            let len = png_size(cx, w, h)?;
            Ok(Row::packed(
                format!("{i}.png"),
                offset,
                stored,
                len,
                decode(move |b, _| {
                    let mut rgba = gray16(b, w, h, bpp, true)?;
                    if let Some(mask) = mask {
                        let start = (mask - offset) as usize;
                        for (p, a) in rgba
                            .chunks_exact_mut(4)
                            .zip(&b[start..start + w as usize * h as usize])
                        {
                            p[3] = *a
                        }
                    }
                    png(w, h, rgba)
                }),
            ))
        })
        .collect()
}
pub(super) fn pak2_texture(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let all = window(s, cx, 24)?;
    let mut c = Cursor::new(&all);
    let n = c.le16()? as usize;
    let chunks = c.le16()? as usize;
    limits(cx, n)?;
    limits(cx, chunks)?;
    c.take(4)?;
    let mut last = c.le16()? as usize;
    let data = 24 + c.at as u64 + n as u64 * 2 + chunks as u64 * 36 + 36;
    let mut counts = Vec::new();
    for _ in 0..n {
        let next = c.le16()? as usize;
        counts.push(
            next.checked_sub(last)
                .context("Leaf chunk counts out of order")?,
        );
        last = next;
    }
    let mut rows = Vec::new();
    for count in counts {
        let mut parts = Vec::new();
        let (mut minx, mut miny, mut maxx, mut maxy) = (0i32, 0i32, 0i32, 0i32);
        for _ in 0..count {
            c.take(8)?;
            let x = c.le16()? as i16 as i32;
            let y = c.le16()? as i16 as i32;
            c.take(4)?;
            let w = c.le16()? as u32;
            let h = c.le16()? as u32;
            c.take(4)?;
            let at = data + u64::from(c.le32()?);
            c.take(8)?;
            range(length(s)?, at, u64::from(w) * u64::from(h) * 2)?;
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x + w as i32);
            maxy = maxy.max(y + h as i32);
            parts.push((x, y, w, h, at));
        }
        if parts.is_empty() {
            continue;
        }
        let w = (maxx - minx) as u32;
        let h = (maxy - miny) as u32;
        let size = png_size(cx, w, h)?;
        let start = parts.iter().map(|p| p.4).min().unwrap();
        let end = parts
            .iter()
            .map(|p| p.4 + u64::from(p.2) * u64::from(p.3) * 2)
            .max()
            .unwrap();
        let i = rows.len();
        rows.push(Row::packed(
            format!("{i}.png"),
            start,
            end - start,
            size,
            decode(move |b, _| {
                let mut rgba = vec![0; crysirror_image::rgba_len(w, h)?];
                for p in rgba.chunks_exact_mut(4) {
                    p[3] = 255
                }
                for &(x, y, pw, ph, at) in &parts {
                    let raw = gray16(&b[(at - start) as usize..], pw, ph, 16, true)?;
                    for yy in 0..ph {
                        for xx in 0..pw {
                            let (dx, dy) = (x + xx as i32, y + yy as i32);
                            if dx >= 0 && dy >= 0 && dx < w as i32 && dy < h as i32 {
                                let to = (dy as usize * w as usize + dx as usize) * 4;
                                let from = (yy as usize * pw as usize + xx as usize) * 4;
                                rgba[to..to + 4].copy_from_slice(&raw[from..from + 4]);
                            }
                        }
                    }
                }
                png(w, h, rgba)
            }),
        ));
    }
    Ok(rows)
}
pub(super) fn gxt(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(u32_at(s, 4)? == 0x10000003, "unsupported GXT version");
    let n = u32_at(s, 8)? as usize;
    limits(cx, n)?;
    index(s, cx, 32, n as u64 * 32)?
        .chunks_exact(32)
        .enumerate()
        .map(|(i, r)| {
            let at = u64::from(le32(r, 0)?);
            let stored = u64::from(le32(r, 4)?);
            let pal = le32(r, 8)?;
            let kind = le32(r, 16)?;
            let w = u16::from_le_bytes(r[24..26].try_into().unwrap()) as u32;
            let h = u16::from_le_bytes(r[26..28].try_into().unwrap()) as u32;
            let len = png_size(cx, w, h)?;
            let plan = if pal != u32::MAX {
                Plan::Unsupported("GXT paletted entries unsupported by pinned source")
            } else if kind != 0x60000000 {
                Plan::Unsupported("GXT nonlinear textures unsupported by pinned source")
            } else {
                decode(move |b, _| png(w, h, gray16(b, w, h, 8, false)?))
            };
            Ok(Row::packed(format!("{i}.png"), at, stored, len, plan))
        })
        .collect()
}

pub(super) fn px(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    use std::collections::BTreeSet;
    let mut pending = vec![(0u64, 0usize)];
    let mut seen = BTreeSet::new();
    let mut specs = Vec::new();
    let total = length(s)?;
    while let Some((at, depth)) = pending.pop() {
        ensure!(depth < 128, "PX nesting limit");
        if !seen.insert(at) {
            continue;
        }
        limits(cx, seen.len())?;
        let h = bytes(s, at, 32, 32)?;
        let control = u16::from_le_bytes(h[16..18].try_into().unwrap());
        let w = le32(&h, 20)?;
        let height = le32(&h, 24)?;
        if control & 0xc0 != 0 {
            let n = le32(&h, 0)? as usize;
            limits(cx, n)?;
            let base = at + 32 + n as u64 * 4;
            let offsets = index(s, cx, at + 32, n as u64 * 4)?
                .chunks_exact(4)
                .map(|r| Ok(base + u64::from(le32(r, 0)?)))
                .collect::<Result<Vec<_>>>()?;
            if control & 0x80 != 0 {
                for o in offsets.into_iter().rev() {
                    pending.push((o, depth + 1));
                }
            } else if !offsets.is_empty() {
                specs.push((w, height, offsets));
            }
        } else {
            specs.push((w, height, vec![at]));
        }
        limits(cx, specs.len())?;
    }
    specs
        .into_iter()
        .enumerate()
        .map(|(i, (w, h, offsets))| {
            let len = png_size(cx, w, h)?;
            Ok(Row::packed(
                format!("{i}.png"),
                0,
                total,
                len,
                decode(move |b, _| {
                    let mut out = vec![0u32; crysirror_image::rgba_len(w, h)? / 4];
                    let mut pal = [0u32; 256];
                    for at in &offsets {
                        px_block(b, *at as usize, w, h, &mut pal, &mut out)?;
                    }
                    let mut rgba = Vec::with_capacity(out.len() * 4);
                    for v in out {
                        let p = v.to_le_bytes();
                        rgba.extend([p[2], p[1], p[0], p[3]]);
                    }
                    png(w, h, rgba)
                }),
            ))
        })
        .collect()
}
fn alpha(v: u32, zero: bool) -> u32 {
    if v & 0xff000000 != 0 {
        (v & 0xffffff) | ((v >> 23).wrapping_add(255) << 24)
    } else if zero {
        0
    } else {
        v | 0xff000000
    }
}
fn px_block(
    b: &[u8],
    at: usize,
    iw: u32,
    ih: u32,
    pal: &mut [u32; 256],
    out: &mut [u32],
) -> Result<()> {
    let mut c = Cursor::new(b);
    c.at = at;
    let w = c.le32()? as usize;
    let h = c.le32()? as usize;
    let x = c.le32()? as i32;
    let y = c.le32()? as i32;
    let kind = c.le16()?;
    let subtype = c.le16()?;
    c.take(12)?;
    if kind == 0 {
        for p in pal {
            *p = c.le32()?
        }
        return Ok(());
    }
    if kind == 1 || kind == 7 {
        let mut own = [0u32; 256];
        if kind == 7 {
            for p in &mut own {
                *p = c.le32()?
            }
        } else {
            ensure!(subtype == 8 || subtype == 32, "unknown PX direct subtype");
        }
        let pixels = w.checked_mul(h).context("PX pixel count overflow")?;
        ensure!(pixels <= out.len(), "PX direct image size");
        for p in &mut out[..pixels] {
            *p = if kind == 7 {
                alpha(own[c.u8()? as usize], false)
            } else if subtype == 8 {
                let v = u32::from(c.u8()?);
                0xff000000 | v * 0x10101
            } else {
                alpha(c.le32()?, false)
            }
        }
        return Ok(());
    }
    ensure!(
        kind == 4 && [8, 9, 32, 48].contains(&subtype),
        "unknown PX block type"
    );
    ensure!(w <= 1024 && h <= 1024, "PX block dimensions");
    let mut block = vec![0u32; 1024 * 1024];
    let mut pos = 0usize;
    let mut use_alpha = true;
    let mut commands = 0usize;
    loop {
        commands += 1;
        ensure!(commands <= b.len() / 4 + 1, "PX command limit");
        let control = c.le32()?;
        if control == u32::MAX {
            break;
        }
        let count;
        if subtype == 9 {
            if control & 0x180000 != 0 {
                use_alpha = !use_alpha
            }
            pos = pos
                .checked_add((control as usize & 0x1ff) * 1024)
                .and_then(|p| p.checked_add_signed(((control as i32) >> 21) as isize))
                .context("PX block offset overflow")?;
            count = ((control >> 9) % 1024) as usize;
        } else {
            if (subtype == 32 || subtype == 48) && control & 0xff000000 != 0 {
                continue;
            }
            pos = pos
                .checked_add(control as usize / 4)
                .context("PX block offset overflow")?;
            c.take(4)?;
            count = c.le32()? as usize;
        }
        ensure!(count <= b.len(), "PX block run length");
        for _ in 0..count {
            let v = match subtype {
                8 => pal[c.u8()? as usize] | 0xff000000,
                9 => {
                    let a = if use_alpha {
                        u32::from(c.u8()?).wrapping_shl(25).wrapping_sub(0x1000000)
                    } else {
                        0xff000000
                    };
                    pal[c.u8()? as usize] | a
                }
                32 => alpha(c.le32()?, false),
                48 => {
                    let v = c.le32()?;
                    c.take(2)?;
                    alpha(v, true)
                }
                _ => unreachable!(),
            };
            if let Some(dst) = block.get_mut(pos) {
                *dst = v
            }
            pos = pos.checked_add(1).context("PX block cursor overflow")?;
        }
    }
    for yy in 0..h {
        for xx in 0..w {
            let (dx, dy) = (x as i64 + xx as i64, y as i64 + yy as i64);
            if dx >= 0 && dy >= 0 && dx < i64::from(iw) && dy < i64::from(ih) {
                out[dy as usize * iw as usize + dx as usize] = block[yy * 1024 + xx]
            }
        }
    }
    Ok(())
}
