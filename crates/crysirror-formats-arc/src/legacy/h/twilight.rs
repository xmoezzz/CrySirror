// SPDX-License-Identifier: GPL-3.0-or-later
// Ported from arc_unpacker (C) 2016 rr-; MT19937 originally Matsumoto/Nishimura.
use super::*;
use std::collections::BTreeMap;
fn crypt(b: &mut [u8], mut a: u8, mut key: u8, delta: u8) {
    for b in b {
        *b ^= a;
        a = a.wrapping_add(key);
        key = key.wrapping_add(delta);
    }
}
pub(super) fn pak1(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u16_at(s, 0)? as usize;
    limits(cx, n)?;
    ensure!(n > 0 || length(s)? == 6, "PAK1 zero entry count");
    let mut t = index(s, cx, 2, n as u64 * 108)?;
    crypt(&mut t, 0x64, 0x64, 0x4d);
    t.chunks_exact(108)
        .map(|r| {
            let mut path = sjis(zstr(&r[..100]))?;
            let size = u64::from(le32(r, 100)?);
            let at = u64::from(le32(r, 104)?);
            let base = path.rsplit(['/', '\\']).next().unwrap_or(&path);
            let cipher = if base == "musicroom.dat" {
                Some((0x5c, 0x5a, 0x3d))
            } else if has_ext(&path, "sce") {
                Some((0x63, 0x62, 0x42))
            } else if base == "cardlist.dat" {
                Some((0x60, 0x61, 0x41))
            } else {
                None
            };
            let plan = if let Some((a, b, d)) = cipher {
                path = ext(&path, "txt");
                decode(move |input, _| {
                    let mut out = input.to_vec();
                    crypt(&mut out, a, b, d);
                    Ok(out)
                })
            } else {
                Plan::Raw
            };
            Ok(Row::packed(path, at, size, size, plan))
        })
        .collect()
}
struct Mt {
    state: [u32; 624],
    at: usize,
}
impl Mt {
    fn new(seed: u32) -> Self {
        let mut s = [0u32; 624];
        s[0] = seed;
        for i in 1..624 {
            s[i] = (s[i - 1] ^ (s[i - 1] >> 30))
                .wrapping_mul(1812433253)
                .wrapping_add(i as u32)
        }
        Self { state: s, at: 624 }
    }
    fn next(&mut self) -> u32 {
        if self.at >= 624 {
            for i in 0..624 {
                let y = (self.state[i] & 0x80000000) | (self.state[(i + 1) % 624] & 0x7fffffff);
                self.state[i] = self.state[(i + 397) % 624]
                    ^ (y >> 1)
                    ^ if y & 1 != 0 { 0x9908b0df } else { 0 };
            }
            self.at = 0
        }
        let mut y = self.state[self.at];
        self.at += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;
        y
    }
}
pub(super) fn pak2(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u16_at(s, 0)? as usize;
    limits(cx, n)?;
    ensure!(n > 0 || length(s)? == 6, "PAK2 zero entry count");
    let size = u64::from(u32_at(s, 2)?);
    ensure!(size <= n as u64 * 265, "PAK2 table size");
    let mut t = index(s, cx, 6, size)?;
    let mut mt = Mt::new((size + 6) as u32);
    let (mut a, mut b) = (0xc5u8, 0x83u8);
    for t in &mut t {
        *t ^= mt.next() as u8 ^ a;
        a = a.wrapping_add(b);
        b = b.wrapping_add(0x53);
    }
    let mut c = Cursor::new(&t);
    let mut rows = Vec::new();
    for _ in 0..n {
        let at = u64::from(c.le32()?);
        let size = u64::from(c.le32()?);
        let ns = c.u8()? as usize;
        let path = sjis(c.take(ns)?)?;
        let key = ((at >> 1) | 0x23) as u8;
        rows.push(Row::packed(
            path,
            at,
            size,
            size,
            decode(move |b, _| Ok(b.iter().map(|b| b ^ key).collect())),
        ));
    }
    Ok(rows)
}
pub(super) fn pak1_gfx(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = bytes(s, 0, 1, 1)?[0] as u64;
    let p = index(s, cx, 1, n * 512)?;
    let mut pal = Vec::new();
    if n > 0 {
        let mut c = Cursor::new(&p[..512]);
        for _ in 0..256 {
            pal.push(pixel(&mut c, 16)?);
        }
    } else {
        pal.resize(256, [0; 4]);
    }
    let mut at = 1 + n * 512;
    let total = length(s)?;
    let mut rows = Vec::new();
    while at < total {
        let h = bytes(s, at, 17, 17)?;
        let w = le32(&h, 0)?;
        let height = le32(&h, 4)?;
        let depth = h[12];
        let stored = u64::from(le32(&h, 13)?);
        at += 17;
        let size = png_size(cx, w, height)?;
        let palette = pal.clone();
        let i = rows.len();
        rows.push(Row::packed(
            format!("{i:04}.png"),
            at,
            stored,
            size,
            decode(move |b, _| {
                let chunk = match depth {
                    32 | 24 => 4,
                    16 => 2,
                    8 => 1,
                    _ => anyhow::bail!("PAK1 unsupported pixel depth"),
                };
                let count = crysirror_image::rgba_len(w, height)? / 4;
                let mut c = Cursor::new(b);
                let mut raw = Vec::with_capacity(count * chunk);
                while raw.len() < count * chunk {
                    let repeat = match chunk {
                        4 => c.le32()? as usize,
                        2 => c.le16()? as usize,
                        _ => c.u8()? as usize,
                    };
                    let v = c.take(chunk)?;
                    ensure!(
                        repeat > 0 && repeat <= (count * chunk - raw.len()) / chunk,
                        "PAK1 pixel run overflow"
                    );
                    for _ in 0..repeat {
                        raw.extend_from_slice(v);
                    }
                }
                let mut c = Cursor::new(&raw);
                let mut rgba = Vec::with_capacity(count * 4);
                for _ in 0..count {
                    let mut p = if depth == 8 {
                        palette[c.u8()? as usize]
                    } else {
                        pixel(&mut c, if depth == 16 { 16 } else { 1 })?
                    };
                    if depth == 24 {
                        p[3] = 255
                    }
                    rgba.extend(p);
                }
                png(w, height, rgba)
            }),
        ));
        range(total, at, stored)?;
        at += stored;
        limits(cx, rows.len())?;
    }
    Ok(rows)
}
pub(super) fn pak1_sfx(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 0)? as usize;
    limits(cx, n)?;
    let mut at = 4;
    let mut rows = Vec::new();
    for i in 0..n {
        let present = bytes(s, at, 1, 1)?[0];
        at += 1;
        if present == 0 {
            continue;
        }
        let h = bytes(s, at, 22, 22)?;
        let stored = u64::from(le32(&h, 0)?);
        let channels = u16::from_le_bytes(h[6..8].try_into().unwrap());
        let rate = le32(&h, 8)?;
        let bits = u16::from_le_bytes(h[18..20].try_into().unwrap());
        at += 22;
        let spec = crysirror_resource::WaveSpec {
            format: 1,
            channels,
            sample_rate: rate,
            bits_per_sample: bits,
            ..Default::default()
        };
        let size = spec.encoded_len(stored as usize)? as u64;
        entry_size(cx, size)?;
        rows.push(Row::packed(
            format!("{i:04}.wav"),
            at,
            stored,
            size,
            decode(move |b, _| spec.encode(b)),
        ));
        range(length(s)?, at, stored)?;
        at += stored;
    }
    Ok(rows)
}
struct RsaReader<'a> {
    s: &'a dyn AssetReader,
    cx: &'a DecodeContext,
    at: u64,
    modulus: Option<crysirror_resource::KeyMaterial>,
    exponent: u32,
}
impl RsaReader<'_> {
    fn read(&mut self) -> Result<Vec<u8>> {
        ensure!(
            self.at
                .checked_add(64)
                .is_some_and(|n| n <= self.cx.limits.max_index_size.saturating_add(5)),
            "TFPK index byte limit"
        );
        let b = bytes(self.s, self.at, 64, 64)?;
        self.at += 64;
        let data = if let Some(k) = &self.modulus {
            k.with_bytes(|k| crysirror_resource::rsa_public_decrypt(&b, k, self.exponent))?
        } else {
            b
        };
        ensure!(data.len() >= 32, "TFPK RSA block payload too short");
        Ok(data[..32].to_vec())
    }
}
fn hash_name(name: &str, version: u8, initial: u32) -> Result<u32> {
    let normalized = name.replace('/', "\\").to_ascii_lowercase();
    let (b, _, errors) = encoding_rs::SHIFT_JIS.encode(&normalized);
    ensure!(!errors, "TFPK filename cannot be encoded as CP932");
    let mut hash = initial;
    for b in b.iter() {
        let b = *b as i8 as i32 as u32;
        if version == 0 {
            hash = hash.wrapping_mul(0x1000193) ^ b
        } else {
            hash = (hash ^ b).wrapping_mul(0x1000193)
        }
    }
    Ok(if version == 0 {
        hash
    } else {
        hash.wrapping_neg()
    })
}
fn unknown(i: usize, hash: u32, extension: &str) -> String {
    if i == 0 {
        format!("unk-{hash:08x}{extension}")
    } else {
        format!("unk-{i:05}-{hash:08x}{extension}")
    }
}
pub(super) fn tfpk(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let version = bytes(s, 4, 1, 1)?[0];
    ensure!(version <= 1, "unknown TFPK version");
    let p = cx.parameters.get("twilight-frontier/tfpk");
    let modulus = if let Some(alias) = p.and_then(|p| p["rsa_key_id"].as_str()) {
        Some(cx.keys.key(cx.client, alias)?)
    } else {
        let first = bytes(s, 5, 64, 64)?;
        ensure!(
            first[4..16].iter().all(|b| *b == 0),
            "TFPK encrypted index requires rsa_key_id client alias"
        );
        None
    };
    let exponent = p
        .and_then(|p| p["rsa_exponent"].as_u64())
        .unwrap_or(65537)
        .try_into()?;
    let mut reader = RsaReader {
        s,
        cx,
        at: 5,
        modulus,
        exponent,
    };
    let mut names = BTreeMap::new();
    if let Some(files) = p.and_then(|p| p["file_names"].as_array()) {
        for name in files {
            let name = name.as_str().context("TFPK file_names strings required")?;
            names.insert(hash_name(name, version, 0x811c9dc5)?, name.to_string());
        }
    }
    let count = le32(&reader.read()?, 0)? as usize;
    limits(cx, count)?;
    let mut dirs = Vec::new();
    for _ in 0..count {
        let d = reader.read()?;
        dirs.push((le32(&d, 0)?, le32(&d, 4)? as usize));
    }
    let mut detected = BTreeMap::new();
    if !dirs.is_empty() {
        let header = reader.read()?;
        let comp = le32(&header, 0)? as usize;
        let orig = le32(&header, 4)? as usize;
        let blocks = le32(&header, 8)? as usize;
        ensure!(
            orig as u64 <= cx.limits.max_index_size
                && blocks as u64 * 64 <= cx.limits.max_index_size,
            "TFPK filename table byte limit"
        );
        let mut table = Vec::new();
        for _ in 0..blocks {
            table.extend(reader.read()?);
        }
        ensure!(comp <= table.len(), "TFPK filename compressed length");
        let table = inflate_index(&table[..comp], orig as u64)?;
        ensure!(table.len() == orig, "TFPK filename table size");
        let mut c = Cursor::new(&table);
        for (hash, count) in dirs {
            limits(cx, count)?;
            let dir = names
                .get(&hash)
                .cloned()
                .unwrap_or_else(|| unknown(0, hash, ""));
            for _ in 0..count {
                let raw = c.zero()?;
                if let Ok(name) = sjis(raw) {
                    detected.insert(
                        hash_name(&name, version, hash)?,
                        format!("{}/{name}", dir.trim_end_matches('/')),
                    );
                }
            }
        }
    }
    detected.extend(names);
    let n = le32(&reader.read()?, 0)? as usize;
    limits(cx, n)?;
    let mut specs = Vec::new();
    for i in 0..n {
        let one = reader.read()?;
        let two = reader.read()?;
        let three = reader.read()?;
        let (size, offset, hash, key) = if version == 0 {
            (
                le32(&one, 0)?,
                le32(&one, 4)?,
                le32(&two, 0)?,
                three[..16].to_vec(),
            )
        } else {
            let mut key = Vec::new();
            for w in three[..16].chunks_exact(4) {
                key.extend(le32(w, 0)?.wrapping_neg().to_le_bytes())
            }
            (
                le32(&one, 0)? ^ le32(&three, 0)?,
                le32(&one, 4)? ^ le32(&three, 4)?,
                le32(&two, 0)? ^ le32(&three, 8)?,
                key,
            )
        };
        let path = detected
            .get(&hash)
            .cloned()
            .unwrap_or_else(|| unknown(if version == 0 { i } else { i + 1 }, hash, ".dat"));
        specs.push((path, size, offset, key, hash));
    }
    let end = reader.at;
    Ok(specs
        .into_iter()
        .map(|(path, size, offset, key, hash)| {
            Row::packed(
                path,
                end + u64::from(offset),
                u64::from(size),
                u64::from(size),
                decode(move |b, _| {
                    let mut out = b.to_vec();
                    if version == 0 {
                        for (i, b) in out.iter_mut().enumerate() {
                            *b ^= key[i % 16]
                        }
                    } else {
                        let mut aux: [u8; 4] = key[..4].try_into().unwrap();
                        for (i, b) in out.iter_mut().enumerate() {
                            let old = *b;
                            *b = old ^ key[i % 16] ^ aux[i & 3];
                            aux[i & 3] = old;
                        }
                    }
                    Ok(out)
                }),
            )
            .meta(serde_json::json!({"hash":hash}))
        })
        .collect())
}
