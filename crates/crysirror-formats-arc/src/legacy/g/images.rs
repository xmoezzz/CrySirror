// SPDX-License-Identifier: GPL-3.0-or-later
// Native image-resource archives adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
use crysirror_image::{encode_png_uncompressed, png_uncompressed_len, rgba_len, ImageSurface};
#[derive(Clone)]
struct Wip {
    w: u32,
    h: u32,
    depth: u16,
    offset: u64,
    packed: u64,
}
fn wip_table(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Wip>> {
    let h = bytes(s, 0, 8, 8)?;
    ensure!(h.starts_with(b"WIPF"), "WIPF signature");
    let mut c = Cursor::new(&h[4..]);
    let n = c.le16()? as usize;
    let depth = c.le16()?;
    ensure!(depth == 8 || depth == 24, "WIPF depth");
    limits(cx, n)?;
    let table = bytes(s, 8, n as u64 * 24, cx.limits.max_index_size)?;
    let mut c = Cursor::new(&table);
    let mut offset = 8 + table.len() as u64;
    let mut rows = Vec::new();
    for _ in 0..n {
        let w = c.le32()?;
        let h = c.le32()?;
        rgba_len(w, h)?;
        c.take(12)?;
        let packed = c.le32()? as u64;
        rows.push(Wip {
            w,
            h,
            depth,
            offset,
            packed,
        });
        offset = offset
            .checked_add(packed + if depth == 8 { 1024 } else { 0 })
            .context("WIPF offset overflow")?;
    }
    range(length(s)?, 0, offset)?;
    Ok(rows)
}
fn wip_image(d: &[u8], w: &Wip) -> Result<ImageSurface> {
    let pixels = rgba_len(w.w, w.h)? / 4;
    let raw_size = pixels * (w.depth / 8) as usize;
    let mut c = Cursor::new(if w.depth == 8 {
        d.get(1024..).context("WIPF palette")?
    } else {
        d
    });
    let mut raw = vec![0; raw_size];
    let mut dict = [0u8; 4096];
    let mut write = 1;
    let mut at = 0;
    let mut control = 0u16;
    while at < raw_size {
        control >>= 1;
        if control & 256 == 0 {
            control = c.u8()? as u16 | 0xff00;
        }
        if control & 1 != 0 {
            let b = c.u8()?;
            raw[at] = b;
            at += 1;
            dict[write] = b;
            write = (write + 1) & 4095;
        } else {
            let a = c.u8()? as usize;
            let b = c.u8()? as usize;
            let mut read = (a << 4) | (b >> 4);
            for _ in 0..((b & 15) + 2).min(raw_size - at) {
                let v = dict[read];
                read = (read + 1) & 4095;
                raw[at] = v;
                at += 1;
                dict[write] = v;
                write = (write + 1) & 4095;
            }
        }
    }
    let mut rgba = Vec::with_capacity(pixels * 4);
    if w.depth == 8 {
        for i in raw {
            let p = i as usize * 4;
            rgba.extend_from_slice(&[d[p + 2], d[p + 1], d[p], d[p + 3] ^ 255]);
        }
    } else {
        for i in 0..pixels {
            rgba.extend_from_slice(&[raw[pixels * 2 + i], raw[pixels + i], raw[i], 255]);
        }
    }
    Ok(ImageSurface {
        width: w.w,
        height: w.h,
        rgba,
    })
}
pub(super) fn wipf(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let entries = wip_table(s, cx)?;
    let mut mask = None;
    let opt = options(cx, "will/wipf");
    if let Some(path) = opt.and_then(|v| v.get("mask")).and_then(|v| v.as_str()) {
        mask = Some(cx.open_sibling(path)?);
    } else if cx.resolver.is_some()
        && opt.and_then(|v| v.get("mask")).and_then(|v| v.as_bool()) != Some(false)
    {
        if let Some(name) = cx.source_name.as_deref() {
            if !has_ext(name, "msk") {
                let filename = name.rsplit('/').next().unwrap_or(name);
                let stem = filename.rsplit_once('.').map(|v| v.0).unwrap_or(filename);
                mask = cx.open_sibling(&format!("{stem}.msk")).ok();
            }
        }
    }
    let masks = if let Some(s) = &mask {
        let m = wip_table(s.as_ref(), cx)?;
        ensure!(m.len() >= entries.len(), "WIPF mask entry count");
        Some(m)
    } else {
        None
    };
    let mut rows = Vec::new();
    for (i, w) in entries.into_iter().enumerate() {
        let n = png_uncompressed_len(w.w, w.h)? as u64;
        ensure!(n <= cx.limits.max_entry_size, "WIPF PNG output limit");
        let m = if let (Some(source), Some(masks)) = (&mask, &masks) {
            let info = masks[i].clone();
            ensure!(info.w == w.w && info.h == w.h, "WIPF mask dimensions");
            let data = bytes(
                source.as_ref(),
                info.offset,
                info.packed + if info.depth == 8 { 1024 } else { 0 },
                cx.limits.max_entry_size,
            )?;
            Some((info, data))
        } else {
            None
        };
        let offset = w.offset;
        let packed = w.packed + if w.depth == 8 { 1024 } else { 0 };
        rows.push(Row::packed(
            format!("{i:05}.png"),
            offset,
            packed,
            n,
            decode(move |b, _| {
                let mut image = wip_image(b, &w)?;
                if let Some((info, data)) = &m {
                    let alpha = wip_image(data, info)?;
                    image.apply_channel_mask(&alpha, 0)?;
                }
                encode_png_uncompressed(&image)
            }),
        ));
    }
    Ok(rows)
}
fn hex(d: &[u8]) -> Result<u64> {
    ensure!(
        d.iter().all(u8::is_ascii_hexdigit),
        "ODN hexadecimal offset"
    );
    Ok(u64::from_str_radix(std::str::from_utf8(d)?, 16)?)
}
fn rle_len(d: &[u8], chunk: usize, limit: usize) -> Result<usize> {
    let mut c = Cursor::new(d);
    let mut n = 0usize;
    while c.left() > 0 {
        let flags = c.u8()?;
        for i in 0..8 {
            if c.left() == 0 {
                break;
            }
            c.take(chunk)?;
            let repeat = if flags & (1 << i) != 0 {
                u16::from_be_bytes(c.take(2)?.try_into().unwrap()) as usize + 1
            } else {
                1
            };
            n = n
                .checked_add(repeat * chunk)
                .context("ODN decoded size overflow")?;
            ensure!(n <= limit, "ODN decoded byte limit");
        }
    }
    Ok(n)
}
fn rle(d: &[u8], chunk: usize, n: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(d);
    let mut o = Vec::with_capacity(n);
    while c.left() > 0 {
        let flags = c.u8()?;
        for i in 0..8 {
            if c.left() == 0 {
                break;
            }
            let pixel = c.take(chunk)?;
            let repeat = if flags & (1 << i) != 0 {
                u16::from_be_bytes(c.take(2)?.try_into().unwrap()) as usize + 1
            } else {
                1
            };
            ensure!(repeat * chunk <= n - o.len(), "ODN repetition bounds");
            for _ in 0..repeat {
                o.extend_from_slice(pixel);
            }
        }
    }
    ensure!(o.len() == n, "ODN decoded length");
    Ok(o)
}
pub(super) fn odn(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 32, 32)?;
    let variant = options(cx, "valkyria/odn")
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(if &h[8..16] == b"00000000" {
            1
        } else if h[..4] == h[16..20] {
            2
        } else if h[..4] == h[24..28] {
            3
        } else {
            0
        });
    ensure!((1..=3).contains(&variant), "unknown ODN layout");
    let mut at = 0u64;
    let table_end = if variant == 1 {
        cx.limits.max_index_size.min(length(s)?)
    } else {
        hex(&h[8..16])?
    };
    ensure!(table_end <= cx.limits.max_index_size, "ODN table limit");
    let mut entries = Vec::new();
    let mut ended = false;
    while at < table_end {
        limits(cx, entries.len() + 1)?;
        let record = bytes(s, at, if variant == 3 { 24 } else { 16 }, 24)?;
        at += record.len() as u64;
        let path = ascii(&record[..8])?;
        if variant == 1 && path.starts_with("END_") {
            ended = true;
            break;
        }
        let offset = hex(&record[8..16])?;
        entries.push((path, offset));
    }
    if variant == 1 {
        ensure!(ended, "ODN missing END marker");
        for e in &mut entries {
            e.1 = e.1.checked_add(at).context("ODN data offset")?;
        }
    }
    let mut rows = Vec::new();
    for i in 0..entries.len() {
        let (path, offset) = entries[i].clone();
        let end = entries.get(i + 1).map(|e| e.1).unwrap_or(length(s)?);
        let stored = end.checked_sub(offset).context("ODN offsets not sorted")?;
        range(length(s)?, offset, stored)?;
        let chunk = if path.starts_with("back") {
            3
        } else if path.starts_with("codn") || path.starts_with("cccc") {
            4
        } else {
            0
        };
        if chunk == 0 {
            rows.push(Row::raw(path, offset, stored));
            continue;
        }
        let input = bytes(s, offset, stored, cx.limits.max_entry_size)?;
        let n = rle_len(&input, chunk, cx.limits.max_entry_size as usize)?;
        let dims = [(1024, 768), (400, 200), (800, 600), (640, 480)]
            .into_iter()
            .find(|(w, h)| n == w * h * chunk);
        let size = if let Some((w, h)) = dims {
            png_uncompressed_len(w as u32, h as u32)?
        } else {
            n
        };
        ensure!(
            size as u64 <= cx.limits.max_entry_size,
            "ODN final output limit"
        );
        let path = if dims.is_some() {
            format!("{path}.png")
        } else {
            path
        };
        rows.push(Row::packed(
            path,
            offset,
            stored,
            size as u64,
            decode(move |d, _| {
                let raw = rle(d, chunk, n)?;
                if let Some((w, h)) = dims {
                    let mut rgba = vec![0; w * h * 4];
                    for y in 0..h {
                        for x in 0..w {
                            let p = (y * w + x) * chunk;
                            let target = ((h - y - 1) * w + x) * 4;
                            rgba[target..target + 4].copy_from_slice(&[
                                raw[p + 2],
                                raw[p + 1],
                                raw[p],
                                if chunk == 4 { raw[p + 3] } else { 255 },
                            ]);
                        }
                    }
                    encode_png_uncompressed(&ImageSurface {
                        width: w as u32,
                        height: h as u32,
                        rgba,
                    })
                } else {
                    Ok(raw)
                }
            }),
        ));
    }
    Ok(rows)
}
