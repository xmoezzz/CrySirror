// SPDX-License-Identifier: GPL-3.0-or-later
// Structured archive formats ported from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2).context("short u16")?.try_into()?,
    ))
}
pub(super) fn exe(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"MZ")?, "missing DOS MZ");
    let nt = le32(&bytes(s, 60, 4, 4)?, 0)? as u64;
    let h = bytes(s, nt, 24, 24)?;
    ensure!(&h[..4] == b"PE\0\0", "unsupported non-PE executable");
    let n = u16_at(&h, 6)? as usize;
    limits(cx, n)?;
    let optional = u16_at(&h, 20)? as u64;
    let opt = index(s, cx, nt + 24, optional)?;
    let magic = u16_at(&opt, 0)?;
    let dirs = match magic {
        0x10b => 96,
        0x20b => 112,
        _ => bail!("unknown PE optional header {magic:x}"),
    };
    let dir_count = le32(&opt, dirs - 4)?;
    ensure!(dir_count >= 3, "PE resource directory missing");
    let res_rva = le32(&opt, dirs + 16)? as u64;
    let res_size = le32(&opt, dirs + 20)? as u64;
    let file_align = le32(&opt, 36)? as u64;
    let section_align = le32(&opt, 32)? as u64;
    let table = index(s, cx, nt + 24 + optional, n as u64 * 40)?;
    let mut sections = Vec::new();
    let mut extra = 0u64;
    for p in table.chunks_exact(40) {
        let vsize = le32(p, 8)? as u64;
        let va = le32(p, 12)? as u64;
        let raw_size = le32(p, 16)? as u64;
        let raw = le32(p, 20)? as u64;
        range(length(s)?, raw, raw_size)?;
        extra = extra.max(raw + raw_size);
        sections.push((va, vsize, raw, raw_size));
    }
    let rva = |r: u64| -> Result<u64> {
        let &(va, _, raw, _) = sections
            .iter()
            .find(|(va, size, _, _)| r >= *va && r < va + size)
            .context("PE RVA section not found")?;
        let raw = if file_align < 0x200 {
            raw
        } else {
            raw / 0x200 * 0x200
        };
        let align = if section_align < 0x1000 {
            file_align
        } else {
            section_align
        };
        let va = if align != 0 { va / align * align } else { va };
        r.checked_add(raw)
            .and_then(|v| v.checked_sub(va))
            .context("PE RVA overflow")
    };
    let mut rows = Vec::new();
    if res_rva != 0 && res_size != 0 {
        let base = rva(res_rva)?;
        let t = index(s, cx, base, res_size)?;
        let mut stack = vec![(0usize, String::new(), 0usize)];
        let mut visited = BTreeSet::new();
        while let Some((at, root, depth)) = stack.pop() {
            ensure!(
                depth < 256 && visited.insert(at),
                "PE cyclic/deep resources"
            );
            let n = u16_at(&t, at + 12)? as usize + u16_at(&t, at + 14)? as usize;
            limits(cx, n)?;
            for i in 0..n {
                let p = at + 16 + i * 8;
                let name = le32(&t, p)?;
                let target = le32(&t, p + 4)?;
                let label = if name & 0x80000000 != 0 {
                    let pos = (name & 0x7fffffff) as usize;
                    let n = u16_at(&t, pos)? as usize;
                    utf16(
                        t.get(pos + 2..pos + 2 + n * 2)
                            .context("PE name outside resource section")?,
                    )?
                } else {
                    pe_label(name)
                };
                let path = if root.is_empty() {
                    label
                } else {
                    format!("{root}／{label}")
                };
                let at = (target & 0x7fffffff) as usize;
                if target & 0x80000000 != 0 {
                    stack.push((at, path, depth + 1));
                } else {
                    limits(cx, rows.len() + 1)?;
                    let off = rva(le32(&t, at)? as u64)?;
                    let size = le32(&t, at + 4)? as u64;
                    rows.push(
                        Row::raw(path, off, size)
                            .meta(serde_json::json!({"code_page":le32(&t,at+8)?})),
                    );
                }
            }
        }
    }
    if n > 0 && extra < length(s)? {
        rows.push(Row::raw("extra_data".into(), extra, length(s)? - extra));
    }
    checked_rows(s, cx, rows)
}
fn pe_label(id: u32) -> String {
    match id {
        1 => "CURSOR",
        2 => "BITMAP",
        3 => "ICON",
        4 => "MENU",
        5 => "DIALOG",
        6 => "STRING",
        7 => "FONT_DIRECTORY",
        8 => "FONT",
        9 => "ACCELERATOR",
        10 => "RC_DATA",
        11 => "MESSAGE_TABLE",
        16 => "VERSION",
        17 => "DLG_INCLUDE",
        19 => "PLUG_AND_PLAY",
        20 => "VXD",
        21 => "ANIMATED_CURSOR",
        22 => "ANIMATED_ICON",
        23 => "HTML",
        24 => "MANIFEST",
        _ => return id.to_string(),
    }
    .into()
}
pub(super) fn noa(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(
        magic(s, 0, b"Entis\x1a\0\0\0\x04\0\x02\0\0\0\0ERISA-Archive file")?,
        "missing NOA"
    );
    let mut rows = Vec::new();
    let mut work = vec![(0x40u64, length(s)?, String::new(), 0usize)];
    let mut seen = BTreeSet::new();
    while let Some((mut at, end, root, depth)) = work.pop() {
        ensure!(depth < 128 && seen.insert(at), "NOA cyclic/deep directory");
        while at < end {
            let h = bytes(s, at, 16, 16)?;
            let section_size = u64::from_le_bytes(h[8..16].try_into()?);
            let section_end = at
                .checked_add(16)
                .and_then(|v| v.checked_add(section_size))
                .context("NOA section overflow")?;
            ensure!(section_end <= end, "NOA section outside archive");
            if &h[..8] == b"DirEntry" {
                let t = index(s, cx, at + 16, section_size)?;
                let mut c = Cursor::new(&t);
                let n = count(&mut c, cx)?;
                for _ in 0..n {
                    let size = c.le64()?;
                    let flags = c.le32()?;
                    let encryption = c.le32()?;
                    let offset = at + c.le64()?;
                    c.take(8)?;
                    let extra = c.le32()? as usize;
                    if flags & 0x70 != 0 {
                        c.take(extra)?;
                    }
                    let name_size = c.le32()? as usize;
                    let name = sjis(zstr(c.take(name_size)?))?;
                    let path = if root.is_empty() {
                        name
                    } else {
                        format!("{root}/{name}")
                    };
                    if flags == 0x10 {
                        let h = bytes(s, offset, 16, 16)?;
                        let sub = u64::from_le_bytes(h[8..16].try_into()?);
                        work.push((offset, offset + 16 + sub, path, depth + 1));
                    } else if flags != 0x20 && flags != 0x40 {
                        limits(cx, rows.len() + 1)?;
                        let h = bytes(s, offset, 16, 16)?;
                        ensure!(&h[..8] == b"filedata", "NOA missing filedata");
                        let stored = u64::from_le_bytes(h[8..16].try_into()?);
                        ensure!(
                            encryption & !(0x80000010 | 0x40000000) == 0,
                            "unknown NOA encryption flags {encryption:08x}"
                        );
                        let key = if encryption & 0x40000000 != 0 {
                            let alias = cx
                                .parameters
                                .get("entis/noa")
                                .and_then(|p| p.get("key_id"))
                                .and_then(|v| v.as_str())
                                .context("NOA BSHF key_id profile missing")?;
                            Some(cx.keys.key(cx.client, alias)?)
                        } else {
                            None
                        };
                        let plan = decode(move |b, _| {
                            let checksum = if encryption != 0 {
                                Some(
                                    b.get(b.len().checked_sub(4).context("NOA checksum length")?..)
                                        .context("NOA checksum")?,
                                )
                            } else {
                                None
                            };
                            let mut data = b.to_vec();
                            if let Some(key) = &key {
                                let decoded = key.with_bytes(|key| {
                                    super::entis::bshf(&data, size as usize, key)
                                })?;
                                ensure!(data.len() >= decoded.len(), "NOA BSHF output length");
                                data[..decoded.len()].copy_from_slice(&decoded);
                            }
                            if encryption & 0x80000010 != 0 {
                                data = super::entis::erisan(&data, size as usize)?;
                            }
                            let output = data
                                .get(..size as usize)
                                .context("NOA decoded size")?
                                .to_vec();
                            if let Some(expected) = checksum {
                                let mut got = [0u8; 4];
                                for (i, v) in output.iter().enumerate() {
                                    got[i % 4] ^= v;
                                }
                                ensure!(got == expected, "NOA checksum mismatch");
                            }
                            Ok(output)
                        });
                        rows.push(
                            Row::packed(path, offset + 16, stored, size, plan)
                                .meta(serde_json::json!({"encryption":encryption})),
                        );
                    }
                }
            }
            at = section_end;
        }
    }
    checked_rows(s, cx, rows)
}
#[derive(Clone)]
enum Cell {
    Number(u64),
    String(String),
    Data(Vec<u8>),
}
type UtfRow = BTreeMap<String, Cell>;
fn number(r: &UtfRow, k: &str) -> Result<u64> {
    match r.get(k) {
        Some(Cell::Number(v)) => Ok(*v),
        _ => bail!("CPK missing numeric {k}"),
    }
}
fn optional_number(r: &UtfRow, k: &str) -> u64 {
    match r.get(k) {
        Some(Cell::Number(v)) => *v,
        _ => 0,
    }
}
fn text(r: &UtfRow, k: &str) -> String {
    match r.get(k) {
        Some(Cell::String(v)) => v.clone(),
        _ => String::new(),
    }
}
fn blob<'a>(r: &'a UtfRow, k: &str) -> Result<&'a [u8]> {
    match r.get(k) {
        Some(Cell::Data(v)) => Ok(v),
        _ => bail!("CPK missing data {k}"),
    }
}
fn be16(c: &mut Cursor<'_>) -> Result<u16> {
    Ok(u16::from_be_bytes(c.take(2)?.try_into()?))
}
fn be64(c: &mut Cursor<'_>) -> Result<u64> {
    Ok(u64::from_be_bytes(c.take(8)?.try_into()?))
}
fn utf_string(b: &[u8], at: usize) -> Result<String> {
    let mut c = Cursor::new(b.get(at..).context("CPK string offset")?);
    Ok(String::from_utf8(c.zero()?.to_vec())?)
}
fn utf_cell(c: &mut Cursor<'_>, kind: u8, b: &[u8], strings: usize, data: usize) -> Result<Cell> {
    Ok(match kind {
        0 | 1 => Cell::Number(c.u8()? as u64),
        2 | 3 => Cell::Number(be16(c)? as u64),
        4 | 5 => Cell::Number(c.be32()? as u64),
        6 | 7 => Cell::Number(be64(c)?),
        8 => Cell::Number(c.be32()? as u64),
        10 => Cell::String(utf_string(b, strings + c.be32()? as usize)?),
        11 => {
            let at = data + c.be32()? as usize;
            let size = c.be32()? as usize;
            Cell::Data(b.get(at..at + size).context("CPK data offset")?.to_vec())
        }
        _ => bail!("unsupported UTF type {kind}"),
    })
}
fn utf(b: &[u8], cx: &DecodeContext) -> Result<Vec<UtfRow>> {
    let mut c = Cursor::new(b);
    ensure!(c.take(4)? == b"@UTF", "missing @UTF");
    let declared = c.be32()? as usize;
    ensure!(
        declared.checked_add(8).is_some_and(|n| n <= b.len()),
        "UTF table length"
    );
    let rows_at = c.be32()? as usize + 8;
    let strings = c.be32()? as usize + 8;
    let data = c.be32()? as usize + 8;
    c.take(4)?;
    let cols = be16(&mut c)? as usize;
    let stride = be16(&mut c)? as usize;
    let n = c.be32()? as usize;
    limits(cx, n)?;
    limits(cx, cols)?;
    let mut columns = Vec::new();
    for _ in 0..cols {
        let mut flags = c.u8()?;
        if flags == 0 {
            let v = c.be32()?;
            ensure!(v <= 255, "UTF extended flags");
            flags = v as u8;
        }
        let name = utf_string(b, strings + c.be32()? as usize)?;
        let constant = if flags & 0xf0 == 0x30 {
            Some(utf_cell(&mut c, flags & 15, b, strings, data)?)
        } else {
            None
        };
        ensure!(
            matches!(flags & 0xf0, 0 | 0x10 | 0x30 | 0x50),
            "unsupported UTF storage"
        );
        columns.push((name, flags, constant));
    }
    ensure!(
        rows_at
            .checked_add(n.checked_mul(stride).context("UTF row overflow")?)
            .is_some_and(|v| v <= b.len()),
        "UTF row extent"
    );
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut c = Cursor::new(&b[rows_at + i * stride..rows_at + (i + 1) * stride]);
        let mut row = BTreeMap::new();
        for (name, flags, value) in &columns {
            let value = match flags & 0xf0 {
                0 | 0x10 => Cell::Number(0),
                0x30 => value.clone().unwrap(),
                0x50 => utf_cell(&mut c, flags & 15, b, strings, data)?,
                _ => unreachable!(),
            };
            row.insert(name.clone(), value);
        }
        out.push(row);
    }
    Ok(out)
}
fn packet(
    s: &dyn AssetReader,
    cx: &DecodeContext,
    at: u64,
    magic_bytes: &[u8],
) -> Result<Vec<UtfRow>> {
    let h = bytes(s, at, 16, 16)?;
    ensure!(&h[..4] == magic_bytes, "CPK packet signature mismatch");
    let n = u64::from_le_bytes(h[8..16].try_into()?);
    let mut b = index(s, cx, at + 16, n)?;
    if !b.starts_with(b"@UTF") {
        let mut key = 0x655fu32;
        for b in &mut b {
            *b ^= key as u8;
            key = key.wrapping_mul(0x4115);
        }
    }
    utf(&b, cx)
}
pub(super) fn cpk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let header = packet(s, cx, 0, b"CPK ")?;
    let h = header.first().context("empty CPK header")?;
    let content = number(h, "ContentOffset")?;
    let align = number(h, "Align")?;
    ensure!(align > 0, "CPK zero alignment");
    let toc_at = optional_number(h, "TocOffset");
    let mut entries = BTreeMap::<u64, (String, u64, u64)>::new();
    if toc_at != 0 {
        for r in packet(s, cx, toc_at, b"TOC ")? {
            let name = text(&r, "FileName");
            let dir = text(&r, "DirName");
            let path = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            entries.insert(
                number(&r, "ID")?,
                (
                    path,
                    toc_at.min(content) + number(&r, "FileOffset")?,
                    number(&r, "FileSize")?,
                ),
            );
        }
    }
    let itoc = optional_number(h, "ItocOffset");
    if itoc != 0 {
        let p = packet(s, cx, itoc, b"ITOC")?;
        if let Some(r) = p.first() {
            if r.contains_key("DataL") {
                for name in ["DataL", "DataH"] {
                    for r in utf(blob(r, name)?, cx)? {
                        let id = number(&r, "ID")?;
                        let size = number(&r, "FileSize")?;
                        entries.entry(id).or_insert((String::new(), 0, 0)).2 = size;
                    }
                }
                let mut at = content;
                for value in entries.values_mut() {
                    value.1 = at;
                    at = at
                        .checked_add(value.2)
                        .and_then(|v| v.checked_add((align - value.2 % align) % align))
                        .context("CPK aligned offset overflow")?;
                }
            }
        }
    }
    let mut rows = Vec::new();
    for (id, (path, off, stored)) in entries {
        limits(cx, rows.len() + 1)?;
        range(length(s)?, off, stored)?;
        if stored >= 16 && magic(s, off, b"CRILAYLA")? {
            let h = bytes(s, off + 8, 8, 8)?;
            let decoded = le32(&h, 0)? as u64;
            let comp = le32(&h, 4)? as u64;
            ensure!(stored >= 16 + comp, "CRILAYLA stored size");
            let size = decoded + (stored - 16 - comp);
            ensure!(size <= cx.limits.max_entry_size, "CRILAYLA output limit");
            rows.push(
                Row::packed(
                    path,
                    off,
                    stored,
                    size,
                    decode(move |b, _| layla(b, decoded as usize, comp as usize)),
                )
                .meta(serde_json::json!({"cpk_id":id})),
            );
        } else {
            rows.push(Row::raw(path, off, stored).meta(serde_json::json!({"cpk_id":id})));
        }
    }
    name_unnamed(&mut rows);
    checked_rows(s, cx, rows)
}
fn layla(b: &[u8], size: usize, comp: usize) -> Result<Vec<u8>> {
    let mut data = b
        .get(16..16 + comp)
        .context("CRILAYLA compressed range")?
        .to_vec();
    data.reverse();
    let mut at = 0usize;
    let mut bits = |n: usize| -> Result<usize> {
        let mut v = 0;
        for _ in 0..n {
            let b = *data.get(at / 8).context("truncated CRILAYLA bits")?;
            v = (v << 1) | ((b >> (7 - at % 8)) & 1) as usize;
            at += 1;
        }
        Ok(v)
    };
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        if bits(1)? != 0 {
            let distance = bits(13)? + 3;
            ensure!(distance <= out.len(), "CRILAYLA lookbehind outside output");
            let mut count = 3usize;
            for width in [2, 3, 5].into_iter().chain(std::iter::repeat(8)) {
                let v = bits(width)?;
                count = count.checked_add(v).context("CRILAYLA run overflow")?;
                ensure!(count <= size - out.len(), "CRILAYLA run outside output");
                if v != (1 << width) - 1 {
                    break;
                }
            }
            for _ in 0..count {
                out.push(out[out.len() - distance]);
            }
        } else {
            out.push(bits(8)? as u8);
        }
    }
    out.reverse();
    let mut result = b[16 + comp..].to_vec();
    result.extend(out);
    Ok(result)
}
