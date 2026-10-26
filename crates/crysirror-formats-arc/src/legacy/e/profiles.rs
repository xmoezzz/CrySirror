// SPDX-License-Identifier: GPL-3.0-or-later
// Archive parsing and caller-owned keys/dependencies, ported from arc_unpacker,
// Copyright (C) 2016 rr-. No title key/filename tables are distributed here.
use super::*;
use std::collections::BTreeSet;
fn options<'a>(cx: &'a DecodeContext, id: &str) -> Result<&'a serde_json::Value> {
    cx.parameters
        .get(id)
        .with_context(|| format!("{id} requires an explicit caller profile"))
}
fn key(cx: &DecodeContext, id: &str) -> Result<Vec<u8>> {
    let alias = options(cx, id)?
        .get("key_id")
        .and_then(|v| v.as_str())
        .context("profile key_id missing")?;
    Ok(cx.keys.key(cx.client, alias)?.with_bytes(|b| b.to_vec()))
}
fn sibling_name(cx: &DecodeContext, ext: &str) -> Result<String> {
    let path = cx
        .source_name
        .as_deref()
        .context("archive requires source_name for companion lookup")?;
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    Ok(format!("{stem}.{ext}"))
}
pub(super) fn dsk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let name = options(cx, "abogado/dsk")
        .ok()
        .and_then(|p| p.get("table"))
        .and_then(|p| p.as_str())
        .map(str::to_owned)
        .unwrap_or(sibling_name(cx, "PFT")?);
    let dep = cx.open_sibling(&name)?;
    let h = bytes(dep.as_ref(), 0, 16, 16)?;
    ensure!(le32(&h, 0)? == 0x08000010, "unsupported PFT version");
    let n = le32(&h, 4)? as usize;
    limits(cx, n)?;
    let table = index(dep.as_ref(), cx, 16, n as u64 * 16)?;
    let mut rows = Vec::new();
    for p in table.chunks_exact(16) {
        rows.push(Row::raw(
            sjis(zstr(&p[..8]))?,
            u64::from(le32(p, 8)?) * 2048,
            le32(p, 12)? as u64,
        ));
    }
    checked_rows(s, cx, rows)
}
pub(super) fn eagls(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let name = options(cx, "eagls/pak")
        .ok()
        .and_then(|p| p.get("table"))
        .and_then(|p| p.as_str())
        .map(str::to_owned)
        .unwrap_or(sibling_name(cx, "idx")?);
    let dep = cx.open_sibling(&name)?;
    let len = length(dep.as_ref())?;
    ensure!(len >= 4, "short EAGLS index");
    let mut data = index(dep.as_ref(), cx, 0, len - 4)?;
    let mut seed = le32(&bytes(dep.as_ref(), len - 4, 4, 4)?, 0)?;
    let key = key(cx, "eagls/pak")?;
    ensure!(!key.is_empty(), "empty EAGLS index key");
    for v in &mut data {
        seed = seed.wrapping_mul(214013).wrapping_add(2531011);
        *v ^= key[((seed >> 16) & 0x7fff) as usize % key.len()];
    }
    let mut c = Cursor::new(&data);
    let mut rows = Vec::new();
    let mut min = u64::MAX;
    loop {
        let path = sjis(zstr(c.take(20)?))?;
        if path.is_empty() {
            break;
        }
        limits(cx, rows.len() + 1)?;
        let off = c.le32()? as u64;
        let size = c.le32()? as u64;
        min = min.min(off);
        rows.push(Row::raw(path, off, size));
    }
    for r in &mut rows {
        r.offset -= min;
    }
    checked_rows(s, cx, rows)
}
fn xor_range(
    s: &dyn AssetReader,
    cx: &DecodeContext,
    at: u64,
    n: u64,
    key: &[u8],
) -> Result<Vec<u8>> {
    ensure!(!key.is_empty(), "empty archive key");
    let mut b = index(s, cx, at, n)?;
    for (i, v) in b.iter_mut().enumerate() {
        *v ^= key[(at as usize + i) % key.len()];
    }
    Ok(b)
}
pub(super) fn dx(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let k = key(cx, "dxlib/dx")?;
    let h = xor_range(s, cx, 0, 24, &k)?;
    ensure!(&h[..2] == b"DX", "DX key/header mismatch");
    let ver = u16::from_le_bytes([h[2], h[3]]);
    let size = le32(&h, 4)? as u64;
    let content = le32(&h, 8)? as u64;
    let table = xor_range(s, cx, le32(&h, 12)? as u64, size, &k)?;
    let files = le32(&h, 16)? as usize;
    let dirs = le32(&h, 20)? as usize;
    let mut rows = Vec::new();
    let mut stack = vec![(0usize, String::new(), 0usize)];
    let mut seen = BTreeSet::new();
    let name = |off: usize| -> Result<String> {
        let b = table.get(off..).context("DX name offset")?;
        let mut c = Cursor::new(b);
        let skip = c.le16()? as usize * 4 + 4;
        let mut c = Cursor::new(b.get(skip..).context("DX name skip")?);
        sjis(c.zero()?)
    };
    while let Some((dir, root, depth)) = stack.pop() {
        ensure!(
            depth < 256 && seen.insert(dir),
            "DX cyclic/deep directories"
        );
        let mut c = Cursor::new(table.get(dirs + dir..).context("DX dir offset")?);
        let dir_offset = c.le32()?;
        let parent = c.le32()?;
        let n = count(&mut c, cx)?;
        let at = c.le32()? as usize;
        let root = if dir_offset != u32::MAX && parent != u32::MAX {
            let label = name(le32(&table, files + dir_offset as usize)? as usize)?;
            if root.is_empty() {
                label
            } else {
                format!("{root}/{label}")
            }
        } else {
            root
        };
        let mut c = Cursor::new(table.get(files + at..).context("DX file table offset")?);
        for _ in 0..n {
            let name_at = c.le32()? as usize;
            let flags = c.le32()?;
            c.take(24)?;
            let off = c.le32()? as u64;
            if flags & 16 != 0 {
                stack.push((off as usize, root.clone(), depth + 1));
            } else {
                limits(cx, rows.len() + 1)?;
                let orig = c.le32()? as u64;
                let comp = if ver >= 2 { c.le32()? } else { u32::MAX };
                let stored = if comp == u32::MAX { orig } else { comp as u64 };
                let path = name(name_at)?;
                let path = if root.is_empty() {
                    path
                } else {
                    format!("{root}/{path}")
                };
                let key = k.clone();
                let off = content + off;
                rows.push(Row::packed(path,off,stored,stored,decode(move|b,_|Ok(b.iter().enumerate().map(|(i,v)|v^key[(off as usize+i)%key.len()]).collect()))).meta(serde_json::json!({"native_original_size":orig,"native_compressed":comp!=u32::MAX})));
            }
        }
    }
    checked_rows(s, cx, rows)
}
#[derive(Clone)]
struct GDecoder {
    src: [usize; 4],
    dst: [usize; 4],
    func: [usize; 2],
    mei: bool,
}
fn array4(value: &serde_json::Value, key: &str) -> Result<[usize; 4]> {
    let v = value
        .get(key)
        .and_then(|v| v.as_array())
        .context("GLib decoder permutation missing")?;
    ensure!(v.len() == 4, "GLib permutation length");
    let mut out = [0; 4];
    for i in 0..4 {
        out[i] = v[i].as_u64().context("GLib permutation integer")? as usize;
    }
    let set: BTreeSet<_> = out.iter().copied().collect();
    ensure!(
        set == BTreeSet::from([0, 1, 2, 3]),
        "GLib invalid permutation"
    );
    Ok(out)
}
fn gdecoder(p: &serde_json::Value, mei: bool) -> Result<GDecoder> {
    let f = p
        .get("funcs")
        .and_then(|v| v.as_array())
        .context("GLib funcs missing")?;
    ensure!(f.len() == 2, "GLib funcs length");
    let func = [
        f[0].as_u64().context("GLib func integer")? as usize,
        f[1].as_u64().context("GLib func integer")? as usize,
    ];
    ensure!(
        func.iter().all(|i| *i < if mei { 7 } else { 6 }),
        "GLib func index"
    );
    Ok(GDecoder {
        src: array4(p, "src")?,
        dst: array4(p, "dst")?,
        func,
        mei,
    })
}
fn gfunc(b: u8, acc: usize, n: usize, mei: bool) -> u8 {
    if mei {
        match n {
            0 => b.rotate_left(5),
            1 => b.rotate_right((acc & 7) as u32),
            2 => {
                if acc & 1 != 0 {
                    b.wrapping_add(acc as u8)
                } else {
                    b.wrapping_sub(acc as u8)
                }
            }
            3 => b ^ ((0x1100u16 >> (acc & 7)) as u8),
            4 => (b ^ (0x80 >> (acc & 7))).rotate_right((acc & 7) as u32),
            5 => b ^ ((b >> 1) & 0x55),
            6 => {
                let c = b ^ (acc as u8 & 1);
                (c << 1) ^ (((c << 1) ^ (c >> 1)) & 0x55)
            }
            _ => unreachable!(),
        }
    } else {
        match n {
            0 => b.rotate_right((acc & 7) as u32),
            1 => b ^ (acc as u8),
            2 => b ^ 255,
            3 => b.wrapping_sub(0x64) ^ 255,
            4 => b.wrapping_add(acc as u8),
            5 => b.rotate_left(4),
            _ => unreachable!(),
        }
    }
}
fn gdecode(b: &[u8], d: &GDecoder) -> Vec<u8> {
    let mut out = vec![0; b.len()];
    let whole = b.len() & !3;
    for i in 0..whole {
        let si = (i & !3) + d.src[i & 3];
        let di = (i & !3) + d.dst[i & 3];
        out[di] = gfunc(gfunc(b[si], i, d.func[0], d.mei), i, d.func[1], d.mei);
    }
    for i in whole..b.len() {
        out[i] = gfunc(gfunc(b[i], i, d.func[0], d.mei), i, d.func[1], d.mei);
    }
    out
}
fn gkey(p: &serde_json::Value, mei: bool, key: u32) -> Result<GDecoder> {
    let k = key.to_string();
    let hex = format!("{key:08x}");
    gdecoder(
        p.get("decoders")
            .and_then(|v| v.get(&k).or_else(|| v.get(&hex)))
            .with_context(|| format!("GLib caller mapping missing for native key {key:08x}"))?,
        mei,
    )
}
pub(super) fn glib2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let p = options(cx, "glib/glib2")?;
    let family = p
        .get("family")
        .and_then(|v| v.as_str())
        .context("GLib family missing")?;
    ensure!(
        matches!(family, "mei" | "musume"),
        "unknown GLib cipher family"
    );
    let mei = family == "mei";
    let hd = gdecoder(p.get("header").context("GLib header mapping missing")?, mei)?;
    let h = gdecode(&bytes(s, 0, 0x5c, 0x5c)?, &hd);
    ensure!(
        h.starts_with(b"GLibArchiveData2.1\0") || h.starts_with(b"GLibArchiveData2.0\0"),
        "GLib key/header mismatch"
    );
    let at = le32(&h, 84)? as u64;
    let size = le32(&h, 88)? as u64;
    let mut t = index(s, cx, at, size)?;
    for i in (0..4).rev() {
        let key = le32(&h, 20 + i * 16)?;
        t = gdecode(&t, &gkey(p, mei, key)?);
    }
    ensure!(t.starts_with(b"CDBD"), "GLib decoded index mismatch");
    let mut c = Cursor::new(&t[4..]);
    let n = count(&mut c, cx)?;
    let names = 16 + n * 24;
    let headers = c.le32()? as usize + 16;
    c.take(4)?;
    let mut paths = Vec::<String>::new();
    let mut rows = Vec::new();
    for i in 0..n {
        let mut c = Cursor::new(
            t.get(16 + i * 24..16 + (i + 1) * 24)
                .context("GLib table row range")?,
        );
        let name_at = names + c.le32()? as usize;
        c.take(4)?;
        let parent = c.le32()?;
        let flags = c.le32()?;
        let header = headers + c.le32()? as usize;
        let header_size = c.le32()? as usize;
        let mut nc = Cursor::new(t.get(name_at..).context("GLib name offset")?);
        let name = sjis(nc.zero()?)?;
        let path = if parent == u32::MAX {
            name
        } else {
            format!(
                "{}/{name}",
                paths
                    .get(parent as usize)
                    .context("GLib parent must precede child")?
            )
        };
        paths.push(path.clone());
        if flags == 0x100 {
            let h = t
                .get(header..header + header_size)
                .context("GLib file header extent")?;
            ensure!(
                le32(h, 0)? as usize + 4 == header_size && header_size >= 80,
                "GLib file header size"
            );
            let size = le32(h, 8)? as u64;
            let off = le32(h, 12)? as u64;
            let mut decoders = Vec::new();
            for i in 0..4.min(size.div_ceil(0x20000) as usize) {
                decoders.push(gkey(p, mei, le32(h, 16 + i * 16)?)?);
            }
            rows.push(Row::packed(
                path,
                off,
                size,
                size,
                decode(move |b, _| {
                    let mut out = Vec::with_capacity(b.len());
                    for (i, chunk) in b.chunks(0x20000).enumerate() {
                        out.extend(gdecode(chunk, &decoders[i % 4]));
                    }
                    Ok(out)
                }),
            ));
        }
    }
    checked_rows(s, cx, rows)
}
fn words_key(cx: &DecodeContext, id: &str) -> Result<[u32; 5]> {
    let b = key(cx, id)?;
    ensure!(b.len()>=20,"LPK key material is base1,base2,aux1,aux2,rotation as five LE u32 values followed by xor byte");
    let mut out = [0; 5];
    for (i, v) in out.iter_mut().enumerate() {
        *v = le32(&b, i * 4)?;
    }
    Ok(out)
}
pub(super) fn lpk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"LPK1")?, "missing LPK1");
    let k = words_key(cx, "lucifen/lpk")?;
    let material = key(cx, "lucifen/lpk")?;
    ensure!(material.len() == 21, "LPK key material length must be 21");
    let xor = material[20];
    let source = cx
        .source_name
        .as_deref()
        .context("LPK needs source_name to derive filename keys")?;
    let name = source.rsplit('/').next().unwrap_or(source);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let name = encoding_rs::SHIFT_JIS.encode(stem).0;
    let (mut key1, mut key2) = (k[0], k[1]);
    for i in 0..name.len() {
        key1 = (key1 ^ name[name.len() - 1 - i] as u32).rotate_right(7);
        key2 = (key2 ^ name[i] as u32).rotate_left(7);
    }
    key1 ^= k[2];
    key2 ^= k[3];
    let tmp = le32(&bytes(s, 4, 4, 4)?, 0)? ^ key2;
    let flags = tmp >> 24;
    ensure!(flags & !31 == 0, "unknown LPK flags");
    let mut size = (tmp & 0xffffff) as u64;
    if flags & 1 != 0 {
        size = (size << 11)
            .checked_sub(8)
            .context("LPK aligned table size")?;
    }
    let mut table = index(s, cx, 8, size)?;
    let mut key = key2;
    let mut rotate = k[4];
    for p in table.chunks_exact_mut(4) {
        let v = u32::from_le_bytes(p.try_into()?) ^ key;
        p.copy_from_slice(&v.to_le_bytes());
        rotate = rotate.rotate_left(4);
        key = key.rotate_right(rotate & 31);
    }
    let mut c = Cursor::new(&table);
    let n = count(&mut c, cx)?;
    let prefix_len = c.u8()? as usize;
    let prefix = c.take(prefix_len)?.to_vec();
    let offsets32 = c.u8()? != 0;
    let letters = c.le32()? as usize;
    let entries = c.at.checked_add(letters).context("LPK trie range")?;
    ensure!(entries <= table.len(), "LPK trie extent");
    let mut stack = vec![(c.at, Vec::<u8>::new(), 0usize)];
    let mut seen = BTreeSet::new();
    let mut mini = Vec::new();
    while let Some((at, name, depth)) = stack.pop() {
        ensure!(depth < 16384 && seen.insert(at), "LPK cyclic/deep trie");
        let mut c = Cursor::new(table.get(at..entries).context("LPK trie node range")?);
        let count = c.u8()? as usize;
        for _ in 0..count {
            let letter = c.u8()?;
            let next = if offsets32 {
                c.le32()?
            } else {
                c.le16()? as u32
            } as usize;
            if letter == 0 {
                mini.push((next, name.clone()));
                limits(cx, mini.len())?;
            } else {
                let mut name = name.clone();
                name.push(letter);
                stack.push((at + c.at + next, name, depth + 1));
            }
        }
    }
    ensure!(mini.len() == n, "LPK trie entry count mismatch");
    let mut rows = Vec::new();
    for (id, name) in mini {
        let stride = if flags & 8 != 0 { 13 } else { 9 };
        let at = entries + id * stride;
        let p = table
            .get(at..at + stride)
            .context("LPK entry outside table")?;
        let entry_flags = p[0];
        let off = (le32(p, 1)? as u64) << if flags & 1 != 0 { 11 } else { 0 };
        let stored = le32(p, 5)? as u64;
        let size = if flags & 8 != 0 {
            le32(p, 9)? as u64
        } else {
            stored
        };
        let pre = prefix.clone();
        let out_size = size + pre.len() as u64;
        ensure!(
            out_size <= cx.limits.max_entry_size,
            "LPK entry output limit"
        );
        let plan = decode(move |b, _| {
            let mut d = if flags & 8 != 0 {
                crysirror_resource::lzss_decompress(b, size as usize, &Default::default())?
            } else {
                b.to_vec()
            };
            if flags & 16 != 0 {
                for v in &mut d {
                    *v = (*v ^ xor).rotate_right(4);
                }
            }
            if flags & 4 != 0 {
                let mut key = key1;
                let mut rotate = k[4];
                let n = d.len().min(0x100) & !3;
                for p in d[..n].chunks_exact_mut(4) {
                    let v = u32::from_le_bytes(p.try_into()?) ^ key;
                    p.copy_from_slice(&v.to_le_bytes());
                    rotate = rotate.rotate_right(4);
                    key = key.rotate_left(rotate & 31);
                }
            }
            let mut out = pre.clone();
            out.extend(d);
            Ok(out)
        });
        rows.push(
            Row::packed(sjis(&name)?, off, stored, out_size, plan)
                .meta(serde_json::json!({"flags":flags,"entry_flags":entry_flags})),
        );
    }
    checked_rows(s, cx, rows)
}
fn number(c: &mut Cursor<'_>) -> Result<u64> {
    let mut n = 0u64;
    for _ in 0..8 {
        let b = c.u8()?;
        n = n.checked_mul(10).context("Cyberworks decimal overflow")?;
        if b != 255 {
            n = n
                .checked_add((b ^ 127) as u64)
                .context("Cyberworks decimal overflow")?;
        }
    }
    Ok(n)
}
pub(super) fn appendix(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = window(s, cx, 0)?;
    let mut c = Cursor::new(&h);
    let n = c.le32()? as usize;
    c.take(n.checked_mul(2).context("Appendix name length")?)?;
    let orig = number(&mut c)?;
    let comp = number(&mut c)?;
    let at = c.at as u64;
    let table = lzss_index(&index(s, cx, at, comp)?, orig, cx, false)?;
    let base = at + comp;
    cyber_rows(s, cx, &table, Some(base))
}
pub(super) fn cyber_dat(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 16, 16)?;
    let mut c = Cursor::new(&h);
    let orig = number(&mut c)?;
    let comp = number(&mut c)?;
    ensure!(
        comp == length(s)? - 16,
        "Cyberworks DAT compressed index extent"
    );
    let table = lzss_index(&index(s, cx, 16, comp)?, orig, cx, false)?;
    cyber_rows(s, cx, &table, None)
}
fn cyber_rows(
    s: &dyn AssetReader,
    cx: &DecodeContext,
    t: &[u8],
    base: Option<u64>,
) -> Result<Vec<Row>> {
    let id = if base.is_some() {
        "cyberworks/appendix"
    } else {
        "cyberworks/dat"
    };
    let mut c = Cursor::new(t);
    let mut rows = Vec::new();
    let deps = if base.is_none() {
        let values = options(cx, id)?
            .get("data_files")
            .and_then(|v| v.as_array())
            .context("Cyberworks caller profile data_files missing")?;
        let mut out = Vec::new();
        for v in values {
            out.push(cx.open_sibling(v.as_str().context("Cyberworks data filename")?)?);
        }
        out
    } else {
        Vec::new()
    };
    while c.left() > 0 {
        limits(cx, rows.len() + 1)?;
        ensure!(
            c.le32()? == if base.is_some() { 24 } else { 23 },
            "Cyberworks entry size"
        );
        let file_id = c.le32()?;
        let orig = c.le32()? as u64;
        let comp = c.le32()? as u64;
        let off = c.le32()? as u64;
        let kind = c.take(2)?.to_vec();
        c.take(4)?;
        let dep = if base.is_some() {
            c.take(2)?;
            None
        } else {
            let dep = c.u8()? as usize;
            Some(deps.get(dep).context("Cyberworks data file id")?.clone())
        };
        let at = base.unwrap_or(0) + off;
        let reader = dep.as_deref().unwrap_or(s);
        range(length(reader)?, at, comp)?;
        ensure!(orig <= cx.limits.max_entry_size, "Cyberworks output limit");
        let source = dep.clone();
        let image = matches!(kind.as_slice(), b"b0" | b"n0" | b"o0");
        let native = if image {
            let raw = bytes(reader, at, comp, cx.limits.max_entry_size)?;
            Some(if orig == comp {
                raw
            } else {
                crysirror_resource::lzss_decompress(&raw, orig as usize, &Default::default())?
            })
        } else {
            None
        };
        let img = if let Some(raw) = native.as_ref() {
            Some(images::cyber_image(raw, options(cx, id)?)?)
        } else {
            None
        };
        let output_size = if let Some(img) = &img {
            crysirror_image::png_uncompressed_len(img.width, img.height)? as u64
        } else if kind == b"w0" {
            orig.checked_sub(5).context("short Cyberworks w0")?
        } else if kind == b"u0" {
            orig.checked_sub(12).context("short Cyberworks u0")?
        } else {
            orig
        };
        ensure!(
            output_size <= cx.limits.max_entry_size,
            "Cyberworks final output limit"
        );
        let k = if matches!(kind.as_slice(), b"j0" | b"k0" | b"u0") {
            Some(key(cx, id)?)
        } else {
            None
        };
        let profile = options(cx, id).ok().cloned();
        let path = if image {
            format!("{file_id}.png")
        } else {
            format!("{file_id}.dat")
        };
        let limit = cx.limits.max_entry_size;
        let native_kind = String::from_utf8_lossy(&kind).into_owned();
        let plan = decode(move |input, _| {
            let raw = if let Some(dep) = &source {
                bytes(dep.as_ref(), at, comp, limit)?
            } else {
                input.to_vec()
            };
            let mut d = if orig == comp {
                raw
            } else {
                crysirror_resource::lzss_decompress(&raw, orig as usize, &Default::default())?
            };
            if image {
                return crysirror_image::encode_png_uncompressed(&images::cyber_image(
                    &d,
                    profile
                        .as_ref()
                        .context("Cyberworks image profile missing")?,
                )?);
            }
            if kind == b"w0" {
                d = d.get(5..).context("short Cyberworks w0")?.to_vec();
            }
            if kind == b"u0" {
                d = d.get(12..).context("short Cyberworks u0")?.to_vec();
            }
            if let Some(k) = &k {
                ensure!(!k.is_empty() && d.len() >= 4, "Cyberworks Ogg key/header");
                let mut key_at = 0;
                for v in d.iter_mut().take(0xe1f).skip(4) {
                    *v ^= k[key_at];
                    key_at += 1;
                    if key_at == k.len() {
                        key_at = 1;
                        ensure!(key_at < k.len(), "Cyberworks Ogg key must have >=2 bytes");
                    }
                }
                d[..4].copy_from_slice(b"OggS");
            }
            Ok(d)
        });
        let (offset, stored) = if base.is_some() { (at, comp) } else { (0, 0) };
        rows.push(
            Row::packed(path, offset, stored, output_size, plan)
                .meta(serde_json::json!({"file_id":file_id,"native_type":native_kind})),
        );
    }
    checked_rows(s, cx, rows)
}
