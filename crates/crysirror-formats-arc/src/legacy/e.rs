// SPDX-License-Identifier: GPL-3.0-or-later
// Archive layouts ported from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
use anyhow::bail;

mod entis;
mod extra;
mod images;
mod int_aliases;
mod int_executable;
mod pac3;
mod profiles;
mod structured;
pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "unity/assets",
        probe: None,
        open: extra::unity,
    },
    Legacy {
        id: "yuzusoft/psb",
        probe: Some(|s| magic(s, 0, b"PSB\0")),
        open: extra::psb,
    },
    Legacy {
        id: "almond-collective/pac3",
        probe: Some(|s| magic(s, 0, b"PAC3")),
        open: pac3::open,
    },
    Legacy {
        id: "abogado/dsk",
        probe: None,
        open: profiles::dsk,
    },
    Legacy {
        id: "eagls/pak",
        probe: None,
        open: profiles::eagls,
    },
    Legacy {
        id: "dxlib/dx",
        probe: None,
        open: profiles::dx,
    },
    Legacy {
        id: "glib/glib2",
        probe: None,
        open: profiles::glib2,
    },
    Legacy {
        id: "lucifen/lpk",
        probe: Some(|s| magic(s, 0, b"LPK1")),
        open: profiles::lpk,
    },
    Legacy {
        id: "fc01/mca",
        probe: Some(|s| magic(s, 0, b"MCA ")),
        open: images::mca,
    },
    Legacy {
        id: "malie/dzi",
        probe: Some(|s| magic(s, 0, b"DZI\r\n")),
        open: images::dzi,
    },
    Legacy {
        id: "cyberworks/appendix",
        probe: None,
        open: profiles::appendix,
    },
    Legacy {
        id: "cyberworks/dat",
        probe: None,
        open: profiles::cyber_dat,
    },
    Legacy {
        id: "abstraction/wad",
        probe: Some(|s| magic(s, 0, b"AGAR")),
        open: wad,
    },
    Legacy {
        id: "almond-collective/pac2",
        probe: Some(|s| magic(s, 0, b"PAC2")),
        open: pac2,
    },
    Legacy {
        id: "amuse-craft/pac",
        probe: Some(|s| Ok(amuse(s, &probe_cx()).is_ok())),
        open: amuse,
    },
    Legacy {
        id: "aoi/vfs",
        probe: Some(|s| magic(s, 0, b"VF")),
        open: vfs,
    },
    Legacy {
        id: "bishop/bsc",
        probe: Some(|s| magic(s, 0, b"BSS-Composition")),
        open: bsc,
    },
    Legacy {
        id: "bluearrowgarden/bin",
        probe: None,
        open: blue_bin,
    },
    Legacy {
        id: "bluearrowgarden/images",
        probe: None,
        open: images::blue_images,
    },
    Legacy {
        id: "cat-system/int",
        probe: Some(|s| magic(s, 0, b"KIF\0")),
        open: int,
    },
    Legacy {
        id: "cat-system/hg3",
        probe: Some(|s| magic(s, 0, b"HG-3")),
        open: images::hg3,
    },
    Legacy {
        id: "chanchan/dat",
        probe: None,
        open: chanchan,
    },
    Legacy {
        id: "cronus/pak",
        probe: Some(|s| {
            Ok(magic(s, 0, b"CHERRY PACK 2.0\0")? || magic(s, 0, b"CHERRY PACK 3.0\0")?)
        }),
        open: cronus,
    },
    Legacy {
        id: "crowd/pkwv",
        probe: Some(|s| magic(s, 0, b"PKWV")),
        open: pkwv,
    },
    Legacy {
        id: "fc01/mrg",
        probe: Some(|s| magic(s, 0, b"MRG\0")),
        open: mrg,
    },
    Legacy {
        id: "french-bread/p",
        probe: None,
        open: french_p,
    },
    Legacy {
        id: "gnu/gzip",
        probe: Some(|s| magic(s, 0, b"\x1f\x8b")),
        open: gzip,
    },
    Legacy {
        id: "ivory/mbl",
        probe: None,
        open: mbl,
    },
    Legacy {
        id: "libido/bid",
        probe: None,
        open: bid,
    },
    Legacy {
        id: "libido/egr",
        probe: None,
        open: images::egr,
    },
    Legacy {
        id: "malie/libu",
        probe: Some(|s| magic(s, 0, b"LIBU")),
        open: libu,
    },
    Legacy {
        id: "malie/libp",
        probe: Some(|s| magic(s, 0, b"LIBP")),
        open: libp,
    },
    Legacy {
        id: "microsoft/exe",
        probe: Some(|s| magic(s, 0, b"MZ")),
        open: structured::exe,
    },
    Legacy {
        id: "entis/noa",
        probe: Some(|s| {
            Ok(magic(
                s,
                0,
                b"Entis\x1a\0\0\0\x04\0\x02\0\0\0\0ERISA-Archive file",
            )?)
        }),
        open: structured::noa,
    },
    Legacy {
        id: "cri/cpk",
        probe: Some(|s| magic(s, 0, b"CPK ")),
        open: structured::cpk,
    },
];
fn probe_cx() -> DecodeContext {
    let mut c = DecodeContext::new(crysirror_core::state::ClientId(0));
    c.limits.max_index_size = 256 * 1024;
    c
}
fn count(c: &mut Cursor<'_>, cx: &DecodeContext) -> Result<usize> {
    let n = c.le32()? as usize;
    limits(cx, n)?;
    Ok(n)
}
fn string(c: &mut Cursor<'_>) -> Result<String> {
    let n = c.le32()? as usize;
    Ok(String::from_utf8(c.take(n)?.to_vec())?)
}
fn checked_rows(s: &dyn AssetReader, cx: &DecodeContext, r: Vec<Row>) -> Result<Vec<Row>> {
    limits(cx, r.len())?;
    for e in &r {
        range(length(s)?, e.offset, e.stored)?;
    }
    Ok(r)
}
fn wad(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"AGAR")?, "missing AGAR");
    let b = window(s, cx, 0)?;
    let mut c = Cursor::new(&b);
    c.take(12)?;
    let extra = c.le32()? as usize;
    c.take(extra)?;
    let n = count(&mut c, cx)?;
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        let name = string(&mut c)?;
        let size = c.le64()?;
        let off = c.le64()?;
        rows.push(Row::raw(name, off, size));
    }
    let dirs = count(&mut c, cx)?;
    for _ in 0..dirs {
        string(&mut c)?;
        let n = count(&mut c, cx)?;
        for _ in 0..n {
            string(&mut c)?;
            c.u8()?;
        }
    }
    for r in &mut rows {
        r.offset = r
            .offset
            .checked_add(c.at as u64)
            .context("WAD offset overflow")?;
    }
    checked_rows(s, cx, rows)
}
fn pac2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"PAC2")?, "missing PAC2");
    let b = window(s, cx, 12)?;
    let mut c = Cursor::new(&b);
    ensure!(c.take(4)? == b"CCOD", "missing CCOD");
    let n = c.le32()? as usize;
    let key = c.take(n)?.to_vec();
    ensure!(!key.is_empty(), "empty PAC2 key");
    ensure!(c.take(4)? == b"FNUM", "missing FNUM");
    c.take(4)?;
    let n = count(&mut c, cx)?;
    let mut pos = 12 + c.at as u64;
    let mut rows = Vec::new();
    for _ in 0..n {
        let h = window(s, cx, pos)?;
        let mut c = Cursor::new(&h);
        ensure!(c.take(4)? == b"FILE", "missing FILE");
        c.take(4)?;
        ensure!(c.take(4)? == b"NAME", "missing NAME");
        let n = c.le32()? as usize;
        let path = sjis(zstr(c.take(n)?))?;
        ensure!(c.take(4)? == b"DATA", "missing DATA");
        let size = c.le32()? as u64;
        let offset = pos + c.at as u64;
        let k = key.clone();
        rows.push(Row::packed(
            path,
            offset,
            size,
            size,
            decode(move |b, _| {
                Ok(b.iter()
                    .enumerate()
                    .map(|(i, v)| v ^ k[(offset as usize + i) % k.len()])
                    .collect())
            }),
        ));
        pos = offset.checked_add(size).context("PAC2 offset overflow")?;
    }
    checked_rows(s, cx, rows)
}
fn amuse(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let (n, name, offset) = if magic(s, 0, b"PAC ")? {
        (le32(&bytes(s, 8, 4, 4)?, 0)? as usize, 32, 0x804)
    } else {
        let n = u16::from_le_bytes(bytes(s, 0, 2, 2)?.try_into().unwrap()) as usize;
        ensure!(n > 0, "empty PAC");
        let last = bytes(s, 0x3fe + (n as u64 - 1) * 24 + 16, 8, 8)?;
        ensure!(
            u64::from(le32(&last, 0)?) + u64::from(le32(&last, 4)?) == length(s)?,
            "unknown PAC version"
        );
        (n, 16, 0x3fe)
    };
    limits(cx, n)?;
    let b = index(s, cx, offset, (n * (name + 8)) as u64)?;
    let mut c = Cursor::new(&b);
    let mut rows = Vec::new();
    for _ in 0..n {
        let path = sjis(zstr(c.take(name)?))?.replace('_', "/");
        let size = c.le32()? as u64;
        rows.push(Row::raw(path, c.le32()? as u64, size));
    }
    checked_rows(s, cx, rows)
}
fn vfs(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"VF")?, "missing VF");
    let h = bytes(s, 2, 14, 14)?;
    let mut c = Cursor::new(&h);
    let ver = c.le16()?;
    ensure!(
        matches!(ver, 0x100 | 0x101 | 0x200),
        "unsupported VFS version {ver:x}"
    );
    let n = c.le16()? as usize;
    limits(cx, n)?;
    let stride = c.le16()? as usize;
    ensure!(
        stride >= if ver == 0x200 { 18 } else { 27 },
        "short VFS entry"
    );
    let table = index(s, cx, 16, (n * stride) as u64)?;
    let mut rows = Vec::new();
    for raw in table.chunks_exact(stride) {
        let mut c = Cursor::new(raw);
        let path = if ver == 0x200 {
            let at = 16 + (n * stride) as u64 + 8 + u64::from(c.le32()?) * 2;
            c.take(6)?;
            let b = window(s, cx, at)?;
            let end = b
                .chunks_exact(2)
                .position(|p| p == [0, 0])
                .context("unterminated VFS name")?
                * 2;
            utf16(&b[..end])?
        } else {
            sjis(zstr(c.take(19)?))?
        };
        let off = c.le32()? as u64;
        rows.push(Row::raw(path, off, c.le32()? as u64));
    }
    checked_rows(s, cx, rows)
}
fn bsc(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"BSS-Composition")?, "missing BSC");
    let mut rows = Vec::new();
    let mut at = 32;
    let len = length(s)?;
    while at < len {
        limits(cx, rows.len() + 1)?;
        let size = le32(&bytes(s, at + 0x36, 4, 4)?, 0)? as u64 + 0x40;
        range(len, at, size)?;
        rows.push(Row::raw(String::new(), at, size));
        at += size;
    }
    name_unnamed(&mut rows);
    Ok(rows)
}
fn blue_bin(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let len = length(s)?;
    ensure!(len >= 8, "short BIN");
    let foot = bytes(s, len - 8, 8, 8)?;
    let orig = le32(&foot, 0)? as u64;
    ensure!(orig <= cx.limits.max_index_size, "BIN index limit");
    let comp = le32(&foot, 4)? as u64;
    let at = len.checked_sub(8 + comp).context("BIN index offset")?;
    let t = inflate_index(&index(s, cx, at, comp)?, orig)?;
    ensure!(t.len() as u64 == orig, "BIN index size");
    let mut c = Cursor::new(&t);
    let n = count(&mut c, cx)?;
    let mut rows = Vec::new();
    for _ in 0..n {
        let segs = count(&mut c, cx)?;
        let path = sjis(zstr(c.take(44)?))?;
        let mut parts = Vec::new();
        let mut first = u64::MAX;
        let mut last = 0;
        let mut size = 0u64;
        for _ in 0..segs {
            let flags = c.le32()?;
            let off = c.le32()? as u64;
            let comp = c.le32()? as u64;
            let orig = c.le32()? as u64;
            range(len, off, comp)?;
            first = first.min(off);
            last = last.max(off + comp);
            size = size.checked_add(orig).context("BIN size overflow")?;
            parts.push((off, comp, orig, flags));
        }
        ensure!(size <= cx.limits.max_entry_size, "BIN output limit");
        if segs == 0 {
            first = 0;
        }
        let plan = decode(move |b, _| {
            let mut out = Vec::with_capacity(size as usize);
            for (off, comp, orig, _) in &parts {
                let start = (off - first) as usize;
                let end = start + *comp as usize;
                let v = inflate_index(
                    b.get(start..end).context("BIN segment outside input")?,
                    *orig,
                )?;
                ensure!(v.len() as u64 == *orig, "BIN segment output size");
                out.extend(v);
            }
            Ok(out)
        });
        rows.push(Row::packed(path, first, last - first, size, plan));
    }
    Ok(rows)
}
/// Index transforms carry logical byte offsets so caller-owned ciphers can
/// preserve absolute-position semantics without access to host pointers.
fn index_profile(
    cx: &DecodeContext,
    id: &str,
    name: &str,
    offset: u64,
    data: Vec<u8>,
    required: bool,
) -> Result<Vec<u8>> {
    let n = data.len() as u64;
    ensure!(n <= cx.limits.max_index_size, "archive index profile limit");
    let entry = ArchiveEntry {
        id: 0,
        path: name.into(),
        size: n,
        stored_size: n,
        metadata: serde_json::json!({"format":id,"offset":offset}),
    };
    let pipeline = cx.pipeline(id, &entry)?;
    ensure!(
        !required || !pipeline.is_empty(),
        "{id} encrypted index needs caller {name} transform profile"
    );
    let mut reader: Arc<dyn AssetReader> =
        Arc::new(crysirror_core::service::MemoryAssetReader(Arc::from(data)));
    for spec in pipeline {
        reader = cx.apply(&spec, reader, &entry.metadata)?;
    }
    let out = crysirror_resource::read_all(reader.as_ref(), cx.limits.max_index_size)?;
    ensure!(
        out.len() as u64 == n,
        "archive index profile changed native layout length"
    );
    Ok(out)
}
fn int(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    cx.check_cancelled()?;
    ensure!(magic(s, 0, b"KIF\0")?, "missing KIF");
    let n = le32(&bytes(s, 4, 4, 4)?, 0)? as usize;
    limits(cx, n)?;
    let raw = index(s, cx, 8, n as u64 * 72)?;
    let native_key = raw
        .chunks_exact(72)
        .find(|p| zstr(&p[..64]) == b"__key__.dat");
    let mut rows = Vec::new();
    let mut known_decode = native_key.is_none();
    if let Some(key_record) = native_key {
        let profile = cx.parameters.get("cat-system/int");
        let alias = profile
            .and_then(|p| p.get("game_id_key_id"))
            .and_then(|v| v.as_str());
        let executable = profile
            .and_then(|p| p.get("executable"))
            .and_then(|v| v.as_str());
        let game_id = if let Some(alias) = alias {
            Some(cx.keys.key(cx.client, alias)?.with_bytes(|b| b.to_vec()))
        } else if let Some(path) = executable {
            Some(int_executable::game_id(cx, path)?)
        } else {
            None
        };
        if let Some(game_id) = game_id {
            // Native body Blowfish uses only this archive's file key. Offset
            // and ordinal are index metadata, never body cipher inputs.
            known_decode = true;
            let seed = {
                let b = &game_id;
                let mut seed = 0xffffffffu32;
                for v in b {
                    seed ^= (*v as u32) << 24;
                    for _ in 0..8 {
                        let carry = seed & 0x80000000 != 0;
                        seed <<= 1;
                        if carry {
                            seed ^= 0x04c11db7;
                        }
                    }
                    seed = !seed;
                }
                seed
            };
            let file_key = mt_classic_first(le32(key_record, 68)?).to_le_bytes();
            for (i, p) in raw.chunks_exact(72).enumerate() {
                cx.check_cancelled()?;
                if zstr(&p[..64]) == b"__key__.dat" {
                    continue;
                }
                let mut name = p[..64].to_vec();
                let r = mt_classic_first(seed.wrapping_add(i as u32));
                let mut shift = ((r >> 24)
                    .wrapping_add(r >> 16)
                    .wrapping_add(r >> 8)
                    .wrapping_add(r)) as u8 as usize;
                let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
                for v in &mut name {
                    if let Some(index) =
                        (0..52).find(|index| alphabet[51 - ((shift as usize + index) % 52)] == *v)
                    {
                        *v = alphabet[index];
                    }
                    shift += 1;
                }
                let mut pair = p[64..72].to_vec();
                let off = le32(&pair, 0)?.wrapping_add(i as u32);
                pair[..4].copy_from_slice(&off.to_le_bytes());
                let pair = crysirror_resource::blowfish_le_decrypt(&pair, &file_key, 8)?;
                let offset = le32(&pair, 0)? as u64;
                let size = le32(&pair, 4)? as u64;
                rows.push(
                    Row::packed(
                        sjis(zstr(&name))?,
                        offset,
                        size,
                        size,
                        decode(move |b, _| {
                            crysirror_resource::blowfish_le_decrypt(b, &file_key, b.len())
                        }),
                    )
                    .meta(serde_json::json!({"native_index":i,"encrypted":true})),
                );
            }
        } else {
            let table = index_profile(cx, "cat-system/int", "__index__", 8, raw, true)?;
            for (i, p) in table.chunks_exact(72).enumerate() {
                let path = sjis(zstr(&p[..64]))?;
                if path == "__key__.dat" {
                    continue;
                }
                let off = le32(p, 64)? as u64;
                let size = le32(p, 68)? as u64;
                rows.push(
                    Row::packed(path, off, size, size, Plan::Profile)
                        .meta(serde_json::json!({"native_index":i,"encrypted":true})),
                );
            }
        }
    } else {
        for (i, p) in raw.chunks_exact(72).enumerate() {
            cx.check_cancelled()?;
            rows.push(
                Row::raw(
                    sjis(zstr(&p[..64]))?,
                    le32(p, 64)? as u64,
                    le32(p, 68)? as u64,
                )
                .meta(serde_json::json!({"native_index":i,"encrypted":false})),
            );
        }
    }
    int_aliases::coalesce(s, cx, rows, known_decode)
}
/// MT19937 classic seeding differs from the modern standard recurrence.
fn mt_classic_first(mut seed: u32) -> u32 {
    let mut mt = [0u32; 624];
    for value in &mut mt {
        *value = seed & 0xffff0000;
        seed = seed.wrapping_mul(69069).wrapping_add(1);
        *value |= (seed & 0xffff0000) >> 16;
        seed = seed.wrapping_mul(69069).wrapping_add(1);
    }
    let y = (mt[0] & 0x80000000) | (mt[1] & 0x7fffffff);
    let mut y = mt[397] ^ (y >> 1) ^ if y & 1 != 0 { 0x9908b0df } else { 0 };
    y ^= y >> 11;
    y ^= (y << 7) & 0x9d2c5680;
    y ^= (y << 15) & 0xefc60000;
    y ^= y >> 18;
    y
}
fn chanchan(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    for ver in 1..=3 {
        if let Ok(r) = chan_ver(s, cx, ver) {
            return Ok(r);
        }
    }
    bail!("unrecognized ChanChan DAT variant")
}
fn chan_ver(s: &dyn AssetReader, cx: &DecodeContext, ver: u8) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 8, 8)?;
    let n = le32(&h, if ver == 2 { 4 } else { 0 })? as usize;
    limits(cx, n)?;
    ensure!(n > 0, "empty DAT");
    let mut rows = Vec::new();
    if ver == 1 {
        let mut pos = 4;
        for _ in 0..n {
            let h = index(s, cx, pos, 260)?;
            let size = le32(&h, 256)? as u64;
            rows.push(Row::raw(sjis(zstr(&h[..256]))?, pos + 260, size));
            pos += 260 + size;
        }
        ensure!(pos == length(s)?, "DAT v1 trailing bytes");
    } else {
        let start = if ver == 2 { 8 } else { 32 };
        let stride = if ver == 2 { 264 } else { 272 };
        if ver == 2 {
            ensure!(
                le32(&h, 0)? as u64 == start + n as u64 * stride,
                "DAT v2 table size"
            );
        }
        let t = index(s, cx, start, n as u64 * stride)?;
        for p in t.chunks_exact(stride as usize) {
            let path = sjis(zstr(&p[..256]))?;
            let offset = le32(p, 260)? as u64;
            if ver == 2 {
                rows.push(Row::raw(path, offset, le32(p, 256)? as u64));
            } else {
                let h = bytes(s, offset, 36, 36)?;
                let size = le32(&h, 16)? as u64;
                let stored = le32(&h, 20)? as u64;
                let plan = if size == stored {
                    Plan::Raw
                } else {
                    Plan::Lzss(serde_json::Value::Null)
                };
                rows.push(Row::packed(path, offset + 36, stored, size, plan));
            }
        }
    }
    checked_rows(s, cx, rows)
}
fn cronus(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(
        magic(s, 0, b"CHERRY PACK 2.0\0")? || magic(s, 0, b"CHERRY PACK 3.0\0")?,
        "missing CHERRY PACK"
    );
    let h = bytes(s, 16, 12, 12)?;
    let encrypted = le32(&h, 0)? > 0;
    let header_key = if let Some(alias) = cx
        .parameters
        .get("cronus/pak")
        .and_then(|p| p.get("header_key_id"))
        .and_then(|v| v.as_str())
    {
        cx.keys
            .key(cx.client, alias)?
            .with_bytes(|b| -> Result<(u32, u32)> {
                ensure!(b.len() == 8, "Cronus header key requires 8 bytes");
                Ok((le32(b, 0)?, le32(b, 4)?))
            })?
    } else {
        (0, 0)
    };
    let n = (le32(&h, 4)? ^ header_key.0) as usize;
    limits(cx, n)?;
    let base = (le32(&h, 8)? ^ header_key.1) as u64;
    ensure!(
        base >= 28,
        "Cronus requires caller-decrypted title header profile"
    );
    let mut table = index(s, cx, 28, base - 28)?;
    if encrypted {
        let key = b"CHERRYSOFT".iter().map(|x| *x as u32).sum::<u32>();
        for (i, b) in table.iter_mut().enumerate() {
            *b ^= key.wrapping_add((i as u32).wrapping_mul(key % 32)) as u8;
        }
        table = lzss_index(&table, n as u64 * 24, cx, false)?;
    }
    ensure!(table.len() >= n * 24, "short Cronus table");
    let mut rows = Vec::new();
    for p in table[..n * 24].chunks_exact(24) {
        rows.push(Row::raw(
            sjis(zstr(&p[..16]))?,
            base + le32(p, 16)? as u64,
            le32(p, 20)? as u64,
        ));
    }
    checked_rows(s, cx, rows)
}
fn pkwv(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"PKWV")?, "missing PKWV");
    let h = bytes(s, 4, 4, 4)?;
    let mut c = Cursor::new(&h);
    let fmts = c.le16()? as usize;
    let n = c.le16()? as usize;
    limits(cx, n)?;
    let base = 8 + fmts as u64 * 20 + n as u64 * 24;
    let b = index(s, cx, 8, base - 8)?;
    let formats = &b[..fmts * 20];
    let mut rows = Vec::new();
    for p in b[fmts * 20..].chunks_exact(24) {
        let fmt = u16::from_le_bytes([p[0], p[1]]) as usize;
        let f = formats
            .get(fmt * 20..fmt * 20 + 20)
            .context("PKWV format index")?
            .to_vec();
        let mut path = sjis(zstr(&p[2..12]))?;
        if let Some((stem, _)) = path.rsplit_once('.') {
            path = format!("{stem}.wav");
        } else {
            path.push_str(".wav");
        }
        let size = le32(p, 12)? as u64;
        let off = u64::from_le_bytes(p[16..24].try_into().unwrap()) + base;
        ensure!(size + 46 <= cx.limits.max_entry_size, "PKWV WAV byte limit");
        rows.push(Row::packed(
            path,
            off,
            size,
            size + 46,
            decode(move |b, _| {
                let mut out = b"RIFF".to_vec();
                out.extend((b.len() as u32 + 38).to_le_bytes());
                out.extend(b"WAVEfmt \x12\0\0\0");
                out.extend(&f[0..8]);
                let channels = u16::from_le_bytes([f[2], f[3]]) as u32;
                let rate = le32(&f, 4)?;
                let bits = u16::from_le_bytes([f[12], f[13]]) as u32;
                out.extend((rate * channels * bits / 8).to_le_bytes());
                out.extend(((channels * bits / 8) as u16).to_le_bytes());
                out.extend((bits as u16).to_le_bytes());
                out.extend([0, 0]);
                out.extend(b"data");
                out.extend((b.len() as u32).to_le_bytes());
                out.extend(b);
                Ok(out)
            }),
        ));
    }
    checked_rows(s, cx, rows)
}
fn french_p(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 8, 8)?;
    ensure!(le32(&h, 0)? <= 1, "invalid P header");
    let n = (le32(&h, 4)? ^ 0xe3df59ac) as usize;
    limits(cx, n)?;
    let t = index(s, cx, 8, n as u64 * 68)?;
    let mut rows = Vec::new();
    for (i, p) in t.chunks_exact(68).enumerate() {
        let name: Vec<u8> = p[..60]
            .iter()
            .enumerate()
            .map(|(j, b)| b ^ ((i * j * 3 + 0x3d) as u8))
            .collect();
        let key = zstr(&name).to_vec();
        ensure!(!key.is_empty(), "empty P filename key");
        let size = (le32(p, 64)? ^ 0xe3df59ac) as u64;
        let plan = decode(move |b, _| {
            let mut out = b.to_vec();
            for (i, b) in out.iter_mut().take(0x2173).enumerate() {
                *b ^= key[i % key.len()].wrapping_add(i as u8).wrapping_add(3);
            }
            Ok(out)
        });
        rows.push(Row::packed(
            sjis(zstr(&name))?,
            le32(p, 60)? as u64,
            size,
            size,
            plan,
        ));
    }
    checked_rows(s, cx, rows)
}
fn gzip(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    // Deflate is scanned incrementally to discover concatenated member ranges;
    // no decoded file body is retained while indexing.
    let mut pos = 0;
    let len = length(s)?;
    let mut rows = Vec::new();
    while pos < len {
        limits(cx, rows.len() + 1)?;
        let member = pos;
        let b = window(s, cx, pos)?;
        let mut c = Cursor::new(&b);
        ensure!(c.take(2)? == b"\x1f\x8b", "bad gzip member magic");
        ensure!(c.u8()? == 8, "unsupported gzip method");
        let flags = c.u8()?;
        ensure!(flags & 0xe0 == 0, "reserved gzip flags");
        c.take(6)?;
        if flags & 4 != 0 {
            let n = c.le16()? as usize;
            c.take(n)?;
        }
        let path = if flags & 8 != 0 {
            sjis(c.zero()?)?
        } else {
            String::new()
        };
        if flags & 16 != 0 {
            c.zero()?;
        }
        if flags & 2 != 0 {
            c.take(2)?;
        }
        pos += c.at as u64;
        let data_start = pos;
        let mut decoder = flate2::Decompress::new(false);
        let mut scratch = [0u8; 32768];
        loop {
            let chunk = bytes(s, pos, (len - pos).min(32768), 32768)?;
            ensure!(!chunk.is_empty(), "truncated gzip deflate");
            let before = decoder.total_in();
            let status = decoder.decompress(&chunk, &mut scratch, flate2::FlushDecompress::None)?;
            let consumed = decoder.total_in() - before;
            pos += consumed;
            ensure!(
                decoder.total_out() <= cx.limits.max_entry_size,
                "gzip output byte limit"
            );
            if status == flate2::Status::StreamEnd {
                break;
            }
            ensure!(
                consumed > 0 || decoder.total_out() > 0,
                "stalled gzip deflate"
            );
        }
        let size = decoder.total_out();
        let tail = bytes(s, pos, 8, 8)?;
        ensure!(
            le32(&tail, 4)? == size as u32,
            "gzip member length mismatch"
        );
        pos += 8;
        let plan = decode(move |b, _| {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(b)
                .take(size + 1)
                .read_to_end(&mut out)?;
            ensure!(out.len() as u64 == size, "gzip decoded length");
            Ok(out)
        });
        rows.push(
            Row::packed(path, member, pos - member, size, plan)
                .meta(serde_json::json!({"deflate_offset":data_start})),
        );
    }
    name_unnamed(&mut rows);
    Ok(rows)
}
fn mbl(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 8, 8)?;
    let n = le32(&h, 0)? as usize;
    limits(cx, n)?;
    ensure!(n > 0, "empty MBL");
    for (start, name) in [(4, 16), (8, le32(&h, 4)? as usize)] {
        if name == 0 || name > 16384 {
            continue;
        }
        let attempt = (|| -> Result<Vec<Row>> {
            let t = index(s, cx, start, n as u64 * (name as u64 + 8))?;
            let mut rows = Vec::new();
            for p in t.chunks_exact(name + 8) {
                rows.push(Row::raw(
                    sjis(zstr(&p[..name]))?,
                    le32(p, name)? as u64,
                    le32(p, name + 4)? as u64,
                ));
            }
            let e = rows.last().unwrap();
            ensure!(
                e.offset + e.stored == length(s)?,
                "MBL final entry mismatch"
            );
            checked_rows(s, cx, rows)
        })();
        if let Ok(r) = attempt {
            return Ok(r);
        }
    }
    bail!("unsupported MBL layout; mg_data requires explicit entry transform profile")
}
fn bid(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let start = le32(&bytes(s, 0, 4, 4)?, 0)? as u64;
    ensure!(start >= 8 && (start - 8) % 24 == 0, "invalid BID index");
    let n = (start - 8) / 24;
    limits(cx, n as usize)?;
    let t = index(s, cx, 8, start - 8)?;
    let mut rows = Vec::new();
    for p in t.chunks_exact(24) {
        rows.push(Row::raw(
            sjis(zstr(&p[..16]))?,
            start + le32(p, 16)? as u64,
            le32(p, 20)? as u64,
        ));
    }
    checked_rows(s, cx, rows)
}
fn libu(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let encrypted = !magic(s, 0, b"LIBU")?;
    let h = index_profile(
        cx,
        "malie/libu",
        "__header__",
        0,
        bytes(s, 0, 16, 16)?,
        encrypted,
    )?;
    ensure!(&h[..4] == b"LIBU", "LIBU decoded header mismatch");
    let n = le32(&h, 8)? as usize;
    limits(cx, n)?;
    let t = index_profile(
        cx,
        "malie/libu",
        "__index__",
        16,
        index(s, cx, 16, n as u64 * 80)?,
        encrypted,
    )?;
    let mut rows = Vec::new();
    for p in t.chunks_exact(80) {
        let end = p[..68]
            .chunks_exact(2)
            .position(|p| p == [0, 0])
            .unwrap_or(34)
            * 2;
        let size = le32(p, 68)? as u64;
        rows.push(Row::packed(
            utf16(&p[..end])?,
            le32(p, 72)? as u64,
            size,
            size,
            if encrypted { Plan::Profile } else { Plan::Raw },
        ));
    }
    checked_rows(s, cx, rows)
}
fn libp(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let encrypted = !magic(s, 0, b"LIBP")?;
    let h = index_profile(
        cx,
        "malie/libp",
        "__header__",
        0,
        bytes(s, 0, 16, 16)?,
        encrypted,
    )?;
    ensure!(&h[..4] == b"LIBP", "LIBP decoded header mismatch");
    let n = le32(&h, 4)? as usize;
    let m = le32(&h, 8)? as usize;
    limits(cx, n)?;
    limits(cx, m)?;
    let t = index_profile(
        cx,
        "malie/libp",
        "__index__",
        16,
        index(s, cx, 16, n as u64 * 32 + m as u64 * 4)?,
        encrypted,
    )?;
    let align = cx
        .parameters
        .get("malie/libp")
        .and_then(|p| p.get("data_alignment"))
        .and_then(|v| v.as_u64())
        .unwrap_or(4096);
    ensure!(
        align > 0 && align.is_power_of_two() && align <= 1 << 20,
        "invalid LIBP data alignment"
    );
    let base = (16 + t.len() as u64 + align - 1) & !(align - 1);
    let mut rows = Vec::new();
    let mut stack = vec![(0usize, String::new(), 0usize)];
    let mut seen = std::collections::BTreeSet::new();
    while let Some((i, root, depth)) = stack.pop() {
        ensure!(depth <= 256 && seen.insert(i), "LIBP cyclic/deep directory");
        ensure!(i < n, "LIBP directory index");
        let p = &t[i * 32..i * 32 + 32];
        let name = sjis(zstr(&p[..20]))?;
        let path = if root.is_empty() {
            name
        } else if name.is_empty() {
            root.clone()
        } else {
            format!("{root}/{name}")
        };
        let flags = le32(p, 20)?;
        let off = le32(p, 24)? as usize;
        let size = le32(p, 28)? as u64;
        if flags & 0x10000 != 0 {
            ensure!(off < m, "LIBP offset index");
            let offset = base + u64::from(le32(&t, n * 32 + off * 4)?) * 1024;
            rows.push(Row::packed(
                path,
                offset,
                size,
                size,
                if encrypted { Plan::Profile } else { Plan::Raw },
            ));
        } else if off > i {
            ensure!(
                off.checked_add(size as usize).is_some_and(|v| v <= n),
                "LIBP directory range"
            );
            for child in (off..off + size as usize).rev() {
                stack.push((child, path.clone(), depth + 1));
            }
        }
    }
    checked_rows(s, cx, rows)
}
fn mrg(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"MRG\0")?, "missing MRG");
    let h = bytes(s, 8, 8, 8)?;
    let base = le32(&h, 0)? as u64;
    ensure!(base >= 16, "MRG index offset");
    let n = le32(&h, 4)? as usize;
    limits(cx, n)?;
    let mut t = index(s, cx, 16, base - 16)?;
    ensure!(t.len() >= (n + 1) * 32, "short MRG table");
    let file_len = length(s)?;
    ensure!(file_len <= u32::MAX as u64, "MRG >4GiB");
    let mut key = t[t.len() - 1].rotate_left(1) ^ ((file_len >> 24) as u8);
    let mut last = (t[t.len() - 1].rotate_left(1) ^ key) as u32;
    let mut pos = 1u32;
    for at in (t.len() - 4..t.len() - 1).rev() {
        pos += 1;
        key = key.wrapping_sub(pos as u8);
        last = (last << 8) | (t[at].rotate_left(1) ^ key) as u32;
    }
    ensure!(last as u64 == file_len, "failed MRG key inference");
    while pos < t.len() as u32 {
        pos += 1;
        key = key.wrapping_sub(pos as u8);
    }
    let total = t.len();
    for (i, b) in t.iter_mut().enumerate() {
        *b = b.rotate_left(1) ^ key;
        key = key.wrapping_add((total - i) as u8);
    }
    let mut rows = Vec::new();
    for i in 0..n {
        let p = &t[i * 32..(i + 1) * 32];
        let off = le32(p, 28)? as u64;
        let end = le32(&t, (i + 1) * 32 + 28)? as u64;
        ensure!(end >= off, "MRG offsets not ordered");
        let stored = end - off;
        let size = le32(p, 14)? as u64;
        let filter = p[18];
        let max = cx.limits.max_entry_size;
        let plan = if filter == 0 {
            Plan::Raw
        } else {
            decode(move |b, _| {
                let mut data = if filter >= 2 {
                    mrg_arithmetic(b, max)?
                } else {
                    b.to_vec()
                };
                if filter < 3 {
                    data = fc_lzss(&data, size as usize)?;
                }
                Ok(data)
            })
        };
        rows.push(
            Row::packed(sjis(zstr(&p[..14]))?, off, stored, size, plan)
                .meta(serde_json::json!({"filter":filter})),
        );
    }
    checked_rows(s, cx, rows)
}
pub(super) fn fc_lzss(b: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(b);
    let mut dict = [0u8; 4096];
    let mut at = 0xfee;
    let mut out = vec![0; size];
    let mut written = 0;
    let mut ctl = 0u16;
    while written < size && c.left() > 0 {
        ctl >>= 1;
        if ctl & 0x100 == 0 {
            ctl = c.u8()? as u16 | 0xff00;
        }
        if ctl & 1 != 0 {
            let v = c.u8()?;
            dict[at] = v;
            at = (at + 1) % 4096;
            out[written] = v;
            written += 1;
        } else {
            let tmp = c.le16()? as usize;
            let mut p = tmp % 4096;
            for _ in 0..((tmp >> 12) + 3).min(size - written) {
                let v = dict[p];
                out[written] = v;
                written += 1;
                dict[at] = v;
                at = (at + 1) % 4096;
                p = (p + 1) % 4096;
            }
        }
    }
    Ok(out)
}
fn mrg_arithmetic(b: &[u8], limit: u64) -> Result<Vec<u8>> {
    ensure!(b.len() >= 0x108, "short MRG arithmetic header");
    let size = (le32(b, 0)? ^ le32(b, 0x104)?) as usize;
    ensure!(size as u64 <= limit, "MRG arithmetic output limit");
    let mut c = Cursor::new(&b[4..]);
    let mut cumulative = [0u32; 257];
    let mut freq = [0u32; 256];
    let mut symbols = Vec::new();
    for i in 0..256 {
        freq[i] = c.u8()? as u32;
        cumulative[i + 1] = cumulative[i] + freq[i];
        symbols.extend(std::iter::repeat_n(i as u8, freq[i] as usize));
    }
    let quant = cumulative[256];
    ensure!(quant > 0, "zero MRG quant");
    let mut d = (quant - 1) >> 8;
    let mut mask = 255u32;
    while d > 0 {
        d >>= 1;
        mask = (mask << 1) | 1;
    }
    let scale = 0x10000 / quant;
    let mut a = c.be32()?;
    let mut lo = 0u32;
    let mut span = u32::MAX;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        span = ((span >> 8) * scale) >> 8;
        ensure!(span != 0, "MRG arithmetic zero range");
        let idx = a.wrapping_sub(lo) / span;
        let v = *symbols
            .get(idx as usize)
            .context("MRG arithmetic invalid quant")? as usize;
        out.push(v as u8);
        lo = lo.wrapping_add(cumulative[v].wrapping_mul(span));
        span = span.wrapping_mul(freq[v]);
        let mut steps = 0;
        while (span.wrapping_add(lo) ^ lo) & 0xff000000 == 0 {
            ensure!(steps < 16, "MRG arithmetic stalled");
            steps += 1;
            a = (a << 8) | c.u8()? as u32;
            lo <<= 8;
            span <<= 8;
        }
        while span <= mask {
            ensure!(steps < 16, "MRG arithmetic stalled");
            steps += 1;
            span = (!lo & mask) << 8;
            a = (a << 8) | c.u8()? as u32;
            lo <<= 8;
        }
    }
    Ok(out)
}
