// SPDX-License-Identifier: GPL-3.0-or-later
// Translated from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
use std::collections::BTreeMap;
pub(super) fn tac(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let header = bytes(s, 0, 36, 36)?;
    let v110 = header.starts_with(b"TArc1.10\0\0\0\0");
    ensure!(
        v110 || header.starts_with(b"TArc1.00\0\0\0\0"),
        "TAC version"
    );
    let count = le32(&header, 20)? as usize;
    limits(cx, count)?;
    let dirs = le32(&header, 24)? as usize;
    limits(cx, dirs)?;
    let size = le32(&header, 28)? as u64;
    let base = if v110 { 44 } else { 36 };
    let data = bytes(s, base, size, cx.limits.max_index_size)?;
    let data = crysirror_resource::blowfish_le_decrypt(&data, b"TLibArchiveData", data.len())?;
    let table = crate::files::inflate(&data, cx.limits.max_index_size as usize)?;
    let mut c = Cursor::new(&table);
    let mut directory = Vec::new();
    for _ in 0..dirs {
        directory.push((c.le16()?, c.le16()? as usize, c.le32()? as usize));
    }
    let mut records = Vec::new();
    for _ in 0..count {
        records.push((
            c.le64()?,
            c.le32()? != 0,
            c.le32()? as u64,
            c.le32()? as u64,
            c.le32()? as u64,
        ));
    }
    for (hash, n, start) in directory {
        ensure!(
            start.checked_add(n).is_some_and(|n| n <= count),
            "TAC directory entry range"
        );
        for r in &mut records[start..start + n] {
            r.0 = (r.0 << 16) | hash as u64;
        }
    }
    let mut out = Vec::new();
    for (i, (hash, compressed, n, offset, packed)) in records.into_iter().enumerate() {
        let limit = cx.limits.max_entry_size as usize;
        let plan = decode(move |d, _| {
            if compressed {
                crate::files::inflate(d, limit)
            } else {
                let k = format!("{hash}_tlib_secure_");
                let mut prefix = d.len().min(10240);
                if d.len() >= 8 {
                    let h = crysirror_resource::blowfish_le_decrypt(&d[..8], k.as_bytes(), 8)?;
                    if h.starts_with(b"RIFF") || h.starts_with(b"TArc") {
                        prefix = d.len();
                    }
                }
                crysirror_resource::blowfish_le_decrypt(d, k.as_bytes(), prefix)
            }
        });
        out.push(
            Row::packed(
                format!("{i:05}.dat"),
                base + size + offset,
                packed,
                if compressed { n } else { packed },
                plan,
            )
            .meta(serde_json::json!({"hash":hash,"compressed":compressed})),
        );
    }
    Ok(out)
}
fn crc64(d: &[u8]) -> u64 {
    let mut v = !0u64;
    for b in d {
        v ^= (*b as u64) << 56;
        for _ in 0..8 {
            v = if v & 0x8000000000000000 != 0 {
                (v << 1) ^ 0x42f0e1eba9ea3693
            } else {
                v << 1
            };
        }
    }
    !v
}
pub(super) fn whale(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = le32(&bytes(s, 0, 4, 4)?, 0)? ^ 0x26aca46e;
    limits(cx, n as usize)?;
    let table = bytes(s, 4, n as u64 * 21, cx.limits.max_index_size)?;
    let mut names = BTreeMap::new();
    if let Some(list) = options(cx, "whale/dat")
        .and_then(|v| v.get("file_names"))
        .and_then(|v| v.as_array())
    {
        for value in list {
            let name = value.as_str().context("Whale filename string")?;
            let (b, _, err) = encoding_rs::SHIFT_JIS.encode(name);
            ensure!(!err && !b.is_empty(), "Whale filename encoding");
            names.insert(crc64(&b), b.into_owned());
        }
    }
    let mut c = Cursor::new(&table);
    let mut rows = Vec::new();
    for i in 0..n {
        let hash = c.le64()?;
        let kind = c.u8()? ^ hash as u8;
        ensure!(kind <= 2, "Whale entry type");
        let (mut offset, mut packed, mut orig) = (
            c.le32()? ^ hash as u32,
            c.le32()? ^ hash as u32,
            c.le32()? ^ hash as u32,
        );
        let name = names.get(&hash).cloned();
        if kind != 2 {
            let raw = name
                .as_ref()
                .with_context(|| format!("Whale filename profile missing for hash {hash:016x}"))?;
            offset ^= raw[raw.len() >> 1] as u32;
            packed ^= raw[raw.len() >> 2] as u32;
            orig ^= raw[raw.len() >> 3] as u32;
        }
        let path = if let Some(n) = &name {
            sjis(n)?
        } else {
            format!("{i:04}.txt")
        };
        let k = if kind == 2 {
            Some(caller_key(cx, "whale/dat", "whale/dat.game_key")?)
        } else {
            None
        };
        let limit = cx.limits.max_entry_size as usize;
        rows.push(
            Row::packed(
                path,
                offset as u64,
                packed as u64,
                if kind == 2 {
                    orig as u64
                } else {
                    packed as u64
                },
                decode(move |d, _| {
                    let mut o = d.to_vec();
                    if let Some(k) = &k {
                        let xor = k.with_bytes(|k| (hash ^ crc64(k)) as u32);
                        for word in o.chunks_exact_mut(4) {
                            let v = u32::from_le_bytes(word.try_into().unwrap()) ^ xor;
                            word.copy_from_slice(&v.to_le_bytes());
                        }
                        o = crate::files::inflate(&o, limit)?;
                    } else if kind == 1 {
                        let n = name.as_ref().context("Whale obfuscated filename")?;
                        let block = (o.len() as f32 / n.len() as f32).floor() as usize;
                        for (j, b) in o
                            .iter_mut()
                            .take(block * n.len().saturating_sub(1))
                            .enumerate()
                        {
                            *b ^= n[j / block];
                        }
                    }
                    Ok(o)
                }),
            )
            .meta(serde_json::json!({"hash":hash,"native_type":kind})),
        );
    }
    Ok(rows)
}
pub(super) fn tactics(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"TACTICS_ARC_FILE")?, "TACTICS signature");
    let option = options(cx, "tactics/arc");
    let version = option
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_u64());
    let (data, mut records, embedded) = if version == Some(1) {
        tactics_v1(s, cx)?
    } else {
        match tactics_v0(s, cx) {
            Ok(v) => v,
            Err(e) if version == Some(0) => return Err(e),
            Err(_) => tactics_v1(s, cx)?,
        }
    };
    let variable = option
        .and_then(|v| v.get("compression"))
        .and_then(|v| v.as_str())
        == Some("variable-lzss");
    let caller = if option.and_then(|v| v.get("key_id")).is_some() {
        Some(caller_key(cx, "tactics/arc", "tactics/arc.key")?)
    } else {
        None
    };
    let mut out = Vec::new();
    for (path, offset, packed, orig) in records.drain(..) {
        let key = if let Some(k) = &caller {
            k.with_bytes(|k| k.to_vec())
        } else {
            embedded.clone()
        };
        let n = if orig == 0 { packed } else { orig };
        out.push(Row::packed(
            path,
            data + offset,
            packed,
            n,
            decode(move |d, _| {
                let mut o = d.to_vec();
                if !key.is_empty() {
                    for (i, b) in o.iter_mut().enumerate() {
                        *b ^= key[i % key.len()];
                    }
                }
                if orig > 0 {
                    if variable {
                        variable_lzss(&o, orig as usize)
                    } else {
                        crysirror_resource::lzss_decompress(&o, orig as usize, &Default::default())
                    }
                } else {
                    Ok(o)
                }
            }),
        ));
    }
    Ok(out)
}
type TacticsRecords = Vec<(String, u64, u64, u64)>;
fn tactics_v0(s: &dyn AssetReader, cx: &DecodeContext) -> Result<(u64, TacticsRecords, Vec<u8>)> {
    let h = bytes(s, 16, 16, 16)?;
    let packed = le32(&h, 0)? as u64;
    let orig = le32(&h, 4)? as usize;
    let n = le32(&h, 8)? as usize;
    limits(cx, n)?;
    ensure!(
        packed <= 10 * 1024 * 1024 && orig as u64 <= cx.limits.max_index_size,
        "TACTICS table limit"
    );
    let input = bytes(s, 32, packed, cx.limits.max_index_size)?;
    let input: Vec<_> = input.iter().map(|b| b ^ 255).collect();
    let table = crysirror_resource::lzss_decompress(&input, orig, &Default::default())?;
    let mut c = Cursor::new(&table);
    let key = c.zero()?.to_vec();
    let mut out = Vec::new();
    for _ in 0..n {
        let off = c.le32()? as u64;
        let packed = c.le32()? as u64;
        let orig = c.le32()? as u64;
        let name = c.le32()? as usize;
        c.take(8)?;
        let name = sjis(c.take(name)?)?;
        range(length(s)?, 32 + input.len() as u64 + off, packed)?;
        out.push((name, off, packed, orig));
    }
    Ok((32 + input.len() as u64, out, key))
}
fn tactics_v1(s: &dyn AssetReader, cx: &DecodeContext) -> Result<(u64, TacticsRecords, Vec<u8>)> {
    let mut at = 16;
    let mut rows = Vec::new();
    let len = length(s)?;
    while at < len {
        limits(cx, rows.len() + 1)?;
        let h = bytes(s, at, 4, 4)?;
        let packed = le32(&h, 0)? as u64;
        if packed == 0 {
            break;
        }
        let h = bytes(s, at, 20, 20)?;
        let orig = le32(&h, 4)? as u64;
        let n = le32(&h, 8)? as u64;
        let name = sjis(&bytes(s, at + 20, n, cx.limits.max_index_size)?)?;
        at += 20 + n;
        range(len, at, packed)?;
        rows.push((name, at, packed, orig));
        at += packed;
    }
    Ok((0, rows, Vec::new()))
}
fn variable_lzss(d: &[u8], n: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(d);
    let mut declared = 0u64;
    for shift in (0..=28).step_by(7) {
        let b = c.u8()?;
        declared |= ((b & 127) as u64) << shift;
        if b & 128 == 0 {
            break;
        }
    }
    ensure!(declared == n as u64, "TACTICS variable decoded size");
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let b = c.u8()?;
        let mode = b & 3;
        let group = (b >> 2) as usize;
        if mode == 0 {
            let mut size = group + 1;
            if size >= 61 {
                let bytes = size - 60;
                ensure!(bytes <= 4, "TACTICS literal width");
                size = 0;
                for i in 0..bytes {
                    size |= (c.u8()? as usize) << (i * 8);
                }
                size += 1;
            }
            ensure!(size <= n - o.len(), "TACTICS literal range");
            o.extend_from_slice(c.take(size)?);
        } else {
            let (bytes, high, size) = match mode {
                1 => (1, (group >> 3) << 8, (group & 7) + 4),
                2 => (2, 0, group + 1),
                3 => (4, 0, group + 1),
                _ => unreachable!(),
            };
            let mut dist = high;
            for i in 0..bytes {
                dist |= (c.u8()? as usize) << (i * 8);
            }
            ensure!(size <= n - o.len(), "TACTICS back-reference exceeds output");
            crate::files::compression::copy(&mut o, dist, size, n)?;
        }
    }
    Ok(o)
}
