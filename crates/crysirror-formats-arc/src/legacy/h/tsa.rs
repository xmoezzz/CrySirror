// SPDX-License-Identifier: GPL-3.0-or-later
// Ported from arc_unpacker (C) 2016 rr-. Per-title cipher maps are caller data.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
fn lzss(b: &[u8], size: usize) -> Result<Vec<u8>> {
    crysirror_resource::lzss_decompress(
        b,
        size,
        &crysirror_resource::LzssParams {
            mode: crysirror_resource::LzssMode::Bitwise,
            position_bits: 13,
            size_bits: 4,
            min_match: 3,
            initial: 1,
            invert_length: false,
        },
    )
}
#[derive(Clone, Copy)]
struct Cipher {
    key: u8,
    step: u8,
    block: usize,
    limit: usize,
}
impl Cipher {
    fn read(cx: &DecodeContext, p: &serde_json::Value) -> Result<Self> {
        let alias = p["key_id"]
            .as_str()
            .context("TSA decryptor requires client key_id (two bytes: key, step)")?;
        let key = cx
            .keys
            .key(cx.client, alias)?
            .with_bytes(|b| -> Result<[u8; 2]> {
                b.try_into()
                    .context("TSA decryptor key must contain key and step bytes")
            })?;
        let c = Self {
            key: key[0],
            step: key[1],
            block: number(p, "block_size")?.try_into()?,
            limit: number(p, "limit")?.try_into()?,
        };
        ensure!(c.block > 0 && c.block <= 1 << 20, "TSA cipher block size");
        Ok(c)
    }
}
fn crypt(input: &[u8], c: Cipher) -> Result<Vec<u8>> {
    let mut left = input.len();
    let mut block = c.block;
    let mut key = c.key;
    let mut shift = left % block;
    if shift >= block / 4 {
        shift = 0
    }
    shift += left & 1;
    if shift > left {
        return Ok(input.to_vec());
    }
    left -= shift;
    let mut out = Vec::with_capacity(input.len());
    let mut at = 0;
    while left > 0 && out.len() < c.limit {
        block = block.min(left);
        let source = &input[at..at + block];
        let mut dest = vec![0u8; block];
        let mut from = 0;
        for j in 0..2 {
            for i in 0..((block - j + 1) >> 1) {
                dest[block - j - 1 - i * 2] = source[from] ^ key;
                from += 1;
                key = key.wrapping_add(c.step);
            }
        }
        out.extend(dest);
        at += block;
        left -= block;
    }
    out.extend_from_slice(&input[at..]);
    Ok(out)
}
struct Bits<'a> {
    b: &'a [u8],
    bit: usize,
}
impl Bits<'_> {
    fn get(&mut self, n: usize) -> Result<u32> {
        ensure!(
            n <= 32 && self.bit + n <= self.b.len() * 8,
            "truncated PBG3 integer"
        );
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from((self.b[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(v)
    }
    fn integer(&mut self) -> Result<u32> {
        let n = self.get(2)? as usize + 1;
        self.get(n * 8)
    }
}
pub(super) fn pbg3(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 4, 10, 10)?;
    let mut b = Bits { b: &h, bit: 0 };
    let n = b.integer()? as usize;
    limits(cx, n)?;
    let at = u64::from(b.integer()?);
    let t = index(
        s,
        cx,
        at,
        length(s)?.checked_sub(at).context("PBG3 table offset")?,
    )?;
    let mut b = Bits { b: &t, bit: 0 };
    let mut specs = Vec::new();
    for _ in 0..n {
        b.integer()?;
        b.integer()?;
        let checksum = b.integer()?;
        let offset = u64::from(b.integer()?);
        let orig = u64::from(b.integer()?);
        let mut name = Vec::new();
        for _ in 0..256 {
            let ch = b.get(8)? as u8;
            if ch == 0 {
                break;
            }
            name.push(ch)
        }
        specs.push((sjis(&name)?, offset, orig, checksum));
    }
    let mut rows = Vec::new();
    for i in 0..specs.len() {
        let (ref path, offset, orig, checksum) = specs[i];
        let end = specs.get(i + 1).map_or(at, |r| r.1);
        let comp = end.checked_sub(offset).context("PBG3 entry order")?;
        rows.push(
            Row::packed(
                path.clone(),
                offset,
                comp,
                orig,
                decode(|b, e| lzss(b, e.size as usize)),
            )
            .meta(serde_json::json!({"checksum":checksum})),
        );
    }
    Ok(rows)
}
fn table_rows(
    t: &[u8],
    count: usize,
    end: u64,
    pad: bool,
    cx: &DecodeContext,
) -> Result<Vec<(String, u64, u64, u64)>> {
    limits(cx, count)?;
    let mut c = Cursor::new(t);
    let mut specs = Vec::new();
    for _ in 0..count {
        let raw = c.zero()?;
        let path = sjis(raw)?;
        if pad {
            c.take(3 - raw.len() % 4)?;
        }
        let at = u64::from(c.le32()?);
        let orig = u64::from(c.le32()?);
        c.take(4)?;
        specs.push((path, at, orig, 0));
    }
    for i in 0..specs.len() {
        specs[i].3 = specs
            .get(i + 1)
            .map_or(end, |r| r.1)
            .checked_sub(specs[i].1)
            .context("TSA entry offsets out of order")?;
    }
    Ok(specs)
}
pub(super) fn pbg4(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let n = u32_at(s, 4)? as usize;
    limits(cx, n)?;
    let at = u64::from(u32_at(s, 8)?);
    let orig = u64::from(u32_at(s, 12)?);
    ensure!(orig <= cx.limits.max_index_size, "PBG4 index byte limit");
    let table = lzss(
        &index(
            s,
            cx,
            at,
            length(s)?.checked_sub(at).context("PBG4 index offset")?,
        )?,
        orig as usize,
    )?;
    Ok(table_rows(&table, n, at, false, cx)?
        .into_iter()
        .map(|(path, at, orig, stored)| {
            Row::packed(
                path,
                at,
                stored,
                orig,
                decode(|b, e| lzss(b, e.size as usize)),
            )
        })
        .collect())
}
pub(super) fn pbgz(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = crypt(
        &bytes(s, 4, 12, 12)?,
        Cipher {
            key: 0x1b,
            step: 0x37,
            block: 12,
            limit: 0x400,
        },
    )?;
    let n = le32(&h, 0)?.checked_sub(123456).context("PBGZ count")? as usize;
    limits(cx, n)?;
    let at = u64::from(
        le32(&h, 4)?
            .checked_sub(345678)
            .context("PBGZ index offset")?,
    );
    let orig = le32(&h, 8)?
        .checked_sub(567891)
        .context("PBGZ index size")? as usize;
    ensure!(
        orig as u64 <= cx.limits.max_index_size,
        "PBGZ index byte limit"
    );
    let input = index(
        s,
        cx,
        at,
        length(s)?.checked_sub(at).context("PBGZ index range")?,
    )?;
    let table = lzss(
        &crypt(
            &input,
            Cipher {
                key: 0x3e,
                step: 0x9b,
                block: 128,
                limit: 1024,
            },
        )?,
        orig,
    )?;
    let p = profile(cx, "team-shanghai-alice/pbgz")?;
    let mut map = BTreeMap::new();
    for (tag, v) in p["decryptors"]
        .as_object()
        .context("PBGZ requires decryptors object keyed by decimal tag")?
    {
        map.insert(tag.parse::<u8>()?, Cipher::read(cx, v)?);
    }
    table_rows(&table, n, at, false, cx)?
        .into_iter()
        .map(|(path, at, orig, stored)| {
            ensure!(orig >= 4, "PBGZ encrypted payload header");
            let map = map.clone();
            Ok(Row::packed(
                path,
                at,
                stored,
                orig - 4,
                decode(move |b, e| {
                    let data = lzss(b, e.size as usize + 4)?;
                    ensure!(data.starts_with(b"edz"), "PBGZ encryption signature");
                    let cipher = *map.get(&data[3]).context("PBGZ unknown decryptor tag")?;
                    crypt(&data[4..], cipher)
                }),
            ))
        })
        .collect()
}
pub(super) fn probe_tha1(s: &dyn AssetReader) -> Result<bool> {
    if length(s)? < 16 {
        return Ok(false);
    }
    Ok(crypt(
        &bytes(s, 0, 16, 16)?,
        Cipher {
            key: 0x1b,
            step: 0x37,
            block: 16,
            limit: 1024,
        },
    )?
    .starts_with(b"THA1"))
}
pub(super) fn tha1(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = crypt(
        &bytes(s, 0, 16, 16)?,
        Cipher {
            key: 0x1b,
            step: 0x37,
            block: 16,
            limit: 1024,
        },
    )?;
    ensure!(h.starts_with(b"THA1"), "THA1 encrypted header");
    let orig = le32(&h, 4)?
        .checked_sub(123456789)
        .context("THA1 index size")? as usize;
    let comp = u64::from(
        le32(&h, 8)?
            .checked_sub(987654321)
            .context("THA1 compressed index size")?,
    );
    let n = le32(&h, 12)?.checked_sub(135792468).context("THA1 count")? as usize;
    limits(cx, n)?;
    ensure!(
        orig as u64 <= cx.limits.max_index_size,
        "THA1 index byte limit"
    );
    let at = length(s)?.checked_sub(comp).context("THA1 index offset")?;
    let table = lzss(
        &crypt(
            &index(s, cx, at, comp)?,
            Cipher {
                key: 0x3e,
                step: 0x9b,
                block: 128,
                limit: comp as usize,
            },
        )?,
        orig,
    )?;
    let p = profile(cx, "team-shanghai-alice/tha1")?;
    let ciphers = p["decryptors"]
        .as_array()
        .context("THA1 requires eight caller decryptors")?;
    ensure!(ciphers.len() == 8, "THA1 requires eight caller decryptors");
    let ciphers = ciphers
        .iter()
        .map(|v| Cipher::read(cx, v))
        .collect::<Result<Vec<_>>>()?;
    table_rows(&table, n, at, true, cx)?
        .into_iter()
        .map(|(path, at, orig, comp)| {
            let tag = path.as_bytes().iter().fold(0u8, |a, b| a.wrapping_add(*b)) as usize % 8;
            let cipher = ciphers[tag];
            Ok(Row::packed(
                path,
                at,
                comp,
                orig,
                decode(move |b, e| {
                    let b = crypt(b, cipher)?;
                    if comp == orig {
                        Ok(b)
                    } else {
                        lzss(&b, e.size as usize)
                    }
                }),
            ))
        })
        .collect()
}

pub(super) fn thbgm(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let p = cx.parameters.get("team-shanghai-alice/thbgm");
    let definitions = if let Some(path) = p.and_then(|p| p["definitions"].as_str()) {
        cx.open_dependency(path)?
    } else {
        cx.open_sibling("thbgm.fmt")?
    };
    let table = crysirror_resource::read_all(definitions.as_ref(), cx.limits.max_index_size)?;
    let mut c = Cursor::new(&table);
    let mut rows = Vec::new();
    while c.left() > 0 {
        let path = sjis(zstr(c.take(16)?))?;
        if path.is_empty() {
            break;
        }
        let at = u64::from(c.le32()?);
        c.take(4)?;
        let intro = c.le32()?;
        let size = c.le32()?;
        c.le16()?;
        let channels = c.le16()?;
        let rate = c.le32()?;
        c.take(6)?;
        let bits = c.le16()?;
        c.take(4)?;
        ensure!(intro <= size, "THBGM intro exceeds sample size");
        let spec = crysirror_resource::WaveSpec {
            format: 1,
            channels,
            sample_rate: rate,
            bits_per_sample: bits,
            extra: vec![],
            loops: vec![crysirror_resource::WaveLoop {
                start: intro,
                end: size - intro,
                play_count: 0,
            }],
            ..Default::default()
        };
        let len = spec.encoded_len(size as usize)? as u64;
        entry_size(cx, len)?;
        rows.push(Row::packed(
            ext(&path, "wavloop"),
            at,
            u64::from(size),
            len,
            decode(move |b, _| spec.encode(b)),
        ));
        limits(cx, rows.len())?;
    }
    let _ = s;
    Ok(rows)
}

pub(super) fn anm(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let mut groups: BTreeMap<String, Vec<(u64, u32, u32, u32, u32, u16, u32)>> = BTreeMap::new();
    let mut base = 0u64;
    let mut seen = BTreeSet::new();
    loop {
        ensure!(seen.insert(base), "cyclic ANM texture chain");
        limits(cx, seen.len())?;
        let h = bytes(s, base, 64, 64)?;
        let old = le32(&h, 8)? == 0;
        let (w, hgt, name, x, y, tex, has, next) = if old {
            (
                le32(&h, 12)?,
                le32(&h, 16)?,
                le32(&h, 28)?,
                u16::from_le_bytes(h[24..26].try_into().unwrap()) as u32,
                u16::from_le_bytes(h[26..28].try_into().unwrap()) as u32,
                le32(&h, 48)?,
                le32(&h, 52)? > 0,
                le32(&h, 56)?,
            )
        } else {
            (
                u16::from_le_bytes(h[10..12].try_into().unwrap()) as u32,
                u16::from_le_bytes(h[12..14].try_into().unwrap()) as u32,
                le32(&h, 16)?,
                u16::from_le_bytes(h[20..22].try_into().unwrap()) as u32,
                u16::from_le_bytes(h[22..24].try_into().unwrap()) as u32,
                le32(&h, 28)?,
                u16::from_le_bytes(h[32..34].try_into().unwrap()) > 0,
                le32(&h, 36)?,
            )
        };
        if has && w > 0 && hgt > 0 {
            let n = window(s, cx, base + u64::from(name))?;
            let mut c = Cursor::new(&n);
            let path = sjis(c.zero()?)?;
            let at = base + u64::from(tex);
            let header = bytes(s, at, 16, 16)?;
            ensure!(&header[..4] == b"THTX", "ANM corrupt texture data");
            let format = u16::from_le_bytes(header[6..8].try_into().unwrap());
            let tw = u16::from_le_bytes(header[8..10].try_into().unwrap()) as u32;
            let th = u16::from_le_bytes(header[10..12].try_into().unwrap()) as u32;
            let size = le32(&header, 12)?;
            range(length(s)?, at + 16, u64::from(size))?;
            groups
                .entry(path)
                .or_default()
                .push((at + 16, tw, th, x, y, format, size));
        }
        if next == 0 {
            break;
        }
        base = base
            .checked_add(u64::from(next))
            .context("ANM texture offset overflow")?;
    }
    let mut rows = Vec::new();
    for (path, parts) in groups {
        let w = parts
            .iter()
            .map(|p| p.3.checked_add(p.1).context("ANM width overflow"))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap();
        let h = parts
            .iter()
            .map(|p| p.4.checked_add(p.2).context("ANM height overflow"))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap();
        let size = png_size(cx, w, h)?;
        let start = parts.iter().map(|p| p.0).min().unwrap();
        let end = parts.iter().map(|p| p.0 + u64::from(p.6)).max().unwrap();
        rows.push(Row::packed(
            ext(&path, "png"),
            start,
            end - start,
            size,
            decode(move |b, _| {
                let mut rgba = vec![0; crysirror_image::rgba_len(w, h)?];
                for p in &parts {
                    let mut c = Cursor::new(
                        b.get((p.0 - start) as usize..(p.0 - start) as usize + p.6 as usize)
                            .context("ANM texture range")?,
                    );
                    for y in 0..p.2 {
                        for x in 0..p.1 {
                            let color = pixel(&mut c, p.5.try_into()?)?;
                            let at = ((y + p.4) * w + x + p.3) as usize * 4;
                            rgba[at..at + 4].copy_from_slice(&color);
                        }
                    }
                }
                png(w, h, rgba)
            }),
        ));
    }
    Ok(rows)
}
