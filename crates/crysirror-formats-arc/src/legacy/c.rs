// SPDX-License-Identifier: GPL-3.0-or-later
// Layouts ported from arc_unpacker, Copyright (C) 2016 rr-:
// active-soft/adpack, adv/dat, ast/arc, avg/gxp, bishop/bsa, gs/{dat,pak},
// gsd/gsp, ism/isa, libido/arc, lilim/{aos1,aos2,dpk}, mages/mpk,
// riddle-soft/pac, sysadv/pak, tabito/dat, yumemiru/dat, cherry-soft/myk,
// crowd/pck, escude/acp-pk1, dogenzaka/bin, glib/gml, gpk2/gpk2.
use super::*;

pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "active-soft/adpack",
        probe: Some(|s| magic(s, 0, b"ADPACK32")),
        open: adpack,
    },
    Legacy {
        id: "adv/dat",
        probe: Some(adv_probe),
        open: adv_dat,
    },
    Legacy {
        id: "ast/arc",
        probe: Some(|s| Ok(magic(s, 0, b"ARC1")? || magic(s, 0, b"ARC2")?)),
        open: ast_arc,
    },
    Legacy {
        id: "avg/gxp",
        probe: Some(|s| magic(s, 0, b"GXP\0")),
        open: gxp,
    },
    Legacy {
        id: "bishop/bsa",
        probe: Some(|s| magic(s, 0, b"BSArc\0\0\0")),
        open: bsa,
    },
    Legacy {
        id: "gs/dat",
        probe: Some(|s| magic(s, 0, b"GsSYMBOL5BINDATA")),
        open: gs_dat,
    },
    Legacy {
        id: "gs/pak",
        probe: Some(|s| magic(s, 0, b"DataPack5\0\0\0\0\0\0\0")),
        open: gs_pak,
    },
    Legacy {
        id: "gsd/gsp",
        probe: Some(gsp_probe),
        open: gsp,
    },
    Legacy {
        id: "ism/isa",
        probe: Some(|s| magic(s, 0, b"ISM ARCHIVED")),
        open: isa,
    },
    Legacy {
        id: "libido/arc",
        probe: Some(libido_probe),
        open: libido_arc,
    },
    Legacy {
        id: "lilim/aos1",
        probe: None,
        open: aos1,
    },
    Legacy {
        id: "lilim/aos2",
        probe: Some(aos2_probe),
        open: aos2,
    },
    Legacy {
        id: "lilim/dpk",
        probe: Some(dpk_probe),
        open: dpk,
    },
    Legacy {
        id: "mages/mpk",
        probe: Some(|s| magic(s, 0, b"MPK\0")),
        open: mages_mpk,
    },
    Legacy {
        id: "riddle-soft/pac",
        probe: Some(|s| magic(s, 0, b"PAC1")),
        open: riddle_pac,
    },
    Legacy {
        id: "sysadv/pak",
        probe: Some(|s| magic(s, 0, b"\x05PACK2")),
        open: sysadv_pak,
    },
    Legacy {
        id: "tabito/dat",
        probe: None,
        open: tabito_dat,
    },
    Legacy {
        id: "yumemiru/dat",
        probe: Some(|s| Ok(magic(s, 0, b"yanepkDx")? || magic(s, 0, b"PackDat3")?)),
        open: yumemiru_dat,
    },
    Legacy {
        id: "cherry-soft/myk",
        probe: Some(|s| magic(s, 0, b"MYK00\x1A\0\0")),
        open: myk,
    },
    Legacy {
        id: "crowd/pck",
        probe: None,
        open: crowd_pck,
    },
    Legacy {
        id: "escude/acp-pk1",
        probe: Some(|s| Ok(magic(s, 0, b"ACP_PK.1")? || magic(s, 0, b"ACPXPK01")?)),
        open: acp_pk1,
    },
    Legacy {
        id: "dogenzaka/bin",
        probe: None,
        open: dogenzaka_bin,
    },
    Legacy {
        id: "glib/gml",
        probe: Some(|s| magic(s, 0, b"GML_ARC\0")),
        open: gml,
    },
    Legacy {
        id: "gpk2/gpk2",
        probe: Some(|s| magic(s, 0, b"GPK2")),
        open: gpk2,
    },
];

fn u32_at(s: &dyn AssetReader, at: u64) -> Result<u32> {
    le32(&bytes(s, at, 4, 4)?, 0)
}
/// Sizes taken from successive offsets; the last entry runs to the end.
fn chain_sizes(rows: &mut [Row], end: u64) -> Result<()> {
    for i in 0..rows.len() {
        let next = rows.get(i + 1).map_or(end, |r| r.offset);
        let size = next
            .checked_sub(rows[i].offset)
            .context("archive offsets out of order")?;
        rows[i].stored = size;
        rows[i].size = size;
    }
    Ok(())
}

fn adpack(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 12)?)
        .checked_sub(1)
        .context("ADPACK file count")?;
    limits(cx, count as usize)?;
    let table = index(s, cx, 0x10, count * 0x20)?;
    let mut rows: Vec<Row> = table
        .chunks_exact(0x20)
        .map(|r| {
            Ok(Row::raw(
                sjis(zstr(&r[..0x18]))?,
                u64::from(le32(r, 0x1C)?),
                0,
            ))
        })
        .collect::<Result<_>>()?;
    chain_sizes(&mut rows, length(s)?)?;
    Ok(rows)
}

/// ADV DAT: an 8-byte key recovered from the known plaintext "ARCHIVE\0".
fn adv_key(s: &dyn AssetReader) -> Result<([u8; 8], u64, u64, u64)> {
    let raw = bytes(s, 53, 8 + 20 + 0x24, 64)?;
    let header_offset = 53 + 8 + 20;
    let mut header = raw[28..].to_vec();
    let mut key = [0u8; 8];
    for i in 0..8 {
        key[i] = header[i] ^ b"ARCHIVE\0"[i];
    }
    for (i, b) in header.iter_mut().enumerate() {
        *b ^= key[i % 8];
    }
    Ok((
        key,
        header_offset,
        u64::from(le32(&header, 16)?),
        u64::from(le32(&header, 32)?),
    ))
}
fn adv_probe(s: &dyn AssetReader) -> Result<bool> {
    let (_, header, table, count) = adv_key(s)?;
    Ok(header + table + 0x114 * count == length(s)?)
}
fn adv_dat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let (key, header_offset, table_offset, count) = adv_key(s)?;
    limits(cx, count as usize)?;
    let mut table = index(s, cx, header_offset + table_offset, 0x114 * count)?;
    for (i, b) in table.iter_mut().enumerate() {
        *b ^= key[(table_offset as usize + i) % 8];
    }
    table
        .chunks_exact(0x114)
        .map(|r| {
            let offset = u64::from(le32(r, 0x108)?) + header_offset;
            let start = (offset - header_offset) as usize;
            let plan = decode(move |input, _| {
                Ok(input
                    .iter()
                    .enumerate()
                    .map(|(i, b)| b ^ key[(start + i) % 8])
                    .collect())
            });
            let size = u64::from(le32(r, 0x10C)?);
            Ok(Row::packed(
                sjis(zstr(&r[1..0x108]))?,
                offset,
                size,
                size,
                plan,
            ))
        })
        .collect()
}

/// AST ARC1/ARC2. Version 2 inverts names and, unless the payload starts with
/// a PNG/RIFF/OggS signature, inverts data after optional bytewise LZSS.
fn ast_arc(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let size = length(s)?;
    let h = bytes(s, 0, 8, 8)?;
    let version = h[3].wrapping_sub(b'0');
    let count = le32(&h, 4)? as usize;
    limits(cx, count)?;
    let table = window(s, cx, 8)?;
    let mut c = Cursor::new(&table);
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = u64::from(c.le32()?);
        let orig = u64::from(c.le32()?);
        let n = c.u8()? as usize;
        let mut name = c.take(n)?.to_vec();
        if version == 2 {
            name.iter_mut().for_each(|b| *b ^= 0xFF);
        }
        items.push((sjis(&name)?, offset, orig));
    }
    let mut rows = Vec::with_capacity(count);
    for i in 0..items.len() {
        let (path, offset, orig) = items[i].clone();
        let end = items.get(i + 1).map_or(size, |n| n.1);
        let stored = end
            .checked_sub(offset)
            .context("AST offsets out of order")?;
        rows.push(if version != 2 {
            Row::raw(path, offset, stored)
        } else {
            let lzss = stored != orig;
            let plan = decode(move |input, entry| {
                let mut data = if lzss {
                    crysirror_resource::lzss_decompress(
                        input,
                        entry.size as usize,
                        &Default::default(),
                    )?
                } else {
                    input.to_vec()
                };
                let known = [&b"\x89PNG"[..], b"RIFF", b"OggS"]
                    .iter()
                    .any(|m| data.starts_with(m));
                if !known {
                    data.iter_mut().for_each(|b| *b ^= 0xFF);
                }
                Ok(data)
            });
            Row::packed(path, offset, stored, if lzss { orig } else { stored }, plan)
        });
    }
    Ok(rows)
}

const GXP_KEY: &[u8; 23] =
    b"\x40\x21\x28\x38\xA6\x6E\x43\xA5\x40\x21\x28\x38\xA6\x43\xA5\x64\x3E\x65\x24\x20\x46\x6E\x74";
fn gxp_crypt(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ GXP_KEY[i % 23] ^ i as u8)
        .collect()
}
fn gxp(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 24, 24, 24)?;
    let count = le32(&h, 0)? as usize;
    let table_size = u64::from(le32(&h, 4)?);
    let data_offset = u64::from_le_bytes(h[16..24].try_into().unwrap());
    limits(cx, count)?;
    let table = index(s, cx, 48, table_size)?;
    let mut at = 0usize;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let len = le32(
            &gxp_crypt(table.get(at..at + 4).context("truncated GXP table")?),
            0,
        )? as usize;
        let e = gxp_crypt(table.get(at..at + len).context("truncated GXP table")?);
        at += len;
        let size = u64::from(le32(&e, 4)?);
        let name_size = le32(&e, 12)? as usize;
        let offset =
            data_offset + u64::from_le_bytes(e.get(24..32).context("GXP entry")?.try_into()?);
        let path = utf16(e.get(32..32 + name_size * 2).context("GXP name")?)?;
        rows.push(Row::packed(
            path,
            offset,
            size,
            size,
            decode(|input, _| Ok(gxp_crypt(input))),
        ));
    }
    Ok(rows)
}

/// Bishop BSA: `>dir` enters and `<` leaves a directory; directory records
/// carry no offset/size in version 2.
fn bsa(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 8, 8, 8)?;
    let version = u16::from_le_bytes([h[0], h[1]]);
    let count = u64::from(u16::from_le_bytes([h[2], h[3]]));
    let table_offset = u64::from(le32(&h, 4)?);
    let table = window(s, cx, table_offset)?;
    let names_at = (count * 12) as usize;
    let mut c = Cursor::new(&table);
    let mut dir: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    let enter = |dir: &mut Vec<String>, name: &str| {
        if let Some(sub) = name.strip_prefix('>') {
            dir.push(sub.to_string());
            true
        } else if name == "<" {
            dir.pop();
            true
        } else {
            false
        }
    };
    let join = |dir: &[String], name: &str| {
        dir.iter()
            .map(String::as_str)
            .chain([name])
            .collect::<Vec<_>>()
            .join("/")
    };
    for _ in 0..count {
        match version {
            2 => {
                let name = sjis(zstr(c.take(32)?))?;
                if !enter(&mut dir, &name) {
                    let (offset, size) = (c.le32()?, c.le32()?);
                    rows.push(Row::raw(join(&dir, &name), offset.into(), size.into()));
                }
            }
            3 => {
                let name_offset = c.le32()? as usize;
                let (offset, size) = (c.le32()?, c.le32()?);
                let raw = table
                    .get(names_at + name_offset..)
                    .context("BSA name offset")?;
                ensure!(raw.contains(&0), "unterminated BSA name");
                let name = sjis(zstr(raw))?;
                if !enter(&mut dir, &name) {
                    rows.push(Row::raw(join(&dir, &name), offset.into(), size.into()));
                }
            }
            _ => anyhow::bail!("unsupported BSA version {version}"),
        }
    }
    limits(cx, rows.len())?;
    Ok(rows)
}

fn gs_table(
    s: &dyn AssetReader,
    cx: &DecodeContext,
    at: u64,
    comp: u64,
    key: u32,
    orig: u64,
) -> Result<Vec<u8>> {
    let mut data = index(s, cx, at, comp)?;
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= (i as u32 & key) as u8;
    }
    lzss_index(&data, orig, cx, false)
}
fn gs_dat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0xA8, 36, 36)?;
    let count = le32(&h, 0)? as usize;
    limits(cx, count)?;
    let table = gs_table(
        s,
        cx,
        le32(&h, 16)?.into(),
        le32(&h, 20)?.into(),
        le32(&h, 24)?,
        le32(&h, 28)?.into(),
    )?;
    let data_offset = u64::from(le32(&h, 32)?);
    (0..count)
        .map(|i| {
            let r = table
                .get(i * 0x18..i * 0x18 + 12)
                .context("truncated GS table")?;
            Ok(Row::packed(
                format!("{i:05}.dat"),
                u64::from(le32(r, 0)?) + data_offset,
                le32(r, 4)?.into(),
                le32(r, 8)?.into(),
                Plan::Lzss(serde_json::Value::Null),
            ))
        })
        .collect()
}
fn gs_pak(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0x30, 24, 24)?;
    let version = le32(&h, 0)?;
    let entry_size = if version >> 16 < 5 { 0x48 } else { 0x68 };
    let count = le32(&h, 12)? as usize;
    limits(cx, count)?;
    let data_offset = u64::from(le32(&h, 16)?);
    let table = gs_table(
        s,
        cx,
        le32(&h, 20)?.into(),
        le32(&h, 4)?.into(),
        le32(&h, 8)?,
        (count * entry_size) as u64,
    )?;
    table
        .chunks_exact(entry_size)
        .map(|r| {
            Ok(Row::raw(
                sjis(zstr(&r[..0x40]))?,
                u64::from(le32(r, 0x40)?) + data_offset,
                le32(r, 0x44)?.into(),
            ))
        })
        .collect()
}

fn gsp_probe(s: &dyn AssetReader) -> Result<bool> {
    let count = u64::from(u32_at(s, 0)?);
    ensure!(count > 0);
    let last = bytes(s, 4 + (count - 1) * 0x40, 8, 8)?;
    Ok(u64::from(le32(&last, 0)?) + u64::from(le32(&last, 4)?) == length(s)?)
}
fn gsp(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 0)?);
    limits(cx, count as usize)?;
    index(s, cx, 4, count * 0x40)?
        .chunks_exact(0x40)
        .map(|r| {
            Ok(Row::raw(
                sjis(zstr(&r[8..]))?,
                le32(r, 0)?.into(),
                le32(r, 4)?.into(),
            ))
        })
        .collect()
}

fn isa(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 12, 2, 2)?;
    let count = u64::from(u16::from_le_bytes([h[0], h[1]]));
    index(s, cx, 16, count * 32)?
        .chunks_exact(32)
        .map(|r| {
            Ok(Row::raw(
                sjis(zstr(&r[..12]))?,
                le32(r, 16)?.into(),
                le32(r, 20)?.into(),
            ))
        })
        .collect()
}

fn libido_probe(s: &dyn AssetReader) -> Result<bool> {
    let count = u64::from(u32_at(s, 0)?);
    ensure!(count > 0);
    let last = bytes(s, 4 + (count - 1) * 32 + 24, 8, 8)?;
    Ok(u64::from(le32(&last, 0)?) + u64::from(le32(&last, 4)?) == length(s)?)
}
fn libido_arc(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 0)?);
    limits(cx, count as usize)?;
    index(s, cx, 4, count * 32)?
        .chunks_exact(32)
        .map(|r| {
            let key = r[19];
            let name: Vec<u8> = r[..20].iter().map(|b| b ^ key).collect();
            Ok(Row::packed(
                sjis(zstr(&name))?,
                le32(r, 28)?.into(),
                le32(r, 24)?.into(),
                le32(r, 20)?.into(),
                Plan::Lzss(serde_json::Value::Null),
            ))
        })
        .collect()
}

/// Lilim AOS1: records whose name starts with 0xFF skip an embedded block;
/// data offsets are relative to the end of their own record.
fn aos1(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let size = length(s)?;
    let mut at = 0u64;
    let mut rows = Vec::new();
    while at < size {
        let raw = bytes(s, at, 0x10.min(size - at), 0x10)?;
        ensure!(raw.len() == 0x10, "truncated AOS record");
        let name = zstr(&raw).to_vec();
        if name.is_empty() {
            break;
        }
        let rec = bytes(s, at + 0x10, 16, 16)?;
        at += 0x20;
        if name[0] == 0xFF {
            at += u64::from(le32(&rec, 0)?);
            continue;
        }
        rows.push(Row::raw(
            sjis(&name)?,
            u64::from(le32(&rec, 0)?) + at,
            le32(&rec, 4)?.into(),
        ));
        limits(cx, rows.len())?;
    }
    Ok(rows)
}

fn aos2_probe(s: &dyn AssetReader) -> Result<bool> {
    let h = bytes(s, 0, 8, 8)?;
    ensure!(le32(&h, 0)? == 0);
    let data = u64::from(le32(&h, 4)?);
    let last = bytes(s, data.checked_sub(8).context("AOS2")?, 8, 8)?;
    Ok(data + u64::from(le32(&last, 0)?) + u64::from(le32(&last, 4)?) == length(s)?)
}
fn aos2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 4, 8, 8)?;
    let data = u64::from(le32(&h, 0)?);
    let table_size = u64::from(le32(&h, 4)?);
    let count = table_size / 0x28;
    limits(cx, count as usize)?;
    index(
        s,
        cx,
        data.checked_sub(table_size).context("AOS2 table offset")?,
        count * 0x28,
    )?
    .chunks_exact(0x28)
    .map(|r| {
        Ok(Row::raw(
            sjis(zstr(&r[..0x20]))?,
            u64::from(le32(r, 0x20)?) + data,
            le32(r, 0x24)?.into(),
        ))
    })
    .collect()
}

fn dpk_probe(s: &dyn AssetReader) -> Result<bool> {
    Ok(magic(s, 0, b"PA")? && u64::from(u32_at(s, 4)?) == length(s)?)
}
fn dpk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 2, 2, 2)?;
    let count = u64::from(u16::from_le_bytes([h[0], h[1]]));
    let mut offset = 8 + count * 0x14;
    index(s, cx, 8, count * 0x14)?
        .chunks_exact(0x14)
        .map(|r| {
            let size = u64::from(le32(r, 0x10)?);
            let row = Row::raw(sjis(zstr(&r[..0x10]))?, offset, size);
            offset += size;
            Ok(row)
        })
        .collect()
}

fn mages_mpk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 4, 8, 8)?;
    let version = (
        u16::from_le_bytes([h[2], h[3]]),
        u16::from_le_bytes([h[0], h[1]]),
    );
    let count = u64::from(le32(&h, 4)?);
    limits(cx, count as usize)?;
    let mut rows = Vec::new();
    for r in index(s, cx, 64, count * 256)?.chunks_exact(256) {
        let offset = u64::from_le_bytes(r[8..16].try_into().unwrap());
        let size = u64::from_le_bytes(r[16..24].try_into().unwrap());
        if offset != 0 && size != 0 {
            rows.push(
                Row::raw(sjis(zstr(&r[32..]))?, offset, size)
                    .meta(serde_json::json!({"version": [version.0, version.1]})),
            );
        }
    }
    Ok(rows)
}

fn riddle_pac(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 4)?);
    limits(cx, count as usize)?;
    let mut offset = 8 + count * 32;
    index(s, cx, 8, count * 32)?
        .chunks_exact(32)
        .map(|r| {
            ensure!(le32(r, 24)? == le32(r, 28)?, "Data mismatch");
            let size = u64::from(le32(r, 16)?);
            let row = Row::raw(sjis(zstr(&r[..16]))?, offset, size);
            offset += size;
            Ok(row)
        })
        .collect()
}

fn sysadv_pak(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u32_at(s, 6)? as usize;
    limits(cx, count)?;
    let table = window(s, cx, 10)?;
    let mut c = Cursor::new(&table);
    (0..count)
        .map(|_| {
            let n = c.u8()? as usize;
            let name: Vec<u8> = c.take(n)?.iter().map(|b| b ^ 0xFF).collect();
            Ok(Row::raw(sjis(&name)?, c.le32()?.into(), c.le32()?.into()))
        })
        .collect()
}

fn tabito_dat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let total = length(s)?;
    let mut at = 0u64;
    let mut rows = Vec::new();
    loop {
        let h = bytes(s, at, 10.min(total - at), 10)?;
        let size = u64::from(le32(&h, 0)?);
        if size == 0 {
            break;
        }
        ensure!(h.len() == 10, "truncated Tabito record");
        let n = u64::from(u16::from_le_bytes([h[8], h[9]]));
        let name = index(s, cx, at + 10, n)?;
        at += 10 + n;
        rows.push(Row::raw(sjis(zstr(&name))?, at, size));
        at += size;
        limits(cx, rows.len())?;
    }
    Ok(rows)
}

fn yumemiru_dat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 8)?);
    limits(cx, count as usize)?;
    index(s, cx, 12, count * 0x10C)?
        .chunks_exact(0x10C)
        .map(|r| {
            let (offset, orig, comp) = (le32(r, 0x100)?, le32(r, 0x104)?, le32(r, 0x108)?);
            let path = sjis(zstr(&r[..0x100]))?;
            Ok(if orig == comp {
                Row::raw(path, offset.into(), comp.into())
            } else {
                Row::packed(
                    path,
                    offset.into(),
                    comp.into(),
                    orig.into(),
                    Plan::Unsupported(
                        "Yumemiru compressed entries are not supported by the pinned decoder",
                    ),
                )
            })
        })
        .collect()
}

fn myk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 8, 6, 6)?;
    let count = u64::from(u16::from_le_bytes([h[0], h[1]]));
    let mut offset = 16u64;
    index(s, cx, le32(&h, 2)?.into(), count * 16)?
        .chunks_exact(16)
        .map(|r| {
            let size = u64::from(le32(r, 12)?);
            let row = Row::raw(sjis(zstr(&r[..12]))?, offset, size);
            offset += size;
            Ok(row)
        })
        .collect()
}

fn crowd_pck(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 0)?);
    limits(cx, count as usize)?;
    let table = window(s, cx, 4)?;
    let mut c = Cursor::new(&table);
    let mut items = Vec::with_capacity(count as usize);
    for _ in 0..count {
        c.take(4)?;
        items.push((c.le32()?, c.le32()?));
    }
    items
        .into_iter()
        .map(|(offset, size)| Ok(Row::raw(sjis(c.zero()?)?, offset.into(), size.into())))
        .collect()
}

fn acp_pk1(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 8)?);
    limits(cx, count as usize)?;
    index(s, cx, 12, count * 40)?
        .chunks_exact(40)
        .map(|r| {
            Ok(Row::raw(
                sjis(zstr(&r[..32]))?,
                le32(r, 32)?.into(),
                le32(r, 36)?.into(),
            ))
        })
        .collect()
}

fn dogenzaka_bin(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 0)?);
    limits(cx, count as usize)?;
    let mut rows = if u32_at(s, 4)? == 0 {
        let data_start = 4 + 4 * count;
        let mut rows: Vec<Row> = index(s, cx, 4, count * 4)?
            .chunks_exact(4)
            .map(|r| {
                Ok(Row::raw(
                    String::new(),
                    u64::from(le32(r, 0)?) + data_start,
                    0,
                ))
            })
            .collect::<Result<_>>()?;
        chain_sizes(&mut rows, length(s)?)?;
        rows
    } else {
        index(s, cx, 4, count * 8)?
            .chunks_exact(8)
            .map(|r| {
                let size = u64::from(le32(r, 4)?)
                    .checked_sub(12)
                    .context("dogenzaka entry size")?;
                Ok(Row::raw(String::new(), u64::from(le32(r, 0)?) + 12, size))
            })
            .collect::<Result<_>>()?
    };
    name_unnamed(&mut rows);
    Ok(rows)
}

/// GLib GML: inverted, negated-length LZSS table with a byte permutation applied
/// to everything after each entry's 4 plaintext prefix bytes.
fn gml(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 8, 12, 12)?;
    let data_start = u64::from(le32(&h, 0)?);
    let mut packed = index(s, cx, 20, le32(&h, 8)?.into())?;
    packed.iter_mut().for_each(|b| *b ^= 0xFF);
    let table = lzss_index(&packed, le32(&h, 4)?.into(), cx, true)?;
    let mut c = Cursor::new(&table);
    let permutation: Arc<[u8]> = Arc::from(c.take(0x100)?);
    let count = c.le32()? as usize;
    limits(cx, count)?;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let n = c.le32()? as usize;
        let path = sjis(c.take(n)?)?;
        let offset = u64::from(c.le32()?) + data_start;
        let size = u64::from(c.le32()?);
        ensure!(size >= 4, "GML entry shorter than its prefix");
        let prefix: [u8; 4] = c.take(4)?.try_into()?;
        let p = permutation.clone();
        let plan = decode(move |input, _| {
            let mut out = prefix.to_vec();
            out.extend(input.iter().map(|b| p[*b as usize]));
            Ok(out)
        });
        rows.push(Row::packed(path, offset + 4, size - 4, size, plan));
    }
    Ok(rows)
}

fn gpk2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let table_offset = u64::from(u32_at(s, 4)?);
    let count = u64::from(u32_at(s, table_offset)?);
    limits(cx, count as usize)?;
    index(s, cx, table_offset + 4, count * 0x88)?
        .chunks_exact(0x88)
        .map(|r| {
            Ok(Row::raw(
                sjis(zstr(&r[8..]))?,
                le32(r, 0)?.into(),
                le32(r, 4)?.into(),
            ))
        })
        .collect()
}
