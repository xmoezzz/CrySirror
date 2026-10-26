// SPDX-License-Identifier: GPL-3.0-or-later
// Layouts ported from arc_unpacker, Copyright (C) 2016 rr-:
// yuris/ypf, yuka-script/ykc, triangle/med, kiss/{arc,plg},
// liar-soft/{xfl,lwg}, nsystem/fjsys, propeller/mpk, silky/arc.
use super::*;
use anyhow::bail;

pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "yuris/ypf",
        probe: Some(|s| magic(s, 0, b"YPF\0")),
        open: ypf,
    },
    Legacy {
        id: "yuka-script/ykc",
        probe: Some(|s| magic(s, 0, b"YKC001")),
        open: ykc,
    },
    Legacy {
        id: "triangle/med",
        probe: Some(|s| Ok(magic(s, 0, b"MD")? && !med(s, &super::a::probe_context())?.is_empty())),
        open: med,
    },
    Legacy {
        id: "kiss/arc",
        probe: None,
        open: kiss_arc,
    },
    Legacy {
        id: "kiss/plg",
        probe: Some(|s| magic(s, 0, b"LIB\0\0\0\x01\0")),
        open: plg,
    },
    Legacy {
        id: "liar-soft/xfl",
        probe: Some(|s| magic(s, 0, b"LB\x01\0")),
        open: xfl,
    },
    Legacy {
        id: "liar-soft/lwg",
        probe: Some(|s| magic(s, 0, b"LG\x01\0")),
        open: lwg,
    },
    Legacy {
        id: "nsystem/fjsys",
        probe: Some(|s| magic(s, 0, b"FJSYS\0\0\0")),
        open: fjsys,
    },
    Legacy {
        id: "propeller/mpk",
        probe: None,
        open: propeller_mpk,
    },
    Legacy {
        id: "silky/arc",
        probe: None,
        open: silky_arc,
    },
];

fn swap_table(swaps: &[(u8, u8)]) -> [u8; 256] {
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = i as u8;
    }
    for (a, b) in swaps {
        t[*a as usize] = *b;
        t[*b as usize] = *a;
    }
    t
}
const YPF_COMMON: &[(u8, u8)] = &[
    (0x09, 0x0B),
    (0x0D, 0x13),
    (0x15, 0x1B),
    (0x20, 0x23),
    (0x26, 0x29),
    (0x2C, 0x2F),
    (0x2E, 0x32),
];
fn ypf_tables() -> [[u8; 256]; 4] {
    let with = |extra: &[(u8, u8)]| {
        let mut all = extra.to_vec();
        all.extend_from_slice(YPF_COMMON);
        swap_table(&all)
    };
    [
        with(&[
            (0x03, 0x48),
            (0x06, 0x35),
            (0x0C, 0x10),
            (0x11, 0x19),
            (0x1C, 0x1E),
        ]),
        with(&[(0x0C, 0x10), (0x11, 0x19), (0x1C, 0x1E)]),
        with(&[]),
        with(&[
            (0x03, 0x48),
            (0x06, 0x35),
            (0x0C, 0x10),
            (0x11, 0x19),
            (0x1C, 0x1E),
        ]),
    ]
}
/// YU-RIS YPF: name lengths are permuted through one of four known swap
/// tables and name bytes are XORed by a key recovered from known extensions.
fn ypf(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 16, 16)?;
    let version = le32(&h, 4)?;
    let count = le32(&h, 8)? as usize;
    limits(cx, count)?;
    let table = index(s, cx, 0x20, u64::from(le32(&h, 12)?))?;
    let extra = if version == 0xDE {
        12
    } else if version >= 0x1D9 {
        8
    } else {
        4
    };
    let parse = |sizes: &[u8; 256]| -> Result<Vec<(Vec<u8>, bool, u64, u64, u64)>> {
        let mut c = Cursor::new(&table);
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            c.take(4)?;
            let n = sizes[usize::from(c.u8()? ^ 0xFF)] as usize;
            let name = c.take(n)?.to_vec();
            let _kind = c.u8()?;
            let compressed = c.u8()? == 1;
            let size = u64::from(c.le32()?);
            let stored = u64::from(c.le32()?);
            let offset = u64::from(c.le32()?);
            c.take(extra)?;
            out.push((name, compressed, size, stored, offset));
        }
        Ok(out)
    };
    let records = ypf_tables()
        .iter()
        .find_map(|t| parse(t).ok())
        .context("Failed to guess name size permutation table")?;
    let key = records
        .iter()
        .filter(|r| r.0.len() >= 4)
        .find_map(|(name, ..)| {
            let key = name[name.len() - 4] ^ b'.';
            let ext: Vec<u8> = name[name.len() - 3..].iter().map(|b| b ^ key).collect();
            [&b"bmp"[..], b"png", b"ogg", b"wav", b"txt", b"ybn"]
                .contains(&ext.as_slice())
                .then_some(key)
        })
        .context("Failed to guess the key")?;
    records
        .into_iter()
        .map(|(name, compressed, size, stored, offset)| {
            let name: Vec<u8> = name.iter().map(|b| b ^ key).collect();
            let path = sjis(&name)?;
            Ok(if compressed {
                Row::packed(path, offset, stored, size, Plan::Zlib)
            } else {
                Row::raw(path, offset, stored)
            }
            .meta(serde_json::json!({"version": version, "name_key": key})))
        })
        .collect()
}

fn ykc(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 16, 8, 8)?;
    let (at, size) = (u64::from(le32(&h, 0)?), u64::from(le32(&h, 4)?));
    limits(cx, (size / 20) as usize)?;
    let table = index(s, cx, at, size / 20 * 20)?;
    table
        .chunks_exact(20)
        .map(|r| {
            let name = index(s, cx, u64::from(le32(r, 0)?), u64::from(le32(r, 4)?))?;
            Ok(Row::raw(
                sjis(zstr(&name))?,
                u64::from(le32(r, 8)?),
                u64::from(le32(r, 12)?),
            ))
        })
        .collect()
}

fn med(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 8, 8)?;
    ensure!(&h[..2] == b"MD", "Triangle MED magic");
    let entry_size = usize::from(u16::from_le_bytes([h[4], h[5]]));
    let count = usize::from(u16::from_le_bytes([h[6], h[7]]));
    ensure!(entry_size >= 8, "MED entry size");
    let table = index(s, cx, 16, (count * entry_size) as u64)?;
    table
        .chunks_exact(entry_size)
        .map(|r| {
            let n = entry_size - 8;
            Ok(Row::raw(
                sjis(zstr(&r[..n]))?,
                u64::from(le32(r, n + 4)?),
                u64::from(le32(r, n)?),
            ))
        })
        .collect()
}

fn kiss_arc(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let size = length(s)?;
    let table = index(s, cx, 0, size.min(cx.limits.max_index_size))?;
    let mut c = Cursor::new(&table);
    let count = c.le32()? as usize;
    limits(cx, count)?;
    let mut items: Vec<(String, u64)> = Vec::with_capacity(count);
    for _ in 0..count {
        let path = sjis(c.zero()?)?;
        let offset = u64::from(c.le32()?);
        ensure!(c.le32()? == 0, "Expected '0'");
        ensure!(
            items.last().is_none_or(|l| offset >= l.1),
            "KISS ARC offsets out of order"
        );
        items.push((path, offset));
    }
    let ends: Vec<u64> = items.iter().skip(1).map(|i| i.1).chain([size]).collect();
    Ok(items
        .into_iter()
        .zip(ends)
        .map(|((path, offset), end)| Row::raw(path, offset, end.saturating_sub(offset)))
        .collect())
}

fn plg(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(le32(&bytes(s, 8, 4, 4)?, 0)?);
    limits(cx, count as usize)?;
    let table = index(s, cx, 16, count * 0x30)?;
    table
        .chunks_exact(0x30)
        .enumerate()
        .map(|(i, r)| {
            ensure!(le32(r, 0x20)? as usize == i, "Unexpected entry index");
            ensure!(le32(r, 0x2C)? == 0, "Expected '0'");
            Ok(Row::raw(
                sjis(zstr(&r[..0x20]))?,
                u64::from(le32(r, 0x28)?),
                u64::from(le32(r, 0x24)?),
            ))
        })
        .collect()
}

fn xfl(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 4, 8, 8)?;
    let (table_size, count) = (u64::from(le32(&h, 0)?), u64::from(le32(&h, 4)?));
    limits(cx, count as usize)?;
    let start = 12 + table_size;
    let table = index(s, cx, 12, count * 0x28)?;
    table
        .chunks_exact(0x28)
        .map(|r| {
            Ok(Row::raw(
                sjis(zstr(&r[..0x20]))?,
                start + u64::from(le32(r, 0x20)?),
                u64::from(le32(r, 0x24)?),
            ))
        })
        .collect()
}

fn lwg(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 24, 24)?;
    let (width, height, count) = (le32(&h, 4)?, le32(&h, 8)?, le32(&h, 12)? as usize);
    let table_size = u64::from(le32(&h, 20)?);
    limits(cx, count)?;
    let start = 24 + table_size + 4;
    let table = index(s, cx, 24, table_size)?;
    let mut c = Cursor::new(&table);
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        c.take(9)?;
        let offset = start + u64::from(c.le32()?);
        let size = u64::from(c.le32()?);
        let n = c.u8()? as usize;
        rows.push(
            Row::raw(sjis(c.take(n)?)?, offset, size)
                .meta(serde_json::json!({"canvas": [width, height]})),
        );
    }
    Ok(rows)
}

fn fjsys(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 8, 12, 12)?;
    let header_size = u64::from(le32(&h, 0)?);
    let names_size = u64::from(le32(&h, 4)?);
    let count = u64::from(le32(&h, 8)?);
    limits(cx, count as usize)?;
    let names_at = header_size
        .checked_sub(names_size)
        .context("FJSYS names offset")?;
    let names = index(s, cx, names_at, names_size)?;
    let table = index(s, cx, 84, count * 16)?;
    table
        .chunks_exact(16)
        .map(|r| {
            let name = names
                .get(le32(r, 0)? as usize..)
                .context("FJSYS name offset")?;
            ensure!(name.contains(&0), "unterminated FJSYS name");
            let offset = u64::from_le_bytes(r[8..16].try_into().unwrap());
            Ok(Row::raw(sjis(zstr(name))?, offset, u64::from(le32(r, 4)?)))
        })
        .collect()
}

fn propeller_mpk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 8, 8)?;
    let count = u64::from(le32(&h, 4)?);
    limits(cx, count as usize)?;
    let table = index(s, cx, u64::from(le32(&h, 0)?), count * 40)?;
    table
        .chunks_exact(40)
        .map(|r| {
            let key = r[31];
            let key32 = u32::from_le_bytes([key; 4]);
            let name: Vec<u8> = r[..32].iter().map(|b| b ^ key).collect();
            let name = sjis(zstr(&name))?.replace('\\', "/");
            let path = name.strip_prefix('/').unwrap_or(&name).to_string();
            Ok(Row::raw(
                path,
                u64::from(le32(r, 32)? ^ key32),
                u64::from(le32(r, 36)? ^ key32),
            ))
        })
        .collect()
}

/// Silky ARC: name bytes shifted by their distance from the name end;
/// big-endian sizes; bytewise LZSS when stored and decoded sizes differ.
fn silky_arc(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let table_size = u64::from(le32(&bytes(s, 0, 4, 4)?, 0)?);
    let table = index(s, cx, 4, table_size)?;
    let mut c = Cursor::new(&table);
    let mut rows = Vec::new();
    while c.left() > 0 {
        limits(cx, rows.len() + 1)?;
        let n = c.u8()? as usize;
        let name: Vec<u8> = c
            .take(n)?
            .iter()
            .enumerate()
            .map(|(j, b)| b.wrapping_add((n - j) as u8))
            .collect();
        let stored = u64::from(c.be32()?);
        let size = u64::from(c.be32()?);
        let offset = u64::from(c.be32()?);
        let path = sjis(&name)?;
        rows.push(if stored == size {
            Row::raw(path, offset, size)
        } else {
            Row::packed(
                path,
                offset,
                stored,
                size,
                Plan::Lzss(serde_json::Value::Null),
            )
        });
    }
    if rows.is_empty() {
        bail!("empty Silky ARC table");
    }
    Ok(rows)
}
