// SPDX-License-Identifier: GPL-3.0-or-later
// WARC 1.70 layout/essential/CRC/Huffman codecs: arc_unpacker (C) 2016 rr-.
// Title-specific key/logo/image catalogues are intentionally not embedded.
use super::super::*;
use crysirror_resource::TransformSpec;
use serde_json::Value;

const ID: &str = "shiina-rio/warc";
const ESSENTIAL: &[u8] =
    b"Crypt Type 20011002 - Copyright(C) 2000 Y.Yamada/STUDIO \x82\xe6\x82\xb5\x82\xad\x82\xf1";
#[derive(Clone)]
struct Profile {
    name_size: usize,
    engine_version: Option<u32>,
    initial_key_id: Option<String>,
    logo_key_id: Option<String>,
    region_path: Option<String>,
    crc_key_id: Option<String>,
    table: Vec<TransformSpec>,
    before: Vec<TransformSpec>,
    after: Vec<TransformSpec>,
}
impl Profile {
    fn parse(cx: &DecodeContext) -> Result<Self> {
        let p = cx.parameters.get(ID).and_then(Value::as_object).context(
            "WARC needs an explicit native profile (entry_name_size and client key aliases)",
        )?;
        for key in p.keys() {
            ensure!(
                [
                    "entry_name_size",
                    "engine_version",
                    "initial_key_id",
                    "logo_key_id",
                    "region_path",
                    "crc_key_id",
                    "table_transforms",
                    "before",
                    "after"
                ]
                .contains(&key.as_str()),
                "unknown WARC profile option {key}"
            );
        }
        let number = p
            .get("entry_name_size")
            .and_then(Value::as_u64)
            .context("WARC entry_name_size")?;
        ensure!((1..=1024).contains(&number), "WARC native name field size");
        let alias = |key: &str| -> Result<Option<String>> {
            p.get(key)
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .context("WARC alias/path must be a string")
                })
                .transpose()
        };
        let specs = |key: &str| -> Result<Vec<TransformSpec>> {
            let list = p.get(key).cloned().unwrap_or_else(|| serde_json::json!([]));
            let list: Vec<TransformSpec> = serde_json::from_value(list)?;
            ensure!(list.len() <= 16, "WARC transform chain limit");
            Ok(list)
        };
        let engine_version = p
            .get("engine_version")
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .context("WARC engine_version")
            })
            .transpose()?;
        Ok(Self {
            name_size: number as usize,
            engine_version,
            initial_key_id: alias("initial_key_id")?,
            logo_key_id: alias("logo_key_id")?,
            region_path: alias("region_path")?,
            crc_key_id: alias("crc_key_id")?,
            table: specs("table_transforms")?,
            before: specs("before")?,
            after: specs("after")?,
        })
    }
    fn table_size(&self) -> usize {
        (self.name_size + 24) * 16384
    }
    fn key(&self, cx: &DecodeContext, alias: Option<&str>, label: &str) -> Result<Vec<u8>> {
        let alias = alias.with_context(|| format!("WARC profile requires {label}"))?;
        Ok(cx.keys.key(cx.client, alias)?.with_bytes(|b| b.to_vec()))
    }
}
#[derive(Clone, Copy)]
struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x5d588b65).wrapping_add(1);
        self.0
    }
}
fn sine_integral(a: f64) -> Result<f64> {
    if a < 0. {
        return Ok(-sine_integral(-a)?);
    }
    if a < 18. {
        let (mut result, mut term) = (a, a);
        let squared = -(a * a);
        for j in (3..1000).step_by(2) {
            term *= squared / f64::from(j * (j - 1));
            result += term / f64::from(j);
            if result == squared {
                break;
            }
        }
        return Ok(result);
    }
    let (mut flags, mut low0, mut high0, mut low1, mut high1, mut x0, mut x1, mut div, mut i) =
        (0u8, 0., 2., 0., 2., 0., 0., 1. / a, 0u32);
    for _ in 0..100000 {
        x0 += div;
        i += 1;
        div *= f64::from(i) / a;
        if x0 < high0 {
            high0 = x0
        } else {
            flags |= 1;
        }
        x1 += div;
        i += 1;
        div *= f64::from(i) / a;
        if x1 < high1 {
            high1 = x1
        } else {
            flags |= 2;
        }
        x0 -= div;
        i += 1;
        div *= f64::from(i) / a;
        if x0 > low0 {
            low0 = x0
        } else {
            flags |= 4;
        }
        x1 -= div;
        i += 1;
        div *= f64::from(i) / a;
        if x1 > low1 {
            low1 = x1
        } else {
            flags |= 8;
        }
        if flags == 15 {
            return Ok((std::f64::consts::PI
                - a.cos() * (low0 + high0)
                - a.sin() * (low1 + high1))
                / 2.);
        }
    }
    anyhow::bail!("WARC sine integral convergence limit")
}
fn key_addend1(value: u64) -> u64 {
    let bytes = (value as u32).to_le_bytes();
    let f = bytes.map(|v| (1.5 * f64::from(v) + 0.1) as f32);
    let bits = f.map(f32::to_bits);
    u64::from(
        bits[0].swap_bytes().wrapping_add(f[1] as u32)
            | (bits[2] as i32)
                .wrapping_neg()
                .cast_unsigned()
                .wrapping_sub(!bits[3]),
    )
}
fn key_addend2(a: f64, lcg: &mut Lcg) -> Result<u64> {
    const SCALE: f64 = 4294967296.;
    if a > 1. {
        let base = (a * 2. - 1.).sqrt();
        for _ in 0..1000000 {
            let u = 1. - f64::from(lcg.next()) / SCALE;
            let mut v = f64::from(lcg.next()) * 2. / SCALE - 1.;
            if u * u + v * v > 1. {
                continue;
            }
            v /= u;
            let ret = v * base + a - 1.;
            if ret <= 0. {
                continue;
            }
            let likelihood = (a - 1.) * (ret / (a - 1.)).ln() - v * base;
            if likelihood < -50. {
                continue;
            }
            if f64::from(lcg.next()) / SCALE <= likelihood.exp() * (v * v + 1.) {
                return Ok((ret * 256.) as u64);
            }
        }
    } else {
        let threshold = 1f64.exp() / (a + 1f64.exp());
        for _ in 0..1000000 {
            let first = f64::from(lcg.next()) / SCALE;
            let next = f64::from(lcg.next()) / SCALE;
            let (ret, likelihood) = if first < threshold {
                let ret = next.powf(1. / a);
                (ret, (-ret).exp())
            } else {
                let ret = 1. - next.ln();
                (ret, ret.powf(a - 1.))
            };
            if f64::from(lcg.next()) / SCALE < likelihood {
                return Ok((ret * 256.) as u64);
            }
        }
    }
    anyhow::bail!("WARC random key convergence limit")
}
fn crc32_push(state: &mut u32, byte: u8) {
    *state ^= u32::from(byte);
    for _ in 0..8 {
        *state = (*state >> 1) ^ if *state & 1 != 0 { 0xedb88320 } else { 0 };
    }
}
fn initial_keys(p: &Profile, cx: &DecodeContext, data: &[u8]) -> Result<[u32; 10]> {
    ensure!(data.len() >= 108, "WARC initial cipher header");
    let base = p.key(
        cx,
        p.initial_key_id.as_deref(),
        "initial_key_id (five LE words)",
    )?;
    ensure!(base.len() == 20, "WARC base key must contain five words");
    let keys = std::array::from_fn::<_, 5, _>(|i| {
        u32::from_le_bytes(base[i * 4..i * 4 + 4].try_into().unwrap())
    });
    let mut words = [0u32; 80];
    for i in 0..16 {
        words[i] = u32::from_be_bytes(data[44 + i * 4..48 + i * 4].try_into().unwrap());
    }
    for i in 16..80 {
        words[i] = (words[i - 16] ^ words[i - 14] ^ words[i - 8] ^ words[i - 3]).rotate_left(1);
    }
    let mut add = keys;
    for i in 0..80 {
        let f = match i / 16 {
            0 => add[1] ^ add[2] ^ add[3],
            1 => (add[1] & add[2]) | (add[3] & !add[1]),
            2 => add[3] ^ (add[1] | !add[2]),
            3 => (add[1] & add[3]) | (add[2] & !add[3]),
            _ => add[1] ^ (add[2] | !add[3]),
        };
        let v = words[i]
            .wrapping_add(add[4])
            .wrapping_add(f)
            .wrapping_add([0, 0x5a827999, 0x6ed9eba1, 0x8f1bbcdc, 0xa953fd4e][i / 16])
            .wrapping_add(add[0].rotate_left(5));
        add = [v, add[0], add[1].rotate_right(2), add[2], add[3]];
    }
    let mut out = [0u32; 10];
    for i in 0..5 {
        out[i] = keys[i].wrapping_add(add[i]);
    }
    out[9] = ((out[2] as i32 as i64 as u64).wrapping_mul(out[3] as i32 as i64 as u64) >> 8) as u32;
    let mut flags = words[0].swap_bytes() | 0x80000000;
    if flags & 0x78000000 == 0 {
        flags |= 0x98000000;
    }
    let source = cx.open_dependency(
        p.region_path
            .as_deref()
            .context("WARC initial cipher needs region_path")?,
    )?;
    let data = crysirror_resource::read_all(source.as_ref(), cx.limits.max_entry_size)?;
    let (width, height) = image::io::Reader::new(std::io::Cursor::new(&data))
        .with_guessed_format()?
        .into_dimensions()?;
    let rgba_bytes = crysirror_image::rgba_len(width, height)? as u64;
    ensure!(
        rgba_bytes <= cx.limits.max_entry_size,
        "WARC region decoded image byte limit"
    );
    out[6] = cx.with_temporary_bytes(rgba_bytes, || {
        // Region pixels remain inside this temporary reservation. Feed the
        // transformed native BGR bytes to CRC directly instead of allocating
        // a second full-size RGB image.
        let image = crysirror_image::decode_standard(&data)?;
        ensure!(
            image.width == width && image.height == height,
            "WARC region dimensions changed during decode"
        );
        let src_alpha = if flags & 0x08000000 != 0 {
            flags & 511
        } else {
            256
        };
        let dst_alpha = if flags & 0x10000000 != 0 {
            (flags >> 12) & 511
        } else {
            0
        };
        let base_color = words[1] >> 8;
        let mix = |a: u32, b: u32, c: u32| a.wrapping_add(b.wrapping_sub(a).wrapping_mul(c) >> 8);
        let mut crc = !0u32;
        for y in 0..image.height {
            for x in 0..image.width {
                let sx = if flags & 0x40000000 != 0 {
                    image.width - 1 - x
                } else {
                    x
                };
                let sy = if flags & 0x20000000 != 0 {
                    image.height - 1 - y
                } else {
                    y
                };
                let source = &image.rgba[((sy * image.width + sx) * 4) as usize..][..4];
                let alpha = mix(0, u32::from(source[3]), src_alpha);
                for channel in 0..3 {
                    let byte = mix(
                        0,
                        mix(
                            u32::from(source[2 - channel]),
                            (base_color >> (channel * 8)) & 255,
                            dst_alpha,
                        ) & 255,
                        alpha,
                    ) as u8;
                    crc32_push(&mut crc, byte);
                }
            }
        }
        Ok(!crc)
    })?;
    if p.engine_version
        .context("WARC initial cipher needs engine_version")?
        >= 2390
    {
        out[6] = out[6].wrapping_add(out[9]);
    }
    let mut n = (((u64::from(out[0]) & 0x7fffffff) << 32) | u64::from(out[1])) / 10000;
    let ms = n % 1000;
    n /= 1000;
    let second = n % 60;
    n /= 60;
    let minute = n % 60;
    n /= 60;
    let hour = n % 24;
    n /= 24;
    let y400 = n / 146097;
    n -= y400 * 146097;
    let y100 = (n / 36524).min(3);
    n -= y100 * 36524;
    let y4 = n / 1461;
    n -= y4 * 1461;
    let y1 = (n / 365).min(3);
    n -= y1 * 365;
    let year = 1601 + y400 * 400 + y100 * 100 + y4 * 4 + y1;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let table = if leap {
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366]
    } else {
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365]
    };
    let month = table
        .iter()
        .position(|day| n < *day)
        .context("WARC date conversion")?;
    out[5] = year as u32 | ((month as u32) << 16);
    out[7] = hour as u32 | ((minute as u32) << 16);
    out[8] = second as u32 | ((ms as u32) << 16);
    Ok(out)
}
fn essential(p: &Profile, cx: &DecodeContext, data: &mut [u8]) -> Result<()> {
    if data.len() < 3 {
        return Ok(());
    }
    let mut lcg = Lcg(data.len() as u32);
    let a = (data[0] ^ (data.len() as u8)) as i8 as i32;
    let b = (data[1] ^ ((data.len() / 2) as u8)) as i8 as i32;
    let mut start = 0usize;
    let end = data.len().min(1024);
    let mut first = 0u64;
    if data.len() != p.table_size() {
        let logo = p.key(cx, p.logo_key_id.as_deref(), "logo_key_id")?;
        ensure!(!logo.is_empty(), "WARC logo key cannot be empty");
        let current = lcg.next();
        let index = (f64::from(current) * (logo.len() as f64 / 4294967296.)) as usize;
        first = key_addend1(u64::from(current.wrapping_add(u32::from(logo[index])))) & 0xfffffff;
        if end > 128 {
            let keys = initial_keys(p, cx, data)?;
            for (i, key) in keys.iter().enumerate() {
                let at = 4 + i * 4;
                let word = le32(data, at)? ^ key;
                data[at..at + 4].copy_from_slice(&word.to_le_bytes());
            }
            start = 128;
        }
    }
    let integral = sine_integral(f64::from(a))? * 100000000.;
    // Define truncation followed by modulo 2^32 for negative values. Upstream
    // casts negative doubles directly to u32 (undefined in portable C++).
    lcg.0 ^= (integral as i64) as u32;
    let mut token = if a != 0 || b != 0 {
        (f64::from(a) / f64::from(a * a + b * b).sqrt()).acos() / std::f64::consts::PI * 180.
    } else {
        0.
    };
    if b < 0 {
        token = 360. - token;
    }
    let mut key1 = ((first + key_addend2(token, &mut lcg)?) % ESSENTIAL.len() as u64) as usize;
    let mut key2 = 0usize;
    // The native positive-stride range is empty when the initial cipher
    // consumes128 bytes and only a single byte remains (payload length129).
    if start + 2 < end {
        for item in &mut data[start + 2..end] {
            let value = (*item ^ ((f64::from(lcg.next()) / 16777216.) as u8)).rotate_right(1)
                ^ ESSENTIAL[key1]
                ^ ESSENTIAL[key2];
            *item = value;
            key1 = usize::from(value) % ESSENTIAL.len();
            key2 = (key2 + 1) % ESSENTIAL.len();
        }
    }
    Ok(())
}
fn crc_crypt(data: &mut [u8], table: &[u8]) -> Result<()> {
    if data.len() < 1024 {
        return Ok(());
    }
    ensure!(
        table.len() >= 4 && table.len().is_multiple_of(4),
        "WARC CRC key table alignment"
    );
    let mut crc = !0u32;
    for b in &data[..256] {
        crc ^= u32::from(*b) << 24;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x80000000 != 0 { 0x04c11db7 } else { 0 };
        }
    }
    for i in 0..64 {
        let idx = le32(data, 256 + i * 4)? as usize % table.len();
        let key = le32(table, idx / 4 * 4)? ^ crc;
        let at = 512 + i * 4;
        let word = le32(data, at)? ^ key;
        data[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    Ok(())
}
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    buffer: u32,
    left: usize,
}
impl Bits<'_> {
    fn get(&mut self, count: usize) -> Result<u32> {
        let mut value = 0;
        for _ in 0..count {
            if self.left == 0 {
                ensure!(self.at < self.data.len(), "WARC Huffman bitstream ended");
                let n = (self.data.len() - self.at).min(4);
                if n == 4 {
                    self.buffer = le32(self.data, self.at)?;
                } else {
                    for b in &self.data[self.at..self.at + n] {
                        self.buffer = (self.buffer << 8) | u32::from(*b);
                    }
                }
                self.at += n;
                self.left = 32;
            }
            self.left -= 1;
            value = (value << 1) | ((self.buffer >> self.left) & 1);
        }
        Ok(value)
    }
}
fn yh1(input: &[u8], size: usize) -> Result<Vec<u8>> {
    fn tree(bits: &mut Bits<'_>, nodes: &mut Vec<[u16; 2]>, depth: usize) -> Result<u16> {
        ensure!(depth < 256, "WARC Huffman depth limit");
        if bits.get(1)? == 0 {
            return Ok(bits.get(8)? as u16);
        }
        let id = nodes.len() + 256;
        ensure!(id < 512, "WARC Huffman node limit");
        nodes.push([0; 2]);
        let a = tree(bits, nodes, depth + 1)?;
        let b = tree(bits, nodes, depth + 1)?;
        nodes[id - 256] = [a, b];
        Ok(id as u16)
    }
    let mut bits = Bits {
        data: input,
        at: 0,
        buffer: 0,
        left: 0,
    };
    let mut nodes = vec![];
    let root = tree(&mut bits, &mut nodes, 0)?;
    let mut out = Vec::with_capacity(size);
    for _ in 0..size {
        let mut v = root;
        while v >= 256 {
            v = nodes[usize::from(v) - 256][bits.get(1)? as usize];
        }
        out.push(v as u8);
    }
    Ok(out)
}
fn extra(
    cx: &DecodeContext,
    specs: &[TransformSpec],
    bytes: Vec<u8>,
    metadata: &Value,
) -> Result<Vec<u8>> {
    if specs.is_empty() {
        return Ok(bytes);
    }
    let mut reader: Arc<dyn AssetReader> =
        Arc::new(crysirror_core::service::MemoryAssetReader(Arc::from(bytes)));
    for spec in specs {
        reader = cx.apply(spec, reader, metadata)?;
    }
    crysirror_resource::read_all(reader.as_ref(), cx.limits.max_entry_size)
}
pub(super) fn open(source: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(source, 0, 12, 12)?;
    ensure!(
        &h[..8] == b"WARC 1.7",
        "unsupported WARC version (pinned decoder supports 1.70)"
    );
    let p = Profile::parse(cx)?;
    ensure!(
        p.table_size() as u64 <= cx.limits.max_index_size,
        "WARC padded index budget"
    );
    let offset = u64::from(le32(&h, 8)? ^ 0xf182ad82);
    let raw = index(
        source,
        cx,
        offset,
        length(source)?
            .checked_sub(offset)
            .context("WARC table offset")?,
    )?;
    let table = if !p.table.is_empty() {
        // Index transforms must admit their output under the index quota,
        // before a decompressor can allocate an entry-sized result.
        let mut index_context = cx.clone();
        index_context.limits.max_entry_size =
            cx.limits.max_entry_size.min(cx.limits.max_index_size);
        extra(
            &index_context,
            &p.table,
            raw,
            &serde_json::json!({"format":ID,"stage":"index","offset":offset}),
        )?
    } else {
        let mut data = raw;
        data.resize(p.table_size(), 0);
        essential(&p, cx, &mut data)?;
        for word in data.chunks_exact_mut(4) {
            let value = u32::from_le_bytes(word.try_into().unwrap()) ^ offset as u32;
            word.copy_from_slice(&value.to_le_bytes());
        }
        for b in &mut data[8..] {
            *b ^= 0x55;
        }
        inflate_index(&data[8..], cx.limits.max_index_size)?
    };
    ensure!(
        table.len() as u64 <= cx.limits.max_index_size,
        "WARC decoded index byte limit"
    );
    ensure!(
        table.len() % (p.name_size + 24) == 0,
        "WARC table record alignment"
    );
    let mut c = Cursor::new(&table);
    let mut rows = vec![];
    while c.left() > 0 {
        let native = zstr(c.take(p.name_size)?);
        let name: String = native
            .iter()
            .map(|b| if *b >= 128 { '_' } else { char::from(*b) })
            .collect();
        let offset = u64::from(c.le32()?);
        let stored = u64::from(c.le32()?);
        let declared = c.le32()?;
        let time = c.le64()?;
        let flags = c.le32()?;
        if name.is_empty() {
            continue;
        }
        limits(cx, rows.len() + 1)?;
        ensure!(stored >= 8, "WARC entry header size");
        let h = bytes(source, offset, 8, 8)?;
        let header_size = le32(&h, 4)?;
        let magic = le32(&h, 0)? ^ 0x82ad82 ^ header_size;
        let marker = [magic as u8, (magic >> 8) as u8, (magic >> 16) as u8];
        let packed = matches!(&marker, b"YH1" | b"YPK" | b"YLZ");
        let size = if packed {
            u64::from(header_size)
        } else {
            stored - 8
        };
        ensure!(size <= cx.limits.max_entry_size, "WARC decoded entry limit");
        let profile = p.clone();
        let context = cx.clone();
        let metadata = serde_json::json!({"format":ID,"flags":flags,"timestamp":time,"index_size":declared,"header_size":header_size,"compression":String::from_utf8_lossy(&marker)});
        let meta = metadata.clone();
        let encrypted = h[3] > 0;
        let plan = decode(move |input, _| {
            let mut data = input.get(8..).context("WARC payload header")?.to_vec();
            if flags & 0x80000000 != 0 {
                essential(&profile, &context, &mut data)?;
            }
            data = extra(&context, &profile.before, data, &meta)?;
            if flags & 0x20000000 != 0 {
                let key = profile.key(&context, profile.crc_key_id.as_deref(), "crc_key_id")?;
                crc_crypt(&mut data, &key)?;
            }
            if packed {
                ensure!(
                    &marker != b"YLZ",
                    "WARC YLZ decompression is unimplemented in the pinned decoder"
                );
                if encrypted {
                    let key = if &marker == b"YH1" {
                        0x6393528eu32 ^ 0x4b4d
                    } else {
                        (0x4b4du32 | (0x4b4d << 16)) ^ !0
                    };
                    for word in data.chunks_exact_mut(4) {
                        let value = u32::from_le_bytes(word.try_into().unwrap()) ^ key;
                        word.copy_from_slice(&value.to_le_bytes());
                    }
                    if &marker == b"YPK" {
                        let tail = data.len() / 4 * 4;
                        for b in &mut data[tail..] {
                            *b ^= key as u8;
                        }
                    }
                }
                data = if &marker == b"YH1" {
                    yh1(&data, header_size as usize)?
                } else {
                    let out = inflate_index(&data, u64::from(header_size))?;
                    ensure!(out.len() == header_size as usize, "WARC zlib output size");
                    out
                };
                if flags & 0x40000000 != 0 {
                    let key = profile.key(&context, profile.crc_key_id.as_deref(), "crc_key_id")?;
                    crc_crypt(&mut data, &key)?;
                }
                data = extra(&context, &profile.after, data, &meta)?;
            }
            Ok(data)
        });
        rows.push(Row::packed(name, offset, stored, size, plan).meta(metadata));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_essential_table_cipher_matches_pinned_cpp() {
        // Oracle: unchanged decrypt_essential/LCG/math functions compiled from
        // arc_unpacker 456834ecf2e5686813802c37efd829310485c57d, C++14 -O2.
        // Table-sized buffers deliberately exclude entry/title-key branches.
        let mut cx = DecodeContext::new(crysirror_core::state::ClientId(1));
        cx.parameters
            .insert(ID.into(), serde_json::json!({"entry_name_size":32}));
        let profile = Profile::parse(&cx).unwrap();
        for (a, b, expected) in [
            (0u8, 0u8, 0x385e260ba3ed9535u64),
            (31, 3, 0xb829a59d60a16612),
            (17, 7, 0x27459897321a8a65),
        ] {
            let mut bytes = vec![0; profile.table_size()];
            bytes[0] = a;
            bytes[1] = b;
            essential(&profile, &cx, &mut bytes).unwrap();
            let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
            });
            assert_eq!(hash, expected, "essential table case {a},{b}");
        }
    }
    #[test]
    fn native_initial_cipher_with_one_remaining_byte_is_an_empty_range() {
        use crysirror_core::service::MemoryAssetReader;
        let mut cx = DecodeContext::new(crysirror_core::state::ClientId(11));
        let keys = crysirror_resource::MemoryKeyProvider::default();
        keys.insert(cx.client, "initial-fixture", vec![0; 20])
            .unwrap();
        keys.insert(cx.client, "logo-fixture", vec![0]).unwrap();
        cx.keys = Arc::new(keys);
        cx.parameters.insert(ID.into(),serde_json::json!({"entry_name_size":32,"engine_version":2390,"initial_key_id":"initial-fixture","logo_key_id":"logo-fixture","region_path":"region.png"}));
        let region = crysirror_image::encode_png_uncompressed(&crysirror_image::ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        })
        .unwrap();
        cx.resolver = Some(Arc::new(
            move |path: &str| -> Result<Arc<dyn AssetReader>> {
                ensure!(path == "region.png", "unexpected region path");
                Ok(Arc::new(MemoryAssetReader(Arc::from(region.clone()))))
            },
        ));
        let profile = Profile::parse(&cx).unwrap();
        let mut data = vec![0; 129];
        data[0] = 129;
        data[1] = 64;
        essential(&profile, &cx, &mut data).unwrap();
        assert_eq!(
            data[128], 0,
            "native130..129 positive range must remain empty"
        );
        assert_eq!(cx.resident_bytes(), 0);
    }
    #[test]
    fn custom_bitstream_retains_prior_word_in_short_final_fetch() {
        // Native CustomBitStream::fetch does not clear the previous buffer
        // when the final fetch contains fewer than four bytes.
        for (tail, expected) in [
            (vec![0x55], 0x33221155),
            (vec![0x55, 0x66], 0x22115566),
            (vec![0x55, 0x66, 0x77], 0x11556677),
        ] {
            let mut data = vec![0x11, 0x22, 0x33, 0x44];
            data.extend(tail);
            let mut bits = Bits {
                data: &data,
                at: 0,
                buffer: 0,
                left: 0,
            };
            assert_eq!(bits.get(32).unwrap(), 0x44332211);
            assert_eq!(bits.get(32).unwrap(), expected);
            assert!(
                bits.get(1).is_err(),
                "a later fetch must reject truly exhausted input"
            );
        }
    }
    #[test]
    fn compressed_index_pipeline_uses_index_limit_before_output_allocation() {
        use crysirror_core::service::MemoryAssetReader;
        use std::io::Write;
        let index_limit = 25 * 16384u64;
        let oversized = vec![0; index_limit as usize + 25];
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        zlib.write_all(&oversized).unwrap();
        let compressed = zlib.finish().unwrap();
        let mut input = b"WARC 1.7".to_vec();
        input.extend((12u32 ^ 0xf182ad82).to_le_bytes());
        input.extend(compressed);
        for declared in [false, true] {
            let mut cx = DecodeContext::new(crysirror_core::state::ClientId(12));
            cx.limits.max_index_size = index_limit;
            cx.limits.max_entry_size = index_limit * 4;
            cx.limits.max_resident_bytes = index_limit * 8;
            let mut transform = serde_json::json!({"algorithm":"zlib"});
            if declared {
                transform["expected_size"] = serde_json::json!(oversized.len());
            }
            cx.parameters.insert(
                ID.into(),
                serde_json::json!({"entry_name_size":1,"table_transforms":[transform]}),
            );
            let error = open(&MemoryAssetReader(Arc::from(input.clone())), &cx)
                .err()
                .unwrap();
            let expected = if declared {
                "decoded entry size limit"
            } else {
                "decompressed entry exceeds declared size or limit"
            };
            assert!(error.to_string().contains(expected), "{error:#}");
            assert_eq!(
                cx.resident_bytes(),
                0,
                "failed index transform must release its reservation"
            );
        }
    }
    #[test]
    fn region_image_checks_decoded_and_resident_quotas_before_decode() {
        use crysirror_core::service::MemoryAssetReader;
        let mut cx = DecodeContext::new(crysirror_core::state::ClientId(13));
        let keys = crysirror_resource::MemoryKeyProvider::default();
        keys.insert(cx.client, "initial-fixture", vec![0; 20])
            .unwrap();
        keys.insert(cx.client, "logo-fixture", vec![0]).unwrap();
        cx.keys = Arc::new(keys);
        cx.parameters.insert(ID.into(),serde_json::json!({"entry_name_size":32,"engine_version":2390,"initial_key_id":"initial-fixture","logo_key_id":"logo-fixture","region_path":"region.png"}));
        let rgba_bytes = 64 * 64 * 4u64;
        let region = crysirror_image::encode_png(&crysirror_image::ImageSurface {
            width: 64,
            height: 64,
            rgba: vec![0; rgba_bytes as usize],
        })
        .unwrap();
        assert!(
            region.len() < 1024,
            "fixture must fit the encoded-entry quota"
        );
        cx.resolver = Some(Arc::new(
            move |path: &str| -> Result<Arc<dyn AssetReader>> {
                ensure!(path == "region.png", "unexpected region path");
                Ok(Arc::new(MemoryAssetReader(Arc::from(region.clone()))))
            },
        ));
        let profile = Profile::parse(&cx).unwrap();
        let mut data = vec![0; 129];
        data[0] = 129;
        data[1] = 64;
        cx.limits.max_entry_size = 1024;
        let error = essential(&profile, &cx, &mut data.clone()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("region decoded image byte limit"),
            "{error:#}"
        );
        assert_eq!(cx.resident_bytes(), 0);
        cx.limits.max_entry_size = rgba_bytes;
        cx.limits.max_resident_bytes = rgba_bytes + 127;
        let retained = cx.memory(vec![0; 128]).unwrap();
        let error = essential(&profile, &cx, &mut data).unwrap_err();
        assert!(error.to_string().contains("resident"), "{error:#}");
        assert_eq!(
            cx.resident_bytes(),
            128,
            "failed temporary admission must preserve only the existing resource"
        );
        drop(retained);
        assert_eq!(cx.resident_bytes(), 0);
    }
}
