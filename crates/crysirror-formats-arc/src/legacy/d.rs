// SPDX-License-Identifier: GPL-3.0-or-later
// Layouts ported from arc_unpacker, Copyright (C) 2016 rr-:
// real-live/{ovk,nwk}, rpgmaker/{rgssad,rgss3a}, cri/{afs,afs2},
// minato-soft/pac, system-epsilon/packdat, innocent-grey/{packdat,iga},
// wild-bug/wbp.
use super::*;
use std::collections::BTreeMap;

pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "real-live/ovk",
        probe: None,
        open: ovk,
    },
    Legacy {
        id: "real-live/nwk",
        probe: None,
        open: nwk,
    },
    Legacy {
        id: "rpgmaker/rgssad",
        probe: Some(|s| magic(s, 0, b"RGSSAD\0\x01")),
        open: rgssad,
    },
    Legacy {
        id: "rpgmaker/rgss3a",
        probe: Some(|s| magic(s, 0, b"RGSSAD\0\x03")),
        open: rgss3a,
    },
    Legacy {
        id: "cri/afs",
        probe: Some(|s| magic(s, 0, b"AFS\0")),
        open: afs,
    },
    Legacy {
        id: "cri/afs2",
        probe: Some(|s| magic(s, 0, b"AFS2")),
        open: afs2,
    },
    Legacy {
        id: "minato-soft/pac",
        probe: Some(|s| magic(s, 0, b"PAC\0")),
        open: minato_pac,
    },
    Legacy {
        id: "system-epsilon/packdat",
        probe: Some(|s| magic(s, 0, b"PACKDAT.")),
        open: packdat,
    },
    // Same signature, layout and cipher as system-epsilon/packdat; upstream
    // registers both. Auto-detection resolves to system-epsilon.
    Legacy {
        id: "innocent-grey/packdat",
        probe: None,
        open: packdat,
    },
    Legacy {
        id: "innocent-grey/iga",
        probe: Some(|s| magic(s, 0, b"IGA0")),
        open: iga,
    },
    Legacy {
        id: "wild-bug/wbp",
        probe: Some(|s| magic(s, 0, b"ARCFORM4 WBUG ")),
        open: wbp,
    },
];

fn u32_at(s: &dyn AssetReader, at: u64) -> Result<u32> {
    le32(&bytes(s, at, 4, 4)?, 0)
}

/// RealLive voice archives: numbered samples (OGG in OVK, NWA in NWK).
fn reallive(s: &dyn AssetReader, cx: &DecodeContext, record: u64, ext: &str) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 0)?);
    limits(cx, count as usize)?;
    index(s, cx, 4, count * record)?
        .chunks_exact(record as usize)
        .map(|r| {
            let id = le32(r, 8)?;
            Ok(Row::raw(
                format!("sample{id:05}{ext}"),
                le32(r, 4)?.into(),
                le32(r, 0)?.into(),
            )
            .meta(serde_json::json!({"sample": id})))
        })
        .collect()
}
fn ovk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    reallive(s, cx, 16, "")
}
fn nwk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    reallive(s, cx, 12, ".nwa")
}

fn rgs_next(key: u32) -> u32 {
    key.wrapping_mul(7).wrapping_add(3)
}
fn rgs_plan(key: u32) -> Plan {
    decode(move |input, _| {
        let mut key = key;
        let mut out = Vec::with_capacity(input.len() + 3);
        for chunk in input.chunks(4) {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            out.extend((u32::from_le_bytes(w) ^ key).to_le_bytes());
            key = rgs_next(key);
        }
        out.truncate(input.len());
        Ok(out)
    })
}
fn rgs_name(raw: Vec<u8>) -> Result<String> {
    Ok(String::from_utf8(raw)
        .context("RGSS name is not UTF-8")?
        .replace('\\', "/"))
}
fn rgssad(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let size = length(s)?;
    let mut at = 8u64;
    let mut key = 0xDEADCAFEu32;
    let mut rows = Vec::new();
    while at < size {
        let name_size = u32_at(s, at)? ^ key;
        key = rgs_next(key);
        let mut name = index(s, cx, at + 4, name_size.into())?;
        for b in &mut name {
            *b ^= key as u8;
            key = rgs_next(key);
        }
        at += 4 + u64::from(name_size);
        let stored = u64::from(u32_at(s, at)? ^ key);
        key = rgs_next(key);
        at += 4;
        rows.push(Row::packed(
            rgs_name(name)?,
            at,
            stored,
            stored,
            rgs_plan(key),
        ));
        at += stored;
        limits(cx, rows.len())?;
    }
    Ok(rows)
}
fn rgss3a(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let size = length(s)?;
    let key = u32_at(s, 8)?.wrapping_mul(9).wrapping_add(3);
    let mut at = 12u64;
    let mut rows = Vec::new();
    while at < size {
        let offset = u32_at(s, at)? ^ key;
        if offset == 0 {
            break;
        }
        let h = bytes(s, at + 4, 12, 12)?;
        let stored = u64::from(le32(&h, 0)? ^ key);
        let entry_key = le32(&h, 4)? ^ key;
        let name_size = le32(&h, 8)? ^ key;
        let name: Vec<u8> = index(s, cx, at + 16, name_size.into())?
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key.wrapping_shr(i as u32 * 8) as u8)
            .collect();
        at += 16 + u64::from(name_size);
        rows.push(Row::packed(
            rgs_name(name)?,
            offset.into(),
            stored,
            stored,
            rgs_plan(entry_key),
        ));
        limits(cx, rows.len())?;
    }
    Ok(rows)
}

fn afs(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 4)?);
    limits(cx, count as usize)?;
    let table = index(s, cx, 8, count * 8 + 4)?;
    let names = index(
        s,
        cx,
        u64::from(le32(&table, count as usize * 8)?),
        count * 48,
    )?;
    table[..count as usize * 8]
        .chunks_exact(8)
        .zip(names.chunks_exact(48))
        .map(|(r, n)| {
            Ok(Row::raw(
                sjis(zstr(&n[..32]))?,
                le32(r, 0)?.into(),
                le32(r, 4)?.into(),
            ))
        })
        .collect()
}
/// AFS2: 32-byte aligned data; each size is measured from the aligned start
/// to the next raw (unaligned) offset, exactly as the upstream reader does.
fn afs2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 8)?)
        .checked_sub(1)
        .context("AFS2 file count")?;
    limits(cx, count as usize)?;
    let table = index(s, cx, 16 + (count + 1) * 2, count * 4)?;
    let raw: Vec<u64> = table
        .chunks_exact(4)
        .map(|r| u64::from(le32(r, 0).unwrap()))
        .collect();
    let size = length(s)?;
    (0..raw.len())
        .map(|i| {
            let offset = (raw[i] + 0x1F) & !0x1F;
            let end = raw.get(i + 1).copied().unwrap_or(size);
            Ok(Row::raw(
                format!("{i}.dat"),
                offset,
                end.checked_sub(offset)
                    .context("AFS2 offsets out of order")?,
            ))
        })
        .collect()
}

/// Minato Soft PAC: an inverted, Huffman-coded table at the end of the file.
fn minato_table(input: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut bit = 0usize;
    let mut read = |n: usize| -> Result<u32> {
        let mut v = 0;
        for _ in 0..n {
            let byte = *input
                .get(bit / 8)
                .context("truncated Minato Huffman table")?;
            v = (v << 1) | u32::from((byte >> (7 - bit % 8)) & 1);
            bit += 1;
        }
        Ok(v)
    };
    let mut nodes = [[0u16; 512]; 2];
    fn tree(
        read: &mut dyn FnMut(usize) -> Result<u32>,
        nodes: &mut [[u16; 512]; 2],
        pos: &mut usize,
        depth: usize,
    ) -> Result<i32> {
        ensure!(depth < 1024, "Minato Huffman tree too deep");
        if read(1)? != 0 {
            let old = *pos;
            *pos += 1;
            if old < 511 {
                nodes[0][old] = tree(read, nodes, pos, depth + 1)? as u16;
                nodes[1][old] = tree(read, nodes, pos, depth + 1)? as u16;
                return Ok(old as i32);
            }
            return Ok(-1);
        }
        Ok(read(8)? as i32)
    }
    let mut pos = 256usize;
    let root = tree(&mut read, &mut nodes, &mut pos, 0)?;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let mut p = root;
        while (256..=511).contains(&p) {
            p = i32::from(nodes[read(1)? as usize][p as usize]);
        }
        out.push(p as u8);
    }
    Ok(out)
}
fn minato_pac(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let size = length(s)?;
    let count = u64::from(u32_at(s, 4)?);
    limits(cx, count as usize)?;
    let comp = u64::from(u32_at(s, size.checked_sub(4).context("Minato PAC size")?)?);
    let mut packed = index(
        s,
        cx,
        (size - 4)
            .checked_sub(comp)
            .context("Minato table offset")?,
        comp,
    )?;
    packed.iter_mut().for_each(|b| *b ^= 0xFF);
    ensure!(
        count * 76 <= cx.limits.max_index_size,
        "archive index byte limit"
    );
    let table = minato_table(&packed, (count * 76) as usize)?;
    table
        .chunks_exact(76)
        .map(|r| {
            let (offset, orig, stored) = (le32(r, 64)?, le32(r, 68)?, le32(r, 72)?);
            let path = sjis(zstr(&r[..64]))?;
            Ok(if orig == stored {
                Row::raw(path, offset.into(), stored.into())
            } else {
                Row::packed(path, offset.into(), stored.into(), orig.into(), Plan::Zlib)
            })
        })
        .collect()
}

/// PACKDAT: flag 0x10000 marks a dword cipher whose key rotates by each
/// decrypted dword mod 24; trailing bytes past the last dword stay plain.
fn packdat_decrypt(input: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    let n = (out.len() >> 2) as u32;
    let mut key = (n << ((n & 7) + 8)) ^ n;
    for w in out.chunks_exact_mut(4) {
        let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]) ^ key;
        w.copy_from_slice(&v.to_le_bytes());
        key = key.rotate_left(v % 24);
    }
    out
}
fn packdat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(u32_at(s, 8)?);
    limits(cx, count as usize)?;
    index(s, cx, 16, count * 48)?
        .chunks_exact(48)
        .map(|r| {
            let (offset, flags, orig, comp) =
                (le32(r, 32)?, le32(r, 36)?, le32(r, 40)?, le32(r, 44)?);
            let path = sjis(zstr(&r[..32]))?;
            let plan = if orig != comp {
                Plan::Unsupported(
                    "PACKDAT compressed entries are not supported by the pinned decoder",
                )
            } else if flags & 0x10000 != 0 {
                decode(|input, _| Ok(packdat_decrypt(input)))
            } else {
                Plan::Raw
            };
            Ok(
                Row::packed(path, offset.into(), comp.into(), orig.into(), plan)
                    .meta(serde_json::json!({"flags": flags})),
            )
        })
        .collect()
}

/// IGA: 7-bit varints terminated by a set low bit; names are varint chars.
fn varint(c: &mut Cursor) -> Result<u32> {
    let mut v = 0u32;
    while v & 1 == 0 {
        ensure!(v < 1 << 25, "IGA integer overflow");
        v = (v << 7) | u32::from(c.u8()?);
    }
    Ok(v >> 1)
}
fn iga(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let table = window(s, cx, 16)?;
    let mut c = Cursor::new(&table);
    let table_size = varint(&mut c)? as usize;
    let table_end = c.at + table_size;
    let mut specs = Vec::new();
    while c.at < table_end {
        specs.push((
            varint(&mut c)? as usize,
            u64::from(varint(&mut c)?),
            u64::from(varint(&mut c)?),
        ));
        limits(cx, specs.len())?;
    }
    let Some(last) = specs.last().copied() else {
        return Ok(Vec::new());
    };
    let names_size = varint(&mut c)? as usize;
    let names_start = c.at;
    let data_offset = length(s)?
        .checked_sub(last.1 + last.2)
        .context("IGA data offset")?;
    (0..specs.len())
        .map(|i| {
            let (name_at, offset, size) = specs[i];
            let end = specs.get(i + 1).map_or(names_size, |n| n.0);
            let mut n = Cursor::new(&table);
            n.at = names_start + name_at;
            let name = (0..end.checked_sub(name_at).context("IGA name order")?)
                .map(|_| varint(&mut n).map(|v| v as u8))
                .collect::<Result<Vec<u8>>>()?;
            let plan = decode(|input, _| {
                Ok(input
                    .iter()
                    .enumerate()
                    .map(|(i, b)| b ^ (i + 2) as u8)
                    .collect())
            });
            Ok(Row::packed(
                sjis(&name)?,
                data_offset + offset,
                size,
                size,
                plan,
            ))
        })
        .collect()
}

fn wbp(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0x10, 20 + 0x800, 20 + 0x800)?;
    let count = le32(&h, 0)? as usize;
    limits(cx, count)?;
    let offsets = |from: usize| -> Vec<u64> {
        h[from..from + 0x400]
            .chunks_exact(4)
            .map(|r| u64::from(le32(r, 0).unwrap()))
            .filter(|o| *o != 0)
            .collect()
    };
    let (dirs, files) = (offsets(20), offsets(20 + 0x400));
    let mut dir_names = BTreeMap::new();
    for offset in dirs {
        let r = bytes(s, offset + 1, 3, 3)?;
        let name = index(s, cx, offset + 4, r[0].into())?;
        dir_names.insert(r[1], sjis(zstr(&name))?);
    }
    let mut rows = Vec::new();
    for (i, start) in files.iter().enumerate() {
        let mut at = *start;
        loop {
            let r = bytes(s, at, 0x18, 0x18)?;
            let name_size = u64::from(r[1]);
            let dir = dir_names
                .get(&r[2])
                .context("WBP entry refers to an unknown directory")?;
            let name = index(s, cx, at + 0x14, name_size)?;
            let path = format!("{dir}{}", sjis(zstr(&name))?).replace('\\', "/");
            rows.push(Row::raw(path, le32(&r, 4)?.into(), le32(&r, 8)?.into()));
            limits(cx, rows.len())?;
            at += (name_size & 0xFC) + 0x18;
            if !files.get(i + 1).is_some_and(|next| at < *next) {
                break;
            }
        }
    }
    Ok(rows)
}
