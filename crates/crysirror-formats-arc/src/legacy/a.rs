// SPDX-License-Identifier: GPL-3.0-or-later
// Layouts ported from arc_unpacker, Copyright (C) 2016 rr-:
// alice-soft/{ald,afa,alk,dat}, kid/lnk, will/{arc-will,arc-pulltop},
// nitroplus/pak, fvp/bin.
use super::*;
use anyhow::bail;

pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "alice-soft/ald",
        probe: None,
        open: ald,
    },
    Legacy {
        id: "alice-soft/afa",
        probe: Some(afa_probe),
        open: afa,
    },
    Legacy {
        id: "alice-soft/alk",
        probe: Some(|s| magic(s, 0, b"ALK0")),
        open: alk,
    },
    Legacy {
        id: "alice-soft/dat",
        probe: None,
        open: alice_dat,
    },
    Legacy {
        id: "kid/lnk",
        probe: Some(|s| magic(s, 0, b"LNK\0")),
        open: lnk,
    },
    Legacy {
        id: "will/arc-will",
        probe: Some(|s| {
            Ok(!arc_will(s, &probe_context())
                .ok()
                .unwrap_or_default()
                .is_empty())
        }),
        open: arc_will,
    },
    Legacy {
        id: "will/arc-pulltop",
        probe: Some(pulltop_probe),
        open: pulltop,
    },
    Legacy {
        id: "nitroplus/pak",
        probe: Some(nitro_probe),
        open: nitro_pak,
    },
    Legacy {
        id: "fvp/bin",
        probe: None,
        open: fvp_bin,
    },
];

pub(crate) fn probe_context() -> DecodeContext {
    let mut context = DecodeContext::new(crysirror_core::state::ClientId(0));
    context.limits.max_index_size = 256 * 1024;
    context
}

/// Alice Soft ALD: 24-bit sector offsets (value << 8); the first one is the
/// byte size of the offset table itself. A zero offset ends the list.
fn ald(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let r24 = |b: &[u8]| (u64::from(b[0]) << 8) | (u64::from(b[1]) << 16) | (u64::from(b[2]) << 24);
    let count = r24(&bytes(s, 0, 3, 3)?) / 3;
    limits(cx, count as usize)?;
    let table = index(s, cx, 3, count * 3)?;
    let size = length(s)?;
    let mut rows = Vec::new();
    for raw in table.chunks_exact(3) {
        let offset = r24(raw);
        if offset == 0 {
            break;
        }
        let header_size = u64::from(le32(&bytes(s, offset, 4, 4)?, 0)?);
        if size - (offset + 4) >= header_size {
            ensure!(header_size >= 16, "ALD entry header too small");
            let header = index(s, cx, offset, header_size)?;
            let path = sjis(zstr(&header[16..]))?;
            rows.push(Row::raw(
                path,
                offset + header_size,
                u64::from(le32(&header, 4)?),
            ));
        }
    }
    Ok(rows)
}

fn afa_probe(s: &dyn AssetReader) -> Result<bool> {
    Ok(magic(s, 0, b"AFAH")? && magic(s, 8, b"AlicArch")?)
}
fn afa(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 44, 44)?;
    let mut c = Cursor::new(&h);
    ensure!(c.take(4)? == b"AFAH", "missing AFAH");
    c.take(4)?;
    ensure!(c.take(8)? == b"AlicArch", "missing AlicArch");
    let version = c.le32()?;
    c.take(4)?;
    let data_start = u64::from(c.le32()?);
    ensure!(c.take(4)? == b"INFO", "Corrupt file table");
    let comp = u64::from(c.le32()?);
    let _orig = c.le32()?;
    let count = c.le32()? as usize;
    limits(cx, count)?;
    let table = inflate_index(&index(s, cx, 44, comp)?, cx.limits.max_index_size)?;
    let mut t = Cursor::new(&table);
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        t.take(4)?;
        let name_size = t.le32()? as usize;
        let path = sjis(zstr(t.take(name_size)?))?;
        t.take(if version == 1 { 12 } else { 8 })?;
        let offset = u64::from(t.le32()?) + data_start;
        rows.push(Row::raw(path, offset, u64::from(t.le32()?)));
    }
    Ok(rows)
}

fn alk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(le32(&bytes(s, 4, 4, 4)?, 0)?);
    limits(cx, count as usize)?;
    let table = index(s, cx, 8, count * 8)?;
    let mut rows = Vec::new();
    for pair in table.chunks_exact(8) {
        let (offset, size) = (le32(pair, 0)?, le32(pair, 4)?);
        if size != 0 {
            rows.push(Row::raw(
                format!("{:03}.dat", rows.len()),
                offset.into(),
                size.into(),
            ));
        }
    }
    Ok(rows)
}

/// Alice Soft System1-3 DAT: 256-byte sector table. Upstream also picked an
/// extension from the archive filename (cg→vsp, dis→sco, ...); this API only
/// sees bytes, so names keep the neutral `.dat` and callers may rename.
fn alice_dat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let size = length(s)?;
    let sectors = u64::from(u16::from_le_bytes(bytes(s, 0, 2, 2)?.try_into().unwrap()));
    ensure!(sectors >= 2, "Alice DAT header too small");
    let count = (sectors - 1) * 256 / 2 - 1;
    let table = index(s, cx, 2, count * 2)?;
    let mut offsets: Vec<(usize, u64)> = Vec::new();
    let mut finished = false;
    for (i, raw) in table.chunks_exact(2).enumerate() {
        let sector = u64::from(u16::from_le_bytes([raw[0], raw[1]]));
        if sector == 0 {
            finished = true;
            continue;
        }
        let offset = (sector - 1) * 256;
        ensure!(
            offsets.last().is_none_or(|(_, last)| offset >= *last),
            "Expected offsets to be sorted"
        );
        ensure!(!finished, "Expected remaining offsets to be 0");
        ensure!(offset <= size, "Bad data offset");
        if offset < size {
            offsets.push((i, offset));
        }
    }
    ensure!(!offsets.is_empty(), "File table cannot be empty");
    limits(cx, offsets.len())?;
    Ok(offsets
        .iter()
        .enumerate()
        .map(|(n, (i, offset))| {
            let end = offsets.get(n + 1).map_or(size, |(_, next)| *next);
            Row::raw(format!("{i:03}.dat"), *offset, end - offset)
        })
        .collect())
}

/// KID LNK: per-extension 256-byte subtractive keystream seeded by the sum of
/// the name bytes, then optional "lnd" RLE/LZ compression.
fn lnk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(le32(&bytes(s, 4, 4, 4)?, 0)?);
    limits(cx, count as usize)?;
    let table = index(s, cx, 16, count * 32)?;
    let data_start = 16 + count * 32;
    let mut rows = Vec::with_capacity(count as usize);
    for raw in table.chunks_exact(32) {
        let offset = u64::from(le32(raw, 0)?) + data_start;
        let tmp = le32(raw, 4)?;
        let stored = u64::from(tmp >> 1);
        let name = zstr(&raw[8..32]).to_vec();
        let path = sjis(&name)?;
        let key_pos = lnk_key_pos(&path);
        let key = name.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        let compressed = tmp & 1 != 0;
        let size = if compressed {
            let mut header = bytes(s, offset, 16.min(stored), 16)?;
            lnk_decrypt(&mut header, key_pos, key);
            ensure!(
                header.len() == 16 && &header[..4] == b"lnd\0",
                "KID LND header"
            );
            u64::from(le32(&header, 8)?)
        } else {
            stored
        };
        let plan = decode(move |input, entry| {
            let mut data = input.to_vec();
            lnk_decrypt(&mut data, key_pos, key);
            if compressed {
                lnd(&data[16..], entry.size as usize)
            } else {
                Ok(data)
            }
        });
        rows.push(
            Row::packed(path, offset, stored, size, plan)
                .meta(serde_json::json!({"compressed": compressed})),
        );
    }
    Ok(rows)
}
fn lnk_key_pos(path: &str) -> Option<usize> {
    if has_ext(path, "wav") {
        Some(0)
    } else if has_ext(path, "jpg") {
        Some(0x1100)
    } else if has_ext(path, "scr") {
        Some(0x1000)
    } else {
        None
    }
}
fn lnk_decrypt(data: &mut [u8], key_pos: Option<usize>, mut key: u8) {
    let Some(pos) = key_pos else { return };
    for b in data.iter_mut().skip(pos).take(0x100) {
        *b = b.wrapping_sub(key);
        key = key.wrapping_mul(0x6D).wrapping_sub(0x25);
    }
}
fn lnd(input: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; size];
    let (mut o, mut i) = (0usize, 0usize);
    let byte = |i: usize| input.get(i).copied().context("truncated KID LND data");
    while o < size && i < input.len() {
        let b = input[i];
        i += 1;
        if b & 0x80 != 0 {
            if b & 0x40 != 0 {
                let mut n = usize::from(b & 0x1F) + 2;
                if b & 0x20 != 0 {
                    n += usize::from(byte(i)?) << 5;
                    i += 1;
                }
                let v = byte(i)?;
                i += 1;
                for _ in 0..n.min(size - o) {
                    out[o] = v;
                    o += 1;
                }
            } else {
                let n = usize::from((b >> 2) & 0xF) + 2;
                let back = (usize::from(b & 3) << 8) + usize::from(byte(i)?) + 1;
                i += 1;
                ensure!(back <= o, "KID LND look-behind before output start");
                for _ in 0..n.min(size - o) {
                    out[o] = out[o - back];
                    o += 1;
                }
            }
        } else if b & 0x40 != 0 {
            let reps = usize::from(byte(i)?) + 1;
            i += 1;
            let n = usize::from(b & 0x3F) + 2;
            let chunk = input.get(i..i + n).context("truncated KID LND data")?;
            for _ in 0..reps {
                for v in chunk {
                    if o >= size {
                        break;
                    }
                    out[o] = out[o].wrapping_add(*v);
                    o += 1;
                }
            }
            i += n;
        } else {
            let mut n = usize::from(b & 0x1F) + 1;
            if b & 0x20 != 0 {
                n += usize::from(byte(i)?) << 5;
                i += 1;
            }
            for _ in 0..n {
                if o >= size {
                    break;
                }
                out[o] = byte(i)?;
                o += 1;
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Will ARC: per-extension directories with 9- or 13-byte names. Scripts
/// (`wsc`, `scr`) are stored rotated left by two bits.
fn arc_will(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let dir_count = u64::from(le32(&bytes(s, 0, 4, 4)?, 0)?);
    ensure!(dir_count <= 100, "Bad data size");
    let table = index(s, cx, 4, dir_count * 12)?;
    let mut dirs = Vec::new();
    for raw in table.chunks_exact(12) {
        dirs.push((
            ascii(zstr(&raw[..4]))?,
            u64::from(le32(raw, 4)?),
            u64::from(le32(raw, 8)?),
        ));
    }
    let size = length(s)?;
    'names: for name_size in [9u64, 13] {
        let mut min_offset = 4 + dir_count * 12;
        for (_, count, _) in &dirs {
            min_offset += count * (name_size + 8);
        }
        let mut rows = Vec::new();
        for (ext, count, at) in &dirs {
            limits(cx, rows.len() + *count as usize)?;
            let Ok(raw) = index(s, cx, *at, count * (name_size + 8)) else {
                continue 'names;
            };
            for entry in raw.chunks_exact(name_size as usize + 8) {
                let Ok(name) = sjis(zstr(&entry[..name_size as usize])) else {
                    continue 'names;
                };
                let path = format!("{name}.{ext}");
                let n = name_size as usize;
                let (stored, offset) = (u64::from(le32(entry, n)?), u64::from(le32(entry, n + 4)?));
                if offset < min_offset || offset + stored > size {
                    continue 'names;
                }
                let plan = if has_ext(&path, "wsc") || has_ext(&path, "scr") {
                    decode(|input, _| Ok(input.iter().map(|c| c.rotate_right(2)).collect()))
                } else {
                    Plan::Raw
                };
                rows.push(Row::packed(path, offset, stored, stored, plan));
            }
        }
        return Ok(rows);
    }
    bail!("Failed to read file table")
}

fn utf16z(c: &mut Cursor) -> Result<String> {
    let mut raw = Vec::new();
    while c.left() > 0 {
        let chunk = c.take(2.min(c.left()))?;
        if chunk == [0, 0] {
            break;
        }
        raw.extend_from_slice(chunk);
    }
    raw.truncate(raw.len() & !1);
    utf16(&raw)
}
fn pulltop_probe(s: &dyn AssetReader) -> Result<bool> {
    let rows = pulltop(s, &probe_context())?;
    let last = rows.last().context("empty")?;
    Ok(last.offset + last.stored == length(s)?)
}
fn pulltop(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 8, 8)?;
    let (count, table_size) = (le32(&h, 0)? as usize, u64::from(le32(&h, 4)?));
    limits(cx, count)?;
    let table = index(s, cx, 8, table_size)?;
    let mut c = Cursor::new(&table);
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let size = u64::from(c.le32()?);
        let offset = u64::from(c.le32()?) + 8 + table_size;
        rows.push(Row::raw(utf16z(&mut c)?, offset, size));
    }
    Ok(rows)
}

fn nitro_probe(s: &dyn AssetReader) -> Result<bool> {
    Ok(magic(s, 0, b"\x02\0\0\0")? && !nitro_pak(s, &probe_context())?.is_empty())
}
fn nitro_pak(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 16, 16)?;
    ensure!(&h[..4] == b"\x02\0\0\0", "Nitroplus PAK magic");
    let count = le32(&h, 4)? as usize;
    let orig = u64::from(le32(&h, 8)?);
    let comp = u64::from(le32(&h, 12)?);
    limits(cx, count)?;
    ensure!(orig <= cx.limits.max_index_size, "archive index byte limit");
    let table_at = 16 + 0x104;
    let table = inflate_index(&index(s, cx, table_at, comp)?, orig)?;
    let data_at = table_at + comp;
    let mut t = Cursor::new(&table);
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let name_size = t.le32()? as usize;
        let path = sjis(t.take(name_size)?)?;
        let offset = u64::from(t.le32()?) + data_at;
        let size = u64::from(t.le32()?);
        t.take(4)?;
        let compressed = t.le32()? > 0;
        let stored = u64::from(t.le32()?);
        rows.push(if compressed {
            Row::packed(path, offset, stored, size, Plan::Zlib)
        } else {
            Row::raw(path, offset, size)
        });
    }
    Ok(rows)
}

/// FVP (RFVP's original engine) BIN: name offsets into a string pool that
/// follows the 12-byte records; the pool ends at the first file's data.
fn fvp_bin(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let count = u64::from(le32(&bytes(s, 0, 4, 4)?, 0)?);
    limits(cx, count as usize)?;
    let names_start = count * 12 + 8;
    let table = index(s, cx, 8, count * 12)?;
    let records: Vec<_> = table
        .chunks_exact(12)
        .map(|r| {
            (
                le32(r, 0).unwrap() as usize,
                u64::from(le32(r, 4).unwrap()),
                u64::from(le32(r, 8).unwrap()),
            )
        })
        .collect();
    let pool_end = records
        .iter()
        .map(|r| r.1)
        .filter(|o| *o >= names_start)
        .min()
        .unwrap_or(length(s)?);
    let pool = index(s, cx, names_start, pool_end - names_start)?;
    records
        .into_iter()
        .map(|(name, offset, size)| {
            let raw = pool
                .get(name..)
                .context("FVP name offset outside string pool")?;
            ensure!(raw.contains(&0), "unterminated FVP name");
            Ok(Row::raw(sjis(zstr(raw))?, offset, size))
        })
        .collect()
}
