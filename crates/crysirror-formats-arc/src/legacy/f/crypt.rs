// SPDX-License-Identifier: GPL-3.0-or-later
// Native ciphers ported from arc_unpacker (C) 2016 rr-. Title secrets are
// supplied by the client key provider; no upstream title key tables are copied.
use super::*;

pub(super) fn leafpack(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let key = key_bytes(cx, "leaf/leafpack")?;
    let n = u16_at(s, 8)? as usize;
    limits(cx, n)?;
    let size = n as u64 * 24;
    let at = length(s)?
        .checked_sub(size)
        .context("LEAFPACK table size")?;
    let mut table = index(s, cx, at, size)?;
    key.with_bytes(|k| {
        for (i, b) in table.iter_mut().enumerate() {
            *b = b.wrapping_sub(k[i % k.len()])
        }
    });
    table
        .chunks_exact(24)
        .map(|r| {
            let mut name = sjis(zstr(&r[..12]))?;
            if let Some(start) = name.find(' ') {
                while name.as_bytes().get(start) == Some(&b' ') {
                    name.remove(start);
                }
            }
            if name.len() > 3 {
                name.insert(name.len() - 3, '.')
            }
            let key = key.clone();
            let size = u64::from(le32(r, 16)?);
            Ok(Row::packed(
                name,
                le32(r, 12)?.into(),
                size,
                size,
                decode(move |b, _| {
                    Ok(key.with_bytes(|k| {
                        b.iter()
                            .enumerate()
                            .map(|(i, b)| b.wrapping_sub(k[i % k.len()]))
                            .collect()
                    }))
                }),
            ))
        })
        .collect()
}

pub(super) fn kaguya_plan(cx: &DecodeContext, id: &str) -> Result<Plan> {
    let p = profile(cx, id).or_else(|_| profile(cx, "kaguya/link"))?;
    let alias = p["key_id"]
        .as_str()
        .context("Kaguya encrypted entry requires key_id")?;
    let key = cx.keys.key(cx.client, alias)?;
    let anm = p["decrypt_anm"]
        .as_bool()
        .context("Kaguya params require decrypt_anm")?;
    Ok(decode(move |b, _| {
        let mut out = b.to_vec();
        let spans = kaguya_spans(b, anm)?;
        key.with_bytes(|key| {
            for (at, len) in spans {
                for (i, b) in out[at..at + len].iter_mut().enumerate() {
                    *b ^= key[i % key.len()]
                }
            }
        });
        Ok(out)
    }))
}
fn kaguya_spans(b: &[u8], anm: bool) -> Result<Vec<(usize, usize)>> {
    let mut spans = Vec::new();
    let fixed = if b.starts_with(b"BM") {
        Some(54)
    } else if b.starts_with(b"AP-3") || b.starts_with(b"AP-2") {
        Some(24)
    } else if b.starts_with(b"AP") {
        Some(12)
    } else {
        None
    };
    if let Some(at) = fixed {
        ensure!(at <= b.len(), "truncated encrypted Kaguya image");
        return Ok(vec![(at, b.len() - at)]);
    }
    let mut c = Cursor::new(b);
    let tag = c.take(4)?;
    if !anm && [b"AN00".as_slice(), b"AN10", b"AN20", b"AN21"].contains(&tag) {
        return Ok(spans);
    }
    let v = match tag {
        b"AN00" if anm => 0,
        b"AN10" if anm => 10,
        b"AN20" if anm => 20,
        b"AN21" if anm => 21,
        b"PL00" => 100,
        b"PL10" => 110,
        _ => anyhow::bail!("unknown encrypted Kaguya resource layout"),
    };
    if v < 20 {
        c.take(16)?;
        let n = c.le16()? as usize;
        c.take(2 + n * 4)?;
    } else if v < 100 {
        let n = c.le16()?;
        c.take(2)?;
        for _ in 0..n {
            match c.u8()? {
                0 => {}
                1 => {
                    c.take(8)?;
                }
                2..=5 => {
                    c.take(4)?;
                }
                _ => anyhow::bail!("unknown Kaguya animation control"),
            }
        }
        let n = c.le16()? as usize;
        c.take(n * 8)?;
        if v == 21 {
            c.take(7)?;
        }
    }
    let n = c.le16()? as usize;
    if n == 0 {
        return Ok(spans);
    }
    if v >= 20 {
        c.take(16)?;
    }
    let n = if v == 21 || v == 110 { 1 } else { n };
    for _ in 0..n {
        c.take(8)?;
        let w = c.le32()? as u64;
        let h = c.le32()? as u64;
        let ch = if v == 0 { 4 } else { c.le32()? as u64 };
        let len = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(ch))
            .context("Kaguya pixel size overflow")?;
        let len = usize::try_from(len)?;
        let at = c.at;
        c.take(len)?;
        spans.push((at, len));
    }
    Ok(spans)
}

pub(super) fn npa_sg(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let key = key_bytes(cx, "nitroplus/npa-sg")?;
    let size = u32_at(s, 0)? as u64;
    let mut table = index(s, cx, 4, size)?;
    key.with_bytes(|k| {
        for (i, b) in table.iter_mut().enumerate() {
            *b ^= k[i % k.len()]
        }
    });
    let mut c = Cursor::new(&table);
    let n = c.le32()? as usize;
    limits(cx, n)?;
    let mut rows = Vec::new();
    for _ in 0..n {
        let ns = c.le32()? as usize;
        let path = utf16(c.take(ns)?)?;
        let size = u64::from(c.le32()?);
        let at = u64::from(c.le32()?);
        c.take(4)?;
        let key = key.clone();
        rows.push(Row::packed(
            path,
            at,
            size,
            size,
            decode(move |b, _| {
                Ok(key.with_bytes(|k| {
                    b.iter()
                        .enumerate()
                        .map(|(i, b)| b ^ k[i % k.len()])
                        .collect()
                }))
            }),
        ));
    }
    Ok(rows)
}
fn num(p: &serde_json::Value, name: &str) -> Result<u32> {
    let n = p[name]
        .as_u64()
        .with_context(|| format!("native profile missing integer {name}"))?;
    u32::try_from(n).with_context(|| format!("native profile integer overflow: {name}"))
}
fn array<const N: usize>(p: &serde_json::Value, name: &str) -> Result<[u32; N]> {
    let a = p[name]
        .as_array()
        .with_context(|| format!("native profile missing array {name}"))?;
    ensure!(a.len() == N, "native profile array length: {name}");
    let mut out = [0; N];
    for (i, n) in a.iter().enumerate() {
        out[i] = u32::try_from(n.as_u64().context("native profile unsigned integer")?)?;
    }
    Ok(out)
}
fn key_word(cx: &DecodeContext, p: &serde_json::Value, field: &str) -> Result<u32> {
    let alias = p[field]
        .as_str()
        .with_context(|| format!("NPA profile requires client key alias {field}"))?;
    cx.keys
        .key(cx.client, alias)?
        .with_bytes(|b| -> Result<u32> {
            ensure!(
                b.len() == 4,
                "NPA key word must contain four little-endian bytes"
            );
            Ok(u32::from_le_bytes(b.try_into().unwrap()))
        })
}
pub(super) fn npa(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let p = profile(cx, "nitroplus/npa")?;
    let name_key = key_word(cx, &p, "file_name_key_id")?;
    let h = bytes(s, 7, 34, 34)?;
    let k1 = le32(&h, 0)?;
    let k2 = le32(&h, 4)?;
    let comp = h[8] != 0;
    let encrypted = h[9] != 0;
    let n = le32(&h, 10)? as usize;
    limits(cx, n)?;
    let table_size = u64::from(le32(&h, 30)?);
    let base = 41 + table_size;
    let table = index(s, cx, 41, table_size)?;
    let mut c = Cursor::new(&table);
    let perm = if encrypted {
        let alias = p["key_id"]
            .as_str()
            .context("NPA encrypted profile requires key_id (256-byte permutation)")?;
        let key = cx.keys.key(cx.client, alias)?;
        Some(key.with_bytes(|k| -> Result<[u8; 256]> {
            k.try_into().context("NPA permutation must have 256 bytes")
        })?)
    } else {
        None
    };
    let dk = if encrypted {
        key_word(cx, &p, "data_key_id")?
    } else {
        0
    };
    let mut rows = Vec::new();
    for i in 0..n {
        let len = c.le32()? as usize;
        let mut name = c.take(len)?.to_vec();
        for (j, b) in name.iter_mut().enumerate() {
            let key = (j as u32)
                .wrapping_mul(0xfc)
                .wrapping_sub(name_key >> 24)
                .wrapping_sub(name_key >> 16)
                .wrapping_sub(name_key >> 8)
                .wrapping_sub(name_key & 255)
                .wrapping_sub((i >> 24) as u32)
                .wrapping_sub((i >> 16) as u32)
                .wrapping_sub((i >> 8) as u32)
                .wrapping_sub(i as u32);
            *b = b.wrapping_add(key as u8)
        }
        let kind = c.u8()?;
        c.take(4)?;
        let at = base + u64::from(c.le32()?);
        let stored = u64::from(c.le32()?);
        let orig = u64::from(c.le32()?);
        if kind == 1 {
            continue;
        }
        ensure!(kind == 2, "unknown NPA entry type");
        let path = sjis(&name)?;
        let plan = if encrypted || comp {
            let perm = perm;
            decode(move |b, e| {
                let mut b = b.to_vec();
                if let Some(perm) = perm {
                    let mut k = dk;
                    for n in &name {
                        k = k.wrapping_sub(u32::from(*n));
                    }
                    k = k
                        .wrapping_mul(name.len() as u32)
                        .wrapping_add(k1.wrapping_mul(k2))
                        .wrapping_mul(e.size as u32)
                        & 255;
                    for (i, b) in b.iter_mut().take(0x1000 + name.len()).enumerate() {
                        *b = perm[*b as usize]
                            .wrapping_sub(k as u8)
                            .wrapping_sub(i as u8)
                    }
                }
                if comp {
                    inflate(&b, e.size as usize)
                } else {
                    Ok(b)
                }
            })
        } else {
            Plan::Raw
        };
        rows.push(Row::packed(path, at, stored, orig, plan));
    }
    Ok(rows)
}

pub(super) fn npk2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let key = key_bytes(cx, "nitroplus/npk2")?;
    let iv: [u8; 16] = bytes(s, 8, 16, 16)?.try_into().unwrap();
    let n = u32_at(s, 24)? as usize;
    limits(cx, n)?;
    let size = u64::from(u32_at(s, 28)?);
    let encrypted = index(s, cx, 32, size)?;
    let table = key.with_bytes(|k| crysirror_resource::aes256_cbc_decrypt(&encrypted, &iv, k))?;
    let mut c = Cursor::new(&table);
    let mut rows = Vec::new();
    for _ in 0..n {
        c.take(1)?;
        let len = c.le16()? as usize;
        let path = sjis(c.take(len)?)?;
        c.take(36)?;
        let count = c.le32()? as usize;
        limits(cx, count)?;
        let mut segments = Vec::new();
        let (mut start, mut end, mut total) = (u64::MAX, 0u64, 0u64);
        for _ in 0..count {
            let at = c.le64()?;
            let aligned = c.le32()? as usize;
            let comp = c.le32()? as usize;
            let orig = c.le32()? as usize;
            range(length(s)?, at, aligned as u64)?;
            start = start.min(at);
            end = end.max(at + aligned as u64);
            total = total
                .checked_add(orig as u64)
                .context("NPK2 size overflow")?;
            segments.push((at, aligned, comp, orig));
        }
        sized(cx, total)?;
        if segments.is_empty() {
            start = 0
        }
        let key = key.clone();
        rows.push(Row::packed(
            path,
            start,
            end - start,
            total,
            decode(move |b, _| {
                let mut out = Vec::with_capacity(total as usize);
                for &(at, aligned, comp, orig) in &segments {
                    let at = (at - start) as usize;
                    let data = key.with_bytes(|k| {
                        crysirror_resource::aes256_cbc_decrypt(&b[at..at + aligned], &iv, k)
                    })?;
                    if orig > comp {
                        let mut decoded = Vec::new();
                        flate2::read::DeflateDecoder::new(data.as_slice())
                            .take(orig as u64 + 1)
                            .read_to_end(&mut decoded)?;
                        ensure!(decoded.len() == orig, "NPK2 segment size mismatch");
                        out.extend(decoded);
                    } else {
                        ensure!(data.len() == orig, "NPK2 plain segment size mismatch");
                        out.extend(data);
                    }
                }
                Ok(out)
            }),
        ));
    }
    Ok(rows)
}

#[derive(Clone)]
struct CpzPlugin {
    secret: crysirror_resource::KeyMaterial,
    hash_permutation: [u32; 4],
    hash_iv: [u32; 4],
    hash_xor: [u32; 4],
    hash_add: [u32; 4],
    mul: u32,
    add1: u32,
    add2: u32,
    tail_sub: u8,
    addends: [u32; 4],
    init_key: u32,
    start_pos: usize,
    tail_xor: u8,
}
impl CpzPlugin {
    fn read(cx: &DecodeContext, id: &str) -> Result<Self> {
        let p = profile(cx, id)?;
        let out = Self {
            secret: key_bytes(cx, id)?,
            hash_permutation: array(&p, "hash_permutation")?,
            hash_iv: array(&p, "hash_iv")?,
            hash_xor: array(&p, "hash_xor")?,
            hash_add: array(&p, "hash_add")?,
            mul: num(&p, "crypt_23_mul")?,
            add1: num(&p["crypt_1a"], "add1")?,
            add2: num(&p["crypt_1a"], "add2")?,
            tail_sub: num(&p["crypt_1a"], "tail_sub")?.try_into()?,
            addends: array(&p["crypt_1c"], "addends")?,
            init_key: num(&p["crypt_1c"], "init_key")?,
            start_pos: num(&p["crypt_3"], "start_pos")? as usize,
            tail_xor: num(&p["crypt_3"], "tail_xor")?.try_into()?,
        };
        ensure!(
            out.hash_permutation.iter().all(|x| *x < 4),
            "CPZ hash permutation"
        );
        out.secret.with_bytes(|s| {
            ensure!(s.len() >= 96, "CPZ secret requires 96 bytes");
            Ok(())
        })?;
        Ok(out)
    }
}
fn cpz_permutation(p: &CpzPlugin, mut key: u32, seed: u32) -> [u8; 256] {
    let mut table = std::array::from_fn(|i| i as u8);
    for _ in 0..256 {
        table.swap((key >> 16 & 255) as usize, (key & 255) as usize);
        table.swap((key >> 8 & 255) as usize, (key >> 24 & 255) as usize);
        key = seed.wrapping_add(key.rotate_right(2).wrapping_mul(p.mul));
    }
    table
}
fn cpz_1a(b: &mut [u8], p: &CpzPlugin, key: u32) {
    let table: [u32; 24] = p.secret.with_bytes(|s| {
        std::array::from_fn(|i| {
            u32::from_le_bytes(s[i * 4..i * 4 + 4].try_into().unwrap()).wrapping_sub(key)
        })
    });
    let mut shift = key;
    for _ in 0..3 {
        shift = (shift >> 8) ^ key
    }
    shift = ((shift ^ 0xfb) & 15) + 7;
    let mut pos = 5;
    let mut c = b.chunks_exact_mut(4);
    for w in &mut c {
        let n = (u32::from_le_bytes(w.try_into().unwrap()) ^ table[pos])
            .wrapping_add(p.add1)
            .rotate_right(shift)
            .wrapping_add(p.add2);
        w.copy_from_slice(&n.to_le_bytes());
        pos = (pos + 1) % 24;
    }
    for (i, w) in c.into_remainder().iter_mut().enumerate() {
        *w = (*w ^ (table[pos] >> (4 * (3 - i))) as u8).wrapping_sub(p.tail_sub);
        pos = (pos + 1) % 24;
    }
}
fn cpz_1b(b: &mut [u8], key: u32, hash: [u32; 4]) {
    let add = [0x76a3bf29u32, 0, 0x10000000, 0];
    let table: [u32; 4] = std::array::from_fn(|i| hash[i] ^ key.wrapping_add(add[i]));
    let mut pos = 0;
    let mut seed = 0x76548aefu32;
    let mut c = b.chunks_exact_mut(4);
    for w in &mut c {
        let n = (u32::from_le_bytes(w.try_into().unwrap()) ^ table[pos])
            .wrapping_sub(0x4a91c262)
            .rotate_left(3)
            .wrapping_sub(seed);
        w.copy_from_slice(&n.to_le_bytes());
        pos = (pos + 1) % 4;
        seed = seed.wrapping_add(0x10fb562a);
    }
    for w in c.into_remainder() {
        *w = (*w ^ (table[pos] >> 6) as u8).wrapping_add(0x37);
        pos = (pos + 1) % 4;
    }
}
fn cpz_1c(b: &mut [u8], p: &CpzPlugin, key: u32, hash: [u32; 4]) {
    let table: [u32; 4] = std::array::from_fn(|i| hash[i] ^ key.wrapping_add(p.addends[i]));
    let mut pos = 0;
    let mut seed = p.init_key;
    let mut c = b.chunks_exact_mut(4);
    for w in &mut c {
        let n = (u32::from_le_bytes(w.try_into().unwrap()) ^ table[pos])
            .wrapping_sub(seed)
            .rotate_left(2)
            .wrapping_add(0x37a19e8b);
        w.copy_from_slice(&n.to_le_bytes());
        pos = (pos + 1) % 4;
        seed = seed.wrapping_sub(0x139fa9b);
    }
    for w in c.into_remainder() {
        *w = (*w ^ (table[pos] >> 4) as u8).wrapping_add(5);
        pos = (pos + 1) % 4;
    }
}
fn cpz_2(b: &mut [u8], p: &CpzPlugin, key: u32, seed: u32, xor: u8) {
    let table = cpz_permutation(p, key, seed);
    for w in b {
        *w = table[(*w ^ xor) as usize];
    }
}
fn cpz_3(b: &[u8], p: &CpzPlugin, main: u32, hash: [u32; 4], key: u32) -> Vec<u8> {
    let table = cpz_permutation(p, hash[3], main);
    let raw: [u8; 96] = p
        .secret
        .with_bytes(|s| std::array::from_fn(|i| table[s[i] as usize] ^ ((hash[1] >> 2) as u8)));
    let words: [u32; 24] = std::array::from_fn(|i| {
        u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap()) ^ key
    });
    let (mut pos, mut state) = (p.start_pos, 0x2547a39eu32);
    let mut out = b.to_vec();
    let mut c = out.chunks_exact_mut(4);
    for w in &mut c {
        let n = u32::from_le_bytes(w.try_into().unwrap())
            ^ (words[pos & 15] >> 1)
            ^ words[(state >> 6 & 15) as usize];
        let n = hash[(state & 3) as usize] ^ n.wrapping_sub(key);
        w.copy_from_slice(&n.to_le_bytes());
        pos = pos.wrapping_add(1);
        state = state.wrapping_add(n).wrapping_add(key);
    }
    for b in c.into_remainder() {
        *b = table[(*b ^ p.tail_xor) as usize]
    }
    out
}
pub(super) fn cpz(s: &dyn AssetReader, cx: &DecodeContext, v: u8) -> Result<Vec<Row>> {
    let id = if v == 5 {
        "purple-software/cpz5"
    } else {
        "purple-software/cpz6"
    };
    let plugin = CpzPlugin::read(cx, id)?;
    let h = bytes(s, 4, 60, 60)?;
    let delta = u32::from(v - 5);
    let n = le32(&h, 0)? ^ (0xfe3a53d9 + delta);
    limits(cx, n as usize)?;
    let dirs = (le32(&h, 4)? ^ (0x37f298e7 + delta)) as u64;
    let files = (le32(&h, 8)? ^ (0x7a6f3a2c + delta)) as u64;
    ensure!(dirs > 0 && files > 0, "CPZ empty index");
    let mut md5in = Vec::new();
    for (i, k) in [0x43de7c19u32, 0xcc65f415, 0xd016a93c, 0x97a3ba9a]
        .iter()
        .enumerate()
    {
        md5in.extend((le32(&h, 28 + i * 4)? ^ (k + delta)).to_le_bytes())
    }
    let md5 = crysirror_resource::md5_with_iv(&md5in, plugin.hash_iv);
    let hash: [u32; 4] = std::array::from_fn(|i| {
        (md5[plugin.hash_permutation[i] as usize] ^ plugin.hash_xor[i])
            .wrapping_add(plugin.hash_add[i])
    });
    let main = le32(&h, 44)? ^ if v == 5 { 0xae7d39bf } else { 0xae7d39b7 };
    let extra = if v == 5 {
        0
    } else {
        (le32(&h, 52)? ^ 0x37acf832)
            .rotate_right(5)
            .wrapping_mul(0x7da8f173)
            .wrapping_add(0x13712765)
    };
    let table_size = dirs.checked_add(files).context("CPZ index size overflow")?;
    let data = 64 + table_size;
    let mut table = index(s, cx, 64, table_size)?;
    cpz_1a(&mut table, &plugin, main ^ 0x3795b39a);
    cpz_2(&mut table[..dirs as usize], &plugin, main, hash[1], 0x3a);
    cpz_1b(&mut table[..dirs as usize], main, hash);
    let mut c = Cursor::new(&table[..dirs as usize]);
    let mut ds = Vec::new();
    for _ in 0..n {
        let start = c.at;
        let size = c.le32()? as usize;
        ensure!(size >= 17, "CPZ directory record size");
        let count = c.le32()? as usize;
        limits(cx, count)?;
        let at = c.le32()? as usize;
        let key = c.le32()?;
        let path = sjis(c.zero()?)?;
        ensure!(size >= c.at - start, "CPZ directory record truncation");
        c.at = start;
        c.take(size)?;
        ds.push((count, at, key, path));
    }
    let mut rows = Vec::new();
    for i in 0..ds.len() {
        let (count, at, key, ref path) = ds[i];
        let end = ds.get(i + 1).map_or(files as usize, |d| d.1);
        ensure!(at <= end && end <= files as usize, "CPZ file table offsets");
        let t = &mut table[dirs as usize + at..dirs as usize + end];
        cpz_2(t, &plugin, main, hash[2], 0x7e);
        cpz_1c(t, &plugin, key, hash);
        let mut c = Cursor::new(t);
        for _ in 0..count {
            let start = c.at;
            let size = c.le32()? as usize;
            ensure!(size >= 25, "CPZ file record size");
            let at = data
                .checked_add(c.le64()?)
                .context("CPZ entry offset overflow")?;
            let len = u64::from(c.le32()?);
            c.take(4)?;
            let tmp = c.le32()?;
            let entry_key = (main ^ key.wrapping_add(tmp))
                .wrapping_add(n)
                .wrapping_sub(0x5c29e87b)
                ^ extra;
            let name = sjis(c.zero()?)?;
            let name = if path.rsplit(['/', '\\']).next() == Some("root") {
                name
            } else {
                format!("{path}/{name}")
            };
            ensure!(size >= c.at - start, "CPZ file record truncation");
            c.at = start;
            c.take(size)?;
            let plugin = plugin.clone();
            rows.push(Row::packed(
                name,
                at,
                len,
                len,
                decode(move |b, _| Ok(cpz_3(b, &plugin, main, hash, entry_key))),
            ));
            limits(cx, rows.len())?;
        }
    }
    Ok(rows)
}

const PACK_MAGIC: &[u8] = b"FilePackVer3.0\0\0";
pub(super) fn probe_pack(s: &dyn AssetReader) -> Result<bool> {
    let n = length(s)?;
    Ok(n >= 28 && magic(s, n - 28, PACK_MAGIC)?)
}
fn padd(a: u64, b: u64) -> u64 {
    u64::from((a as u32).wrapping_add(b as u32))
        | (u64::from((a >> 32) as u32).wrapping_add(u64::from((b >> 32) as u32)) & 0xffffffff) << 32
}
fn padw(a: u64, b: u64) -> u64 {
    (0..4).fold(0, |o, i| {
        o | (((a >> (i * 16)) as u16).wrapping_add((b >> (i * 16)) as u16) as u64) << (i * 16)
    })
}
fn padb(a: u64, b: u64) -> u64 {
    (0..8).fold(0, |o, i| {
        o | (((a >> (i * 8)) as u8).wrapping_add((b >> (i * 8)) as u8) as u64) << (i * 8)
    })
}
fn derive_seed(b: &[u8]) -> u32 {
    let (mut key, mut result) = (0u64, 0u64);
    for word in b.chunks_exact(8) {
        key = padw(key, 0x0307030703070307);
        result = padw(result, u64::from_le_bytes(word.try_into().unwrap()) ^ key);
    }
    (result ^ (result >> 32)) as u32
}
struct QlieMt {
    state: [u32; 64],
    at: usize,
}
impl QlieMt {
    fn new(seed: u32) -> Self {
        let mut s = [0u32; 64];
        s[0] = seed;
        for i in 1..64 {
            s[i] = (s[i - 1] ^ (s[i - 1] >> 30))
                .wrapping_mul(1712438297)
                .wrapping_add(i as u32)
        }
        Self { state: s, at: 64 }
    }
    fn xor(&mut self, b: &[u8]) {
        for (s, k) in self.state.iter_mut().zip(b.chunks_exact(4)) {
            *s ^= u32::from_le_bytes(k.try_into().unwrap());
        }
    }
    fn next(&mut self) -> u32 {
        if self.at >= 64 {
            for i in 0..63 {
                let y = (self.state[i] & 0x80000000) | ((self.state[i + 1] & 0x7fffffff) >> 1);
                self.state[i] = self.state[(i + 39) % 64]
                    ^ y
                    ^ if self.state[i + 1] & 1 != 0 {
                        0x9908b0df
                    } else {
                        0
                    };
            }
            let y = (self.state[63] & 0x80000000) | ((self.state[0] & 0x7fffffff) >> 1);
            self.state[63] = self.state[38]
                ^ y
                ^ if self.state[63] & 1 != 0 {
                    0x9908b0df
                } else {
                    0
                };
            self.at = 0;
        }
        let mut y = self.state[self.at];
        self.at += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9c4f88e3;
        y ^= (y << 15) & 0xe7f70000;
        y ^= y >> 18;
        y
    }
    fn wide(&mut self) -> u64 {
        let lo = self.next();
        u64::from(lo) | (u64::from(self.next()) << 32)
    }
}
fn pack_decrypt(
    b: &[u8],
    seed: u32,
    name: &[u8],
    key1: Option<&[u8]>,
    key2: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = b.to_vec();
    if key1.is_none() && key2.is_none() {
        let mut key = 0xa73c5f9da73c5f9du64;
        let state = seed.wrapping_add(b.len() as u32) ^ 0xfec9753e;
        let mut state = u64::from(state) | (u64::from(state) << 32);
        for w in out.chunks_exact_mut(8) {
            key = padd(key, 0xce24f523ce24f523) ^ state;
            state = u64::from_le_bytes(w.try_into().unwrap()) ^ key;
            w.copy_from_slice(&state.to_le_bytes());
        }
        return out;
    }
    let (mut mutator, mut state) = (0x85f532u32, 0x33f641u32);
    for (i, b) in name.iter().enumerate() {
        mutator = mutator.wrapping_add(u32::from(*b) * (i as u8 as u32));
        state ^= mutator;
    }
    let n = b.len() as u32;
    state = state.wrapping_add(
        seed ^ (7u32
            .wrapping_mul(n & 0xffffff)
            .wrapping_add(n)
            .wrapping_add(mutator)
            .wrapping_add(mutator ^ n ^ 0x8f32dc)),
    );
    state = (state & 0xffffff).wrapping_mul(9);
    if key2.is_some_and(|k| !k.is_empty()) {
        state ^= 0x453a
    }
    let mut mt = QlieMt::new(state);
    if let Some(k) = key1 {
        mt.xor(k)
    }
    if let Some(k) = key2 {
        mt.xor(k)
    }
    let table: [u64; 16] = std::array::from_fn(|_| mt.wide());
    for _ in 0..9 {
        mt.next();
    }
    let mut state = mt.wide();
    let mut at = mt.next() as usize % 16;
    for w in out.chunks_exact_mut(8) {
        state ^= table[at];
        state = padd(state, table[at]);
        let v = u64::from_le_bytes(w.try_into().unwrap()) ^ state;
        w.copy_from_slice(&v.to_le_bytes());
        state = padb(state, v) ^ v;
        state = (state << 1) & 0xfffffffefffffffe;
        state = padw(state, v);
        at = (at + 1) % 16;
    }
    out
}
fn pack_decompress(b: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(b);
    ensure!(
        c.take(4)? == b"1PC\xff",
        "invalid QLiE compression magic / decryption profile"
    );
    let short = c.le32()? > 0;
    ensure!(c.le32()? as usize == size, "QLiE compressed file size");
    let mut out = Vec::with_capacity(size);
    while c.left() > 0 {
        let mut d1 = std::array::from_fn::<u8, 256, _>(|i| i as u8);
        let mut d2 = [0u8; 256];
        let mut d = 0;
        while d < 256 {
            let mut count = c.u8()? as usize;
            if count > 127 {
                d += count - 127;
                count = 0;
                if d >= 256 {
                    break;
                }
            }
            for _ in 0..=count {
                ensure!(d < 256, "QLiE dictionary overrun");
                d1[d] = c.u8()?;
                if d1[d] as usize != d {
                    d2[d] = c.u8()?
                }
                d += 1;
            }
        }
        let mut left = if short {
            c.le16()? as usize
        } else {
            c.le32()? as usize
        };
        let mut stack = Vec::new();
        let mut steps = 0usize;
        while out.len() < size {
            steps += 1;
            ensure!(
                steps <= size.saturating_mul(512).saturating_add(1),
                "cyclic QLiE dictionary"
            );
            let d = if let Some(d) = stack.pop() {
                d
            } else {
                if left == 0 {
                    break;
                }
                left -= 1;
                c.u8()?
            };
            let index = d as usize;
            if d1[index] == d {
                out.push(d)
            } else {
                ensure!(stack.len() < 256, "QLiE dictionary stack overflow");
                stack.push(d2[index]);
                stack.push(d1[index]);
            }
        }
    }
    ensure!(out.len() == size, "QLiE decompressed size mismatch");
    Ok(out)
}
pub(super) fn pack(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let tail = length(s)?
        .checked_sub(28)
        .context("truncated QLiE footer")?;
    ensure!(magic(s, tail, PACK_MAGIC)?, "QLiE footer signature");
    let n = u32_at(s, tail + 16)? as usize;
    limits(cx, n)?;
    let at = u64::from_le_bytes(bytes(s, tail + 20, 8, 8)?.try_into().unwrap());
    let len = tail.checked_sub(at).context("QLiE table offset")?;
    let table = index(s, cx, at, len)?;
    let mut c = Cursor::new(&table);
    for _ in 0..n {
        let ns = c.le16()? as usize;
        c.take(ns + 28)?;
    }
    c.take(28)?;
    let extra = c.le32()? as usize;
    c.take(extra + 36)?;
    let seed = derive_seed(c.take(256)?) & 0x0fffffff;
    c.at = 0;
    let p = cx.parameters.get("qlie/pack");
    let mut key1 = if let Some(alias) = p.and_then(|p| p["key1_id"].as_str()) {
        Some(cx.keys.key(cx.client, alias)?)
    } else {
        None
    };
    let key2 = if let Some(alias) = p.and_then(|p| p["key2_id"].as_str()) {
        Some(cx.keys.key(cx.client, alias)?)
    } else {
        None
    };
    let basic = p.and_then(|p| p["basic"].as_bool()) == Some(true);
    ensure!(
        !basic || (key1.is_none() && key2.is_none()),
        "QLiE basic profile cannot also select external keys"
    );
    let mut specs = Vec::new();
    for _ in 0..n {
        let len = c.le16()? as usize;
        let mut name = c.take(len)?.to_vec();
        let x = ((seed ^ 0x3e).wrapping_add(len as u32)) as u8;
        for (i, b) in name.iter_mut().enumerate() {
            let i = (i + 1) as u8;
            *b ^= (i ^ x).wrapping_add(i);
        }
        let path = sjis(&name)?;
        let at = c.le64()?;
        let stored = u64::from(c.le32()?);
        let size = u64::from(c.le32()?);
        let compressed = c.le32()? > 0;
        let encrypted = c.le32()? > 0;
        c.take(4)?;
        specs.push((path, name, at, stored, size, compressed, encrypted));
    }
    // Native archives can carry an updated fkey. Resolve it from this archive,
    // under the same bounded resource authority, without scanning a directory.
    if !basic {
        for (path, name, at, stored, size, compressed, encrypted) in &specs {
            if !path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(path)
                .contains("pack_keyfile")
            {
                continue;
            }
            ensure!(
                !encrypted || key1.is_some() || key2.is_some(),
                "QLiE embedded key is encrypted; caller key profile required"
            );
            ensure!(
                *size <= cx.limits.max_index_size && *size <= 1 << 20,
                "QLiE embedded key byte limit"
            );
            let data = index(s, cx, *at, *stored)?;
            let data = if *encrypted {
                let one = key1.as_ref().map(|k| k.with_bytes(|k| k.to_vec()));
                let two = key2.as_ref().map(|k| k.with_bytes(|k| k.to_vec()));
                pack_decrypt(&data, seed, name, one.as_deref(), two.as_deref())
            } else {
                data
            };
            let data = if *compressed {
                pack_decompress(&data, *size as usize)?
            } else {
                data
            };
            ensure!(data.len() as u64 == *size, "QLiE embedded key size");
            key1 = Some(crysirror_resource::KeyMaterial::new(data)?);
        }
    }
    let mut rows = Vec::new();
    for (path, name, at, stored, size, compressed, encrypted) in specs {
        let plan = if encrypted && (!basic && key1.is_none() && key2.is_none()) {
            Plan::Unsupported("QLiE encrypted entry requires explicit basic=true or client key1_id/key2_id profile")
        } else if encrypted || compressed {
            let (k1, k2) = (key1.clone(), key2.clone());
            decode(move |b, e| {
                let data = if encrypted {
                    let one = k1.as_ref().map(|k| k.with_bytes(|k| k.to_vec()));
                    let two = k2.as_ref().map(|k| k.with_bytes(|k| k.to_vec()));
                    pack_decrypt(b, seed, &name, one.as_deref(), two.as_deref())
                } else {
                    b.to_vec()
                };
                if compressed {
                    pack_decompress(&data, e.size as usize)
                } else {
                    Ok(data)
                }
            })
        } else {
            Plan::Raw
        };
        rows.push(Row::packed(path, at, stored, size, plan));
    }
    Ok(rows)
}
