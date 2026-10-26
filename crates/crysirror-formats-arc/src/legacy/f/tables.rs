// SPDX-License-Identifier: GPL-3.0-or-later
// Ported from arc_unpacker (C) 2016 rr-.
use super::*;

pub(super) fn probe_link(s: &dyn AssetReader) -> Result<bool> {
    if !magic(s, 0, b"LINK")? {
        return Ok(false);
    }
    let n = u32_at(s, 4)? as u64;
    if n == 0 {
        return Ok(false);
    }
    let at = 12 + u64::from(u32_at(s, 8)?) + (n - 1) * 8;
    Ok(u64::from(u32_at(s, at)?) + u64::from(u32_at(s, at + 4)?) == length(s)?)
}
pub(super) fn link(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 4)? as usize;
    limits(cx, n)?;
    let names_size = u32_at(s, 8)? as u64;
    let names = index(s, cx, 12, names_size)?;
    let mut names = Cursor::new(&names);
    let table = index(s, cx, 12 + names_size, n as u64 * 8)?;
    let mut rows = Vec::new();
    for r in table.chunks_exact(8) {
        let path = sjis(names.zero()?)?;
        let at = u64::from(le32(r, 0)?);
        let size = u64::from(le32(r, 4)?);
        if ["cg_", "bg_", "sp_"].iter().any(|e| has_ext(&path, e)) {
            ensure!(size >= 4, "truncated LINK compressed data");
            let out = u64::from(u32_at(s, at)?);
            rows.push(Row::packed(
                path,
                at + 4,
                size - 4,
                out,
                decode(|b, e| kaguya_lzss(b, e.size as usize)),
            ));
        } else {
            rows.push(Row::raw(path, at, size))
        }
    }
    Ok(rows)
}
pub(super) fn kaguya_lzss(b: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(b);
    let mut ring = [0u8; 256];
    let mut write = 0xefusize;
    let (mut control, mut flip, mut aux) = (0u32, false, 0u8);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        control <<= 1;
        if control & 0x10000 == 0 {
            control = u32::from(c.u8()?) | 0xff00;
        }
        if control & 0x80 != 0 {
            let v = c.u8()?;
            out.push(v);
            ring[write] = v;
            write = (write + 1) & 255;
        } else {
            let mut read = c.u8()? as usize;
            let count = if flip {
                aux >> 4
            } else {
                aux = c.u8()?;
                aux & 15
            } as usize
                + 2;
            flip = !flip;
            for _ in 0..count.min(size - out.len()) {
                let v = ring[read];
                read = (read + 1) & 255;
                out.push(v);
                ring[write] = v;
                write = (write + 1) & 255;
            }
        }
    }
    Ok(out)
}
pub(super) fn link2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let table = window(s, cx, 8)?;
    let mut c = Cursor::new(&table);
    let n = u32_at(s, 4)? as usize;
    limits(cx, n)?;
    let mut rows = Vec::new();
    for _ in 0..n {
        let len = c.le16()? as usize;
        let name: Vec<_> = c.take(len)?.iter().map(|b| b ^ 255).collect();
        let path = sjis(zstr(&name))?;
        let at = u64::from(c.le32()?);
        let size = u64::from(c.le32()?);
        let kind = c.le16()?;
        rows.push(if kind == 1 {
            ensure!(size >= 4, "truncated LIN2 data");
            Row::packed(
                path,
                at + 4,
                size - 4,
                u32_at(s, at)?.into(),
                decode(|b, e| kaguya_lzss(b, e.size as usize)),
            )
        } else {
            Row::raw(path, at, size)
        });
    }
    Ok(rows)
}
pub(super) fn linkn(s: &dyn AssetReader, cx: &DecodeContext, version: u8) -> Result<Vec<Row>> {
    let mut at = match version {
        3 => 8,
        4 | 5 => 10,
        6 => 8 + u64::from(bytes(s, 7, 1, 1)?[0]),
        _ => unreachable!(),
    };
    let mut rows = Vec::new();
    loop {
        let total = u64::from(u32_at(s, at)?);
        if total == 0 {
            break;
        }
        let h = index(s, cx, at, total.min(cx.limits.max_index_size))?;
        let mut c = Cursor::new(&h);
        c.take(4)?;
        let flags = c.le16()?;
        c.take(7)?;
        let path = if version == 6 {
            let n = c.le16()? as usize;
            utf16(c.take(n)?)?
        } else {
            let n = c.u8()? as usize;
            c.take(2)?;
            sjis(c.take(n)?)?
        };
        let used = c.at as u64;
        let size = total.checked_sub(used).context("LINK record size")?;
        let plan = if flags & 4 != 0 {
            super::crypt::kaguya_plan(cx, &format!("kaguya/link{version}"))?
        } else {
            Plan::Raw
        };
        rows.push(
            Row::packed(path, at + used, size, size, plan).meta(serde_json::json!({"flags":flags})),
        );
        limits(cx, rows.len())?;
        at = at.checked_add(total).context("LINK offset overflow")?;
    }
    Ok(rows)
}
pub(super) fn wfl(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let mut at = 4;
    let size = length(s)?;
    let mut rows = Vec::new();
    while size - at >= 4 {
        let n = u64::from(u32_at(s, at)?);
        at += 4;
        if n == 0 {
            break;
        }
        let mut name = index(s, cx, at, n)?;
        name.iter_mut().for_each(|b| *b ^= 255);
        at += n;
        let path = sjis(zstr(&name))?;
        let kind = u16_at(s, at)?;
        let comp = u64::from(u32_at(s, at + 2)?);
        at += 6;
        let orig = if kind == 1 {
            let n = u64::from(u32_at(s, at)?);
            at += 4;
            n
        } else {
            comp
        };
        let plan = if kind == 1 {
            decode(|b, e| bit_lzss(b, e.size as usize))
        } else {
            Plan::Raw
        };
        rows.push(Row::packed(path, at, comp, orig, plan));
        range(size, at, comp)?;
        at += comp;
        limits(cx, rows.len())?;
    }
    Ok(rows)
}
fn bit_lzss(b: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut bit = 0;
    let mut get = |n: usize| -> Result<usize> {
        let mut v = 0;
        for _ in 0..n {
            let b = *b.get(bit / 8).context("truncated WFL LZSS")?;
            v = (v << 1) | usize::from((b >> (7 - bit % 8)) & 1);
            bit += 1;
        }
        Ok(v)
    };
    let mut ring = [0u8; 4096];
    let mut wp = 1;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        if get(1)? != 0 {
            let v = get(8)? as u8;
            out.push(v);
            ring[wp] = v;
            wp = (wp + 1) & 4095;
        } else {
            let mut rp = get(12)?;
            let n = get(4)? + 2;
            for _ in 0..n.min(size - out.len()) {
                let v = ring[rp];
                rp = (rp + 1) & 4095;
                out.push(v);
                ring[wp] = v;
                wp = (wp + 1) & 4095;
            }
        }
    }
    Ok(out)
}

pub(super) fn leaf_a(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u16_at(s, 2)? as usize;
    limits(cx, n)?;
    let base = 4 + n as u64 * 32;
    index(s, cx, 4, n as u64 * 32)?
        .chunks_exact(32)
        .map(|r| {
            let path = sjis(zstr(&r[..23]))?;
            let flag = r[23];
            let comp = u64::from(le32(r, 24)?);
            let at = base + u64::from(le32(r, 28)?);
            if flag == 0 {
                return Ok(Row::raw(path, at, comp));
            }
            ensure!(comp >= 4, "leaf/a compressed size");
            let orig = u64::from(u32_at(s, at)?);
            let plan = if flag == 3 || flag == 5 {
                Plan::Unsupported("leaf/a type 1/2 encryption is unsupported by pinned source")
            } else {
                decode(move |b, e| {
                    let mut out = lzss(b, e.size as usize)?;
                    if (0x7f..=0x89).contains(&flag) && out.len() >= 32 {
                        let pixels = u64::from(le32(&out, 0)?) * u64::from(le32(&out, 4)?);
                        if 32 + pixels * 4 <= out.len() as u64 {
                            let mut acc = [0u8; 3];
                            for p in out[32..32 + pixels as usize * 4].chunks_exact_mut(4) {
                                for j in 0..3 {
                                    acc[j] = acc[j]
                                        .wrapping_add(p[3])
                                        .wrapping_add(p[j])
                                        .wrapping_sub(flag & 15);
                                    p[j] = acc[j]
                                }
                                p[3] = 0;
                            }
                        }
                    }
                    Ok(out)
                })
            };
            Ok(Row::packed(path, at + 4, comp - 4, orig, plan))
        })
        .collect()
}
pub(super) fn ar10(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 4)? as usize;
    limits(cx, n)?;
    let base = u64::from(u32_at(s, 8)?);
    let key = bytes(s, 12, 1, 1)?[0];
    let t = window(s, cx, 13)?;
    let mut c = Cursor::new(&t);
    let mut rows = Vec::new();
    for _ in 0..n {
        let path = sjis(c.zero()?)?;
        let at = base + u64::from(c.le32()?);
        let len = u64::from(u32_at(s, at)?);
        let klen = bytes(s, at + 4, 1, 1)?[0] ^ key;
        ensure!(klen > 0 || len == 0, "ar10 empty XOR key");
        let k = index(s, cx, at + 5, klen.into())?;
        rows.push(Row::packed(
            path,
            at + 5 + u64::from(klen),
            len,
            len,
            decode(move |b, _| {
                Ok(b.iter()
                    .enumerate()
                    .map(|(i, b)| b ^ k[i % k.len()])
                    .collect())
            }),
        ));
    }
    Ok(rows)
}
pub(super) fn lac(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 4)? as usize;
    limits(cx, n)?;
    index(s, cx, 8, n as u64 * 40)?
        .chunks_exact(40)
        .map(|r| {
            let name: Vec<_> = zstr(&r[..32]).iter().map(|b| b ^ 255).collect();
            Ok(Row::raw(
                sjis(&name)?,
                le32(r, 36)?.into(),
                le32(r, 32)?.into(),
            ))
        })
        .collect()
}
fn packed_header(s: &dyn AssetReader, path: String, at: u64, size: u64, plan: Plan) -> Result<Row> {
    ensure!(size >= 8, "compressed entry header size");
    let stored = u64::from(u32_at(s, at)?);
    ensure!(
        stored >= 8 && stored <= size,
        "compressed entry stored size"
    );
    Ok(Row::packed(
        path,
        at + 8,
        stored - 8,
        u32_at(s, at + 4)?.into(),
        plan,
    ))
}
pub(super) fn kcap(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 4)? as usize;
    limits(cx, n)?;
    if n == 0 {
        return Ok(vec![]);
    }
    let total = length(s)?;
    let mut version = 0;
    for (v, sz) in [(1, 32), (2, 36)] {
        let at = 8 + (n as u64 - 1) * sz + sz - 8;
        if at + 8 <= total && u64::from(u32_at(s, at)?) + u64::from(u32_at(s, at + 4)?) == total {
            version = v
        }
    }
    ensure!(version != 0, "unknown KCAP layout");
    let sz = if version == 1 { 32 } else { 36 };
    index(s, cx, 8, n as u64 * sz)?
        .chunks_exact(sz as usize)
        .filter_map(|r| {
            let f = if version == 1 { 1 } else { le32(r, 0).unwrap() };
            let r = if version == 1 { r } else { &r[4..] };
            if f > 1 && le32(r, 28).unwrap() == 0 {
                return None;
            }
            Some((|| {
                ensure!(f <= 1, "unknown KCAP entry type");
                let path = sjis(zstr(&r[..24]))?;
                let at = u64::from(le32(r, 24)?);
                let size = u64::from(le32(r, 28)?);
                if f == 0 {
                    Ok(Row::raw(path, at, size))
                } else {
                    packed_header(s, path, at, size, Plan::Lzss(serde_json::Value::Null))
                }
            })())
        })
        .collect()
}
pub(super) fn pak1(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 0)? as usize;
    limits(cx, n)?;
    let mut rows = Vec::new();
    for r in index(s, cx, 4, n as u64 * 28)?.chunks_exact(28) {
        let size = u64::from(le32(r, 16)?);
        if size == 0 {
            continue;
        }
        let path = sjis(zstr(&r[..16]))?;
        let at = u64::from(le32(r, 24)?);
        let compressed = le32(r, 20)? > 0;
        rows.push(if !compressed {
            Row::raw(path, at, size)
        } else {
            let p = profile(cx, "leaf/pak1")?;
            let version = p["version"]
                .as_u64()
                .context("leaf/pak1 profile requires version 1 or 2")?;
            ensure!(
                version == 1 || version == 2,
                "leaf/pak1 version must be 1 or 2"
            );
            packed_header(
                s,
                path,
                at,
                size,
                decode(move |b, e| {
                    pak1_lzss(b, e.size as usize, if version == 1 { 4096 } else { 2048 })
                }),
            )?
        });
    }
    Ok(rows)
}
fn pak1_lzss(b: &[u8], size: usize, cap: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(b);
    let mut ring = vec![0u8; cap];
    let (mut wp, mut used, mut control) = (0, 0, 0u16);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        control >>= 1;
        if control & 0x100 == 0 {
            control = u16::from(c.u8()?) | 0xff00
        }
        if control & 1 != 0 {
            let v = c.u8()?;
            out.push(v);
            ring[wp] = v;
            wp = (wp + 1) % cap;
            used = (used + 1).min(cap);
        } else {
            let v = c.le16()?;
            let mut rp = (v >> 4) as usize;
            let mut n = (v & 15) as usize;
            if n == 15 {
                n += c.u8()? as usize
            }
            n += 3;
            ensure!(
                used > 0 && rp < cap && out.len() + n <= size,
                "PAK1 dictionary reference"
            );
            let start = out.len();
            for _ in 0..n {
                out.push(ring[rp]);
                rp = (rp + 1) % used;
            }
            for v in &out[start..] {
                ring[wp] = *v;
                wp = (wp + 1) % cap;
                used = (used + 1).min(cap);
            }
        }
    }
    Ok(out)
}
pub(super) fn pak2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u16_at(s, 0x1c)? as usize;
    limits(cx, n)?;
    let base = 0x20 + n as u64 * 32;
    let mut rows = Vec::new();
    for r in index(s, cx, 0x20, n as u64 * 32)?.chunks_exact(32) {
        let name: Vec<_> = r[2..14].iter().map(|b| b.rotate_left(4)).collect();
        let name = zstr(&name);
        if !name.is_empty() {
            rows.push(Row::raw(
                sjis(name)?,
                base + u64::from(le32(r, 16)?),
                le32(r, 24)?.into(),
            ))
        }
    }
    Ok(rows)
}

pub(super) fn nekopack4(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let t = window(s, cx, 14)?;
    let mut c = Cursor::new(&t);
    let mut rows = Vec::new();
    loop {
        let n = c.le32()? as usize;
        if n == 0 {
            break;
        }
        let name = zstr(c.take(n)?);
        let key = name.iter().fold(0u32, |a, b| a.wrapping_add(u32::from(*b)));
        let at = u64::from(c.le32()? ^ key);
        let size = u64::from(c.le32()? ^ key);
        ensure!(size >= 4, "NEKOPACK compressed size");
        let orig = u32_at(s, at + size - 4)?;
        rows.push(Row::packed(
            sjis(name)?,
            at,
            size - 4,
            orig.into(),
            decode(move |b, e| {
                let mut b = b.to_vec();
                let mut key = ((size >> 3) + 0x22) as u8;
                for v in &mut b {
                    *v ^= key;
                    key = key.wrapping_shl(3);
                    if key == 0 {
                        break;
                    }
                }
                inflate(&b, e.size as usize)
            }),
        ));
        limits(cx, rows.len())?;
    }
    Ok(rows)
}
pub(super) fn gamedat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let v = bytes(s, 11, 1, 1)?[0];
    ensure!(v == b'K' || v == b'2', "unknown GAMEDAT version");
    let n = u32_at(s, 12)? as usize;
    limits(cx, n)?;
    let ns = if v == b'K' { 16 } else { 32 };
    let t = index(s, cx, 16, n as u64 * (ns + 8))?;
    let data = 16 + n as u64 * (ns + 8);
    (0..n)
        .map(|i| {
            let path = sjis(zstr(&t[i * ns as usize..(i + 1) * ns as usize]))?;
            let r = n * ns as usize + i * 8;
            let size = u64::from(le32(&t, r + 4)?);
            Ok(Row::packed(
                path,
                data + u64::from(le32(&t, r)?),
                size,
                size,
                decode(|b, _| {
                    let mut out = b.to_vec();
                    if out.starts_with(b"\x95\x6b\x3c\x9d\x63") {
                        let mut k = 0xc5u8;
                        for b in &mut out {
                            *b ^= k;
                            k = k.wrapping_add(0x5c)
                        }
                    }
                    Ok(out)
                }),
            ))
        })
        .collect()
}
pub(super) fn probe_gpda(s: &dyn AssetReader) -> Result<bool> {
    Ok(magic(s, 0, b"GPDA")? && u64::from(u32_at(s, 4)?) == length(s)?)
}
pub(super) fn gpda(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 12)? as usize;
    limits(cx, n)?;
    let mut rows = Vec::new();
    for r in index(s, cx, 16, n as u64 * 16)?.chunks_exact(16) {
        ensure!(le32(r, 4)? == 0, "GPDA reserved field");
        let size = u64::from(le32(r, 8)?);
        if size == 0 {
            continue;
        }
        let at = u64::from(le32(r, 12)?);
        let path = if at == 0 {
            String::new()
        } else {
            sjis(&index(s, cx, at + 4, u32_at(s, at)?.into())?)?
        };
        rows.push(Row::raw(path, le32(r, 0)?.into(), size));
    }
    name_unnamed(&mut rows);
    Ok(rows)
}
pub(super) fn mgr(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u16_at(s, 0)? as usize;
    limits(cx, n)?;
    (0..n)
        .map(|i| {
            let at = if n == 1 {
                2
            } else {
                u64::from(u32_at(s, 2 + i as u64 * 4)?)
            };
            let orig = u64::from(u32_at(s, at)?);
            let comp = u64::from(u32_at(s, at + 4)?);
            Ok(Row::packed(
                format!("{i}.bmp"),
                at + 8,
                comp,
                orig,
                decode(|b, e| {
                    let mut c = Cursor::new(b);
                    let size = e.size as usize;
                    let mut out = Vec::with_capacity(size);
                    while out.len() < size {
                        let control = c.u8()?;
                        if control < 0x20 {
                            let n = control as usize + 1;
                            ensure!(n <= size - out.len(), "MGR literal overflow");
                            out.extend(c.take(n)?);
                        } else {
                            let mut n = (control >> 5) as usize;
                            if n == 7 {
                                n += c.u8()? as usize
                            }
                            n += 2;
                            let back = ((control as usize & 31) << 8) + 1 + c.u8()? as usize;
                            ensure!(
                                back <= out.len() && n <= size - out.len(),
                                "MGR dictionary reference"
                            );
                            for _ in 0..n {
                                out.push(out[out.len() - back]);
                            }
                        }
                    }
                    Ok(out)
                }),
            ))
        })
        .collect()
}
fn dat_name(path: &str) -> String {
    let name = if path.is_empty() { "unknown" } else { path };
    match name.rsplit_once('.') {
        Some((base, _)) => format!("{base}.dat"),
        None => format!("{name}.dat"),
    }
}
pub(super) fn abmp7(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let mut at = 16 + u64::from(u32_at(s, 12)?);
    let size = u64::from(u32_at(s, at)?);
    at += 4;
    let mut rows = vec![Row::raw("base.dat".into(), at, size)];
    at += size;
    let total = length(s)?;
    while at < total {
        let h = index(s, cx, at, 36)?;
        let n = h[0] as usize;
        ensure!(n <= 31, "ABMP7 name size");
        let size = u64::from(le32(&h, 32)?);
        rows.push(Row::raw(dat_name(&sjis(&h[1..1 + n])?), at + 36, size));
        at += 36 + size;
        limits(cx, rows.len())?;
    }
    Ok(rows)
}
pub(super) fn abmp10(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let mut at = 16;
    let total = length(s)?;
    let mut rows = Vec::new();
    while at < total {
        let tag = bytes(s, at, 16, 16)?;
        at += 16;
        let tag = zstr(&tag);
        if [
            b"abdata10".as_slice(),
            b"abdata11",
            b"abdata12",
            b"abdata13",
        ]
        .contains(&tag)
        {
            let size = u64::from(u32_at(s, at)?);
            at += 4;
            rows.push(Row::raw("unknown.dat".into(), at, size));
            at += size;
        } else {
            ensure!(
                tag == b"abimage10" || tag == b"absound10",
                "unknown ABMP section"
            );
            let n = bytes(s, at, 1, 1)?[0];
            at += 1;
            for _ in 0..n {
                let raw = bytes(s, at, 18, 18)?;
                let tag = zstr(&raw[..16]);
                let ns = u16::from_le_bytes(raw[16..18].try_into().unwrap()) as u64;
                at += 18;
                let name = dat_name(&sjis(&index(s, cx, at, ns)?)?);
                at += ns;
                if [
                    b"absnddat11".as_slice(),
                    b"abimgdat11",
                    b"abimgdat13",
                    b"abimgdat14",
                ]
                .contains(&tag)
                {
                    let ns = u64::from(u16_at(s, at)?);
                    at += 2 + ns;
                } else {
                    ensure!(
                        tag == b"abimgdat10" || tag == b"absnddat10",
                        "unknown ABMP resource"
                    );
                }
                at += 1;
                if tag == b"abimgdat14" {
                    at += 76
                }
                if tag == b"abimgdat13" {
                    at += 12
                }
                let size = u64::from(u32_at(s, at)?);
                at += 4;
                if size > 0 {
                    rows.push(Row::raw(name, at, size))
                }
                at += size;
                limits(cx, rows.len())?;
            }
        }
        ensure!(at <= total, "ABMP entry outside source");
        limits(cx, rows.len())?;
    }
    Ok(rows)
}
