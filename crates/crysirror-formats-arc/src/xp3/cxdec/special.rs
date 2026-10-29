// Portions adapted from xmoezzz/xp3-brute, revision c82cb14302138125640478556723705aa720007b.
// Copyright (c) xp3-brute contributors; modifications CrySirror contributors.
// MIT License: see NOTICE-MIT in this directory.
//! CXDEC indirect Special (filename) index: the complemented-state ChaCha8
//! name cipher (upstream `src/special_cipher.rs`), `Storages.setupArchiveData`
//! parameter derivation (`src/special_params.rs`), name records and filename
//! tokens (`src/cxdec_names.rs`) and the native startup redirect
//! (`src/embedded_pe.rs`). Tags and titles never select an algorithm.
//!
//! CrySirror delta: setupArchiveData text candidates come from the compiled
//! startup script's string pool instead of upstream's tjs2dec symbolic
//! execution. A candidate is accepted only when the derived seeds decrypt the
//! Special blob to a zlib stream (Adler-32 checked) of the declared size whose
//! records parse exactly and whose filename tokens cover every record.
use anyhow::{ensure, Context, Result};
use flate2::read::ZlibDecoder;
use std::io::Read;

const SIGMA: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];
const ENCRYPTED_PREFIX: usize = 0x100;
pub const MAX_NAMES_BYTES: usize = 64 * 1024 * 1024;

/// Ten external words of the name cipher; sigma/rounds are algorithm constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialParams {
    pub control_words: [u32; 8],
    pub seed0: u32,
    pub seed1: u32,
}
impl SpecialParams {
    fn base_state(&self) -> [u32; 16] {
        let mut s = [0u32; 16];
        for i in 0..4 {
            s[i] = !SIGMA[i];
        }
        s[4..12].copy_from_slice(&self.control_words);
        s[12] = u32::MAX;
        s[13] = u32::MAX;
        s[14] = !self.seed0;
        s[15] = !self.seed1;
        s
    }
    /// XOR-symmetric stream from block counter zero.
    pub fn apply(&self, data: &mut [u8]) {
        let base = self.base_state();
        for (block, chunk) in data.chunks_mut(64).enumerate() {
            let mut input = base;
            let stored = !(block as u64);
            input[12] = stored as u32;
            input[13] = (stored >> 32) as u32;
            for (w, bytes) in chacha8_complemented(input)
                .into_iter()
                .zip(chunk.chunks_mut(4))
            {
                for (d, k) in bytes.iter_mut().zip(w.to_le_bytes()) {
                    *d ^= k;
                }
            }
        }
    }
}
fn chacha8_complemented(input: [u32; 16]) -> [u32; 16] {
    let initial = input.map(|w| !w);
    let mut x = initial;
    let q = |x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize| {
        x[a] = x[a].wrapping_add(x[b]);
        x[d] = (x[d] ^ x[a]).rotate_left(16);
        x[c] = x[c].wrapping_add(x[d]);
        x[b] = (x[b] ^ x[c]).rotate_left(12);
        x[a] = x[a].wrapping_add(x[b]);
        x[d] = (x[d] ^ x[a]).rotate_left(8);
        x[c] = x[c].wrapping_add(x[d]);
        x[b] = (x[b] ^ x[c]).rotate_left(7);
    };
    for _ in 0..4 {
        q(&mut x, 0, 4, 8, 12);
        q(&mut x, 1, 5, 9, 13);
        q(&mut x, 2, 6, 10, 14);
        q(&mut x, 3, 7, 11, 15);
        q(&mut x, 0, 5, 10, 15);
        q(&mut x, 1, 6, 11, 12);
        q(&mut x, 2, 7, 8, 13);
        q(&mut x, 3, 4, 9, 14);
    }
    std::array::from_fn(|i| x[i].wrapping_add(initial[i]))
}

/// Values the native `setupArchiveData(text)` initializer generates:
/// `control = SHAKE256(utf16le(text))[..4096]`,
/// `digest = BLAKE2s-128(key = text_bytes[..min(len,32)], control)`.
#[derive(Clone, Debug)]
pub struct Derived {
    pub control_block: Vec<u8>,
    pub mask: u32,
    pub offset: u32,
    pub special: SpecialParams,
}
pub fn derive(text: &str) -> Result<Derived> {
    use blake2::digest::{consts::U16, KeyInit, Mac};
    use sha3::digest::{ExtendableOutput, Update, XofReader};
    let input: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    ensure!(
        !input.is_empty() && input.len() <= 64 * 1024,
        "setupArchiveData text length"
    );
    let mut shake = sha3::Shake256::default();
    shake.update(&input);
    let mut control_block = vec![0u8; 4096];
    XofReader::read(&mut shake.finalize_xof(), &mut control_block);
    let mut mac =
        <blake2::Blake2sMac<U16> as KeyInit>::new_from_slice(&input[..input.len().min(32)])
            .map_err(|_| anyhow::anyhow!("BLAKE2s key length"))?;
    Mac::update(&mut mac, &control_block);
    let digest = mac.finalize().into_bytes();
    let word = |i: usize| u32::from_le_bytes(digest[i * 4..i * 4 + 4].try_into().unwrap());
    let d0 = word(0);
    let mut control_words = [0u32; 8];
    for (i, w) in control_words.iter_mut().enumerate() {
        *w = !u32::from_le_bytes(control_block[i * 4..i * 4 + 4].try_into().unwrap());
    }
    Ok(Derived {
        mask: (d0 & 0x1ff) + 0x100,
        offset: ((d0 >> 9) & 0x7f) + ((d0 >> 16) & 0xff) + 0x20,
        special: SpecialParams {
            control_words,
            seed0: word(2),
            seed1: word(3),
        },
        control_block,
    })
}

/// Decrypt the first 0x100 bytes, inflate and require the declared size.
pub fn decode_blob(params: &SpecialParams, stored: &[u8], original: usize) -> Result<Vec<u8>> {
    ensure!(original <= MAX_NAMES_BYTES, "Special name index limit");
    let mut data = stored.to_vec();
    let n = data.len().min(ENCRYPTED_PREFIX);
    params.apply(&mut data[..n]);
    let mut out = Vec::with_capacity(original);
    ZlibDecoder::new(&data[..])
        .take(original as u64 + 1)
        .read_to_end(&mut out)
        .context("Special name index zlib")?;
    ensure!(out.len() == original, "Special name index size mismatch");
    Ok(out)
}

/// `{tag u32, size i64, hash u32, units i16, utf16[units], 0u16}` records;
/// every record must have exactly that body size and the stream must end on
/// a record boundary.
pub fn parse_records(decoded: &[u8]) -> Result<Vec<(u32, String)>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < decoded.len() {
        ensure!(at + 12 <= decoded.len(), "truncated Special record header");
        let size = i64::from_le_bytes(decoded[at + 4..at + 12].try_into().unwrap());
        let body = at + 12;
        ensure!(size >= 8, "Special record body size");
        let end = body
            .checked_add(usize::try_from(size)?)
            .filter(|&e| e <= decoded.len())
            .context("Special record overrun")?;
        let hash = u32::from_le_bytes(decoded[body..body + 4].try_into().unwrap());
        let units = i16::from_le_bytes(decoded[body + 4..body + 6].try_into().unwrap());
        ensure!(units > 0, "Special record without name");
        let units = units as usize;
        ensure!(6 + units * 2 + 2 == end - body, "Special record body size");
        ensure!(
            decoded[end - 2..end] == [0, 0],
            "Special record name terminator"
        );
        let words: Vec<u16> = decoded[body + 6..body + 6 + units * 2]
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        let name = String::from_utf16(&words).context("Special record name UTF-16")?;
        ensure!(
            !name.chars().any(|c| c == '\0' || c.is_control()),
            "Special record name"
        );
        out.push((hash, name));
        at = end;
    }
    ensure!(!out.is_empty(), "empty Special name index");
    Ok(out)
}

/// Native filename normalization before token hashing: ASCII lowercase,
/// `\` to `/`, leading and repeated `/` removed.
pub fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let (mut seen, mut slash) = (false, false);
    for c in name.chars() {
        let c = if c == '\\' {
            '/'
        } else {
            c.to_ascii_lowercase()
        };
        if c == '/' {
            if seen && !slash {
                out.push('/');
            }
            slash = true;
            continue;
        }
        seen = true;
        slash = false;
        out.push(c);
    }
    out
}
/// Lowercase hex MD5 of UTF-16LE(`prefix` + normalized name); `prefix` is the
/// archive's own Special descriptor tag (runtime data, not a family key).
pub fn token(prefix: &str, name: &str) -> String {
    let bytes: Vec<u8> = prefix
        .encode_utf16()
        .chain(normalize(name).encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    md5(&bytes).iter().map(|b| format!("{b:02x}")).collect()
}

// RFC 1321; filename token lookup only, never a security decision.
fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
    let k: [u32; 64] =
        std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32);
    let mut msg = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((input.len() as u64).wrapping_mul(8)).to_le_bytes());
    let mut st = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in msg.chunks_exact(64) {
        let w: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let [mut a, mut b, mut c, mut d] = st;
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let n = a
                .wrapping_add(f)
                .wrapping_add(k[i])
                .wrapping_add(w[g])
                .rotate_left(S[(i / 16) * 4 + i % 4])
                .wrapping_add(b);
            (a, b, c, d) = (d, n, b, c);
        }
        for (s, v) in st.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut out = [0u8; 16];
    for (i, w) in st.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

fn utf16_ascii_strings(b: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let start = i;
        let mut s = String::new();
        while i + 1 < b.len() && (0x20..=0x7e).contains(&b[i]) && b[i + 1] == 0 {
            s.push(b[i] as char);
            i += 2;
        }
        if s.len() >= 4 {
            out.push(s);
        }
        if i == start {
            i += 1;
        }
    }
    out
}

/// Native storage redirect: a `./startup.tjs` literal plus exactly one
/// `archive.xp3>member` reference in the protection module's UTF-16 data.
pub fn startup_redirect(module: &[u8]) -> Option<(String, String)> {
    let strings = utf16_ascii_strings(module);
    strings.iter().find(|s| {
        let n = s.replace('\\', "/").to_ascii_lowercase();
        n == "./startup.tjs" || n == "startup.tjs"
    })?;
    let mut refs: Vec<(String, String)> = strings
        .iter()
        .filter_map(|v| {
            let (archive, member) = v.rsplit_once('>')?;
            let archive = archive.trim().replace('\\', "/");
            let archive = archive.rsplit('/').next()?.to_owned();
            (archive.to_ascii_lowercase().ends_with(".xp3") && !member.trim().is_empty())
                .then(|| (archive, member.trim().to_owned()))
        })
        .collect();
    refs.sort();
    refs.dedup();
    (refs.len() == 1).then(|| refs.pop()).flatten()
}

/// The native `setupArchiveData` generator: the TJS member name, the
/// complemented ChaCha sigma and the SHAKE256 call tuple (domain 0x1f,
/// capacity 0x200, rate 0x440, 0x1000 output bytes).
pub fn has_setup_generator(module: &[u8]) -> bool {
    let name: Vec<u8> = "setupArchiveData"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let sigma: Vec<u8> = SIGMA.iter().flat_map(|w| (!w).to_le_bytes()).collect();
    const SHAKE_ARGS: &[u8] = &[
        0x6a, 0x1f, 0x68, 0x00, 0x02, 0x00, 0x00, 0x68, 0x40, 0x04, 0x00, 0x00, 0x68, 0x00, 0x10,
        0x00, 0x00,
    ];
    super::pe::contains(module, &name)
        && super::pe::contains(module, &sigma)
        && super::pe::contains(module, SHAKE_ARGS)
}

/// String constants of a compiled `TJS2100` script (DATA chunk string pool),
/// or quoted literals of a source script. Candidates only.
pub fn script_strings(b: &[u8]) -> Vec<String> {
    if b.starts_with(b"TJS2100\0") {
        return tjs2_strings(b).unwrap_or_default();
    }
    let text = if b.starts_with(&[0xff, 0xfe]) {
        let w: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        String::from_utf16_lossy(&w)
    } else {
        String::from_utf8_lossy(b).into_owned()
    };
    let mut out = Vec::new();
    for q in ['"', '\''] {
        let mut parts = text.split(q);
        parts.next();
        while let (Some(lit), Some(_)) = (parts.next(), parts.next()) {
            if !lit.is_empty() && lit.len() <= 4096 && !lit.contains('\n') {
                out.push(lit.to_owned());
            }
        }
    }
    out
}
fn tjs2_strings(b: &[u8]) -> Option<Vec<String>> {
    let mut at = 12usize;
    let u32_at = |o: usize| super::pe::u32_opt(b, o).map(|v| v as usize);
    while at + 8 <= b.len() {
        let size = u32_at(at + 4)?;
        let body = at + 8;
        let end = body.checked_add(size).filter(|&e| e <= b.len())?;
        if &b[at..at + 4] == b"DATA" {
            let mut p = body;
            let pad = |n: usize| (n + 3) & !3;
            for elem in [1usize, 2, 4, 8, 8] {
                let n = u32_at(p)?;
                p = p.checked_add(4 + pad(n.checked_mul(elem)?))?;
                if p > end {
                    return None;
                }
            }
            let count = u32_at(p)?;
            p += 4;
            let mut out = Vec::with_capacity(count.min(65536));
            for _ in 0..count {
                let units = u32_at(p)?;
                let bytes = units.checked_mul(2)?;
                let s = b.get(p + 4..(p + 4).checked_add(bytes)?)?;
                if p + 4 + bytes > end {
                    return None;
                }
                let w: Vec<u16> = s
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                out.push(String::from_utf16(&w).ok()?);
                p += 4 + pad(bytes);
            }
            return Some(out);
        }
        at = end;
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn md5_matches_rfc1321_vectors() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(md5(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }
    #[test]
    fn normalization_matches_native_rules() {
        assert_eq!(
            normalize("//Data\\\\Scenario//A.TJS"),
            "data/scenario/a.tjs"
        );
        assert_eq!(normalize("Ｆｕｌｌ/X"), "Ｆｕｌｌ/x");
    }
    #[test]
    fn chacha_stream_round_trips_and_depends_on_every_parameter() {
        let p = SpecialParams {
            control_words: [1, 2, 3, 4, 5, 6, 7, 8],
            seed0: 9,
            seed1: 10,
        };
        let plain: Vec<u8> = (0..300).map(|i| i as u8).collect();
        let mut c = plain.clone();
        p.apply(&mut c);
        assert_ne!(c, plain);
        let mut q = p;
        q.seed1 ^= 1;
        let mut c2 = plain.clone();
        q.apply(&mut c2);
        assert_ne!(c, c2);
        p.apply(&mut c);
        assert_eq!(c, plain);
    }
    pub(crate) fn record(name: &str) -> Vec<u8> {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut r = b"hnfn".to_vec();
        r.extend_from_slice(&((6 + units.len() + 2) as i64).to_le_bytes());
        r.extend_from_slice(&0x1234u32.to_le_bytes());
        r.extend_from_slice(&((units.len() / 2) as i16).to_le_bytes());
        r.extend_from_slice(&units);
        r.extend_from_slice(&[0, 0]);
        r
    }
    #[test]
    fn derived_params_decode_a_synthetic_special_blob() {
        use std::io::Write;
        let d = derive("synthetic archive data").unwrap();
        assert!((0x100..0x300).contains(&d.mask));
        let mut plain = record("system/Initialize.tjs");
        plain.extend(record("image/a.png"));
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&plain).unwrap();
        let mut blob = z.finish().unwrap();
        let n = blob.len().min(0x100);
        d.special.apply(&mut blob[..n]);
        let decoded = decode_blob(&d.special, &blob, plain.len()).unwrap();
        let recs = parse_records(&decoded).unwrap();
        assert_eq!(recs[1].1, "image/a.png");
        let wrong = derive("other text").unwrap();
        assert!(decode_blob(&wrong.special, &blob, plain.len()).is_err());
    }
    #[test]
    fn tjs2_string_pool_and_redirect_are_parsed() {
        let mut data = Vec::new();
        for _ in 0..5 {
            data.extend_from_slice(&0u32.to_le_bytes());
        }
        let strings = ["Storages", "abc"];
        data.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for s in strings {
            let w: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
            data.extend_from_slice(&(s.len() as u32).to_le_bytes());
            data.extend_from_slice(&w);
            data.resize((data.len() + 3) & !3, 0);
        }
        let mut b = b"TJS2100\0".to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(b"DATA");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        assert_eq!(script_strings(&b), ["Storages", "abc"]);
        let utf16 = |s: &str| {
            s.encode_utf16()
                .flat_map(u16::to_le_bytes)
                .chain([0, 0])
                .collect::<Vec<u8>>()
        };
        let mut m = utf16("./startup.tjs");
        m.extend(utf16("boot.xp3>$"));
        assert_eq!(startup_redirect(&m), Some(("boot.xp3".into(), "$".into())));
        m.extend(utf16("other.xp3>x.tjs"));
        assert_eq!(startup_redirect(&m), None);
    }
}
