// Portions adapted from xmoezzz/xp3-brute c82cb14302138125640478556723705aa720007b
// (src/decoder/pbd.rs), MIT License (docs/third-party/xp3-brute.txt).
// Modifications CrySirror contributors (MPL-2.0).
//! KiriKiri binary data packs (`.pbd`): `TJS/ns0` and `TJS/4s0` containers.
//!
//! `4s0`: 16-byte header (seed, crypt mode, IV length), IV, then an optionally
//! ChaCha-encrypted stream of `[u16 size][LZ4 block]` frames decoded with a
//! 64 KiB history dictionary, then the type-checked value stream and a u32
//! trailer. `ns0`: the value stream directly, followed by a checksum. Both
//! decode; [`encode_ns0`] writes `ns0` (the `4s0` trailer's meaning is not
//! established, so new `4s0` packs are not invented). The value tree is
//! VM-neutral.
use anyhow::{Result, bail, ensure};
use blake2::{
    Blake2sMac256,
    digest::{KeyInit, Mac},
};
use lz4_flex::block::decompress_with_dict;
use xxhash_rust::xxh32::xxh32;

pub const NS0_MAGIC: &[u8; 8] = b"TJS/ns0\0";
pub const FOUR_S0_MAGIC: &[u8; 8] = b"TJS/4s0\0";
const HEADER_SIZE: usize = 16;
const LZ4_DICT_SIZE: usize = 64 * 1024;
const MAX_DEPTH: usize = 256;
const MAX_CONTAINER_ITEMS: usize = 1_000_000;
const MAX_STRING_UNITS: usize = 64 * 1024 * 1024;
/// Upper bound of a decompressed 4s0 stream.
pub const MAX_PLAIN_BYTES: usize = 256 * 1024 * 1024;

/// A decoded value. Dictionaries keep entry order and duplicate keys.
#[derive(Clone, Debug, PartialEq)]
pub enum PbdValue {
    Void,
    String(String),
    Integer(i64),
    Real(f64),
    Array(Vec<PbdValue>),
    Dictionary(Vec<(String, PbdValue)>),
}

pub fn is_pbd(bytes: &[u8]) -> bool {
    bytes.starts_with(NS0_MAGIC) || bytes.starts_with(FOUR_S0_MAGIC)
}

/// Decode a `.pbd` container to its root value.
pub fn decode(bytes: &[u8]) -> Result<PbdValue> {
    ensure!(bytes.len() >= HEADER_SIZE, "PBD header is truncated");
    let seed = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let crypt = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
    let iv_len = u16::from_le_bytes(bytes[14..16].try_into().unwrap()) as usize;
    if bytes.starts_with(NS0_MAGIC) {
        ensure!(crypt == 0 && iv_len == 0, "TJS/ns0 must have crypt=0 and iv_len=0");
        let mut checker = ByteChecker { seed };
        let mut cursor = Cursor { bytes: &bytes[HEADER_SIZE..], pos: 0 };
        let root = parse_value(&mut cursor, &mut checker, 0)?;
        let checksum = cursor.read_u32_le()?;
        ensure!(
            checksum == checker.final_check(),
            "TJS/ns0 checksum mismatch: stored=0x{checksum:08x} expected=0x{:08x}",
            checker.final_check()
        );
        ensure!(cursor.pos == cursor.bytes.len(), "TJS/ns0 has trailing bytes after checksum");
        return Ok(root);
    }
    ensure!(bytes.starts_with(FOUR_S0_MAGIC), "PBD magic is neither TJS/ns0 nor TJS/4s0");
    let iv_end = HEADER_SIZE + iv_len;
    ensure!(iv_end <= bytes.len(), "TJS/4s0 IV is truncated");
    let iv = &bytes[HEADER_SIZE..iv_end];
    let mut framed = bytes[iv_end..].to_vec();
    crypt_in_place(&mut framed, seed, crypt, iv)?;
    let plain = decompress_frames(&framed)?;
    let mut cursor = Cursor { bytes: &plain, pos: 0 };
    let mut checker = FourS0Checker::new(seed);
    let root = parse_value(&mut cursor, &mut checker, 0)?;
    // The trailing u32's engine-side meaning is not needed for decoding.
    cursor.read_u32_le()?;
    ensure!(cursor.pos == cursor.bytes.len(), "TJS/4s0 has trailing bytes after trailer");
    Ok(root)
}

trait TypeChecker {
    fn byte_for_type(&mut self, type_code: u8) -> u8;
}

struct ByteChecker {
    seed: u32,
}
impl ByteChecker {
    fn round(bytes: &mut [u8; 4]) {
        let a = bytes[0] ^ bytes[0].wrapping_mul(2);
        let mut b = a;
        b >>= 2;
        b ^= bytes[2];
        b >>= 3;
        b ^= bytes[2];
        b ^= a;
        bytes[0] = bytes[1];
        bytes[1] = bytes[2];
        bytes[2] = b;
    }
    fn final_check(&self) -> u32 {
        let mut bytes = self.seed.to_le_bytes();
        Self::round(&mut bytes);
        Self::round(&mut bytes);
        Self::round(&mut bytes);
        bytes.swap(0, 2);
        u32::from_le_bytes(bytes)
    }
}
impl TypeChecker for ByteChecker {
    fn byte_for_type(&mut self, type_code: u8) -> u8 {
        let mut bytes = self.seed.to_le_bytes();
        if type_code == 0 {
            return bytes[2];
        }
        Self::round(&mut bytes);
        self.seed = u32::from_le_bytes(bytes);
        bytes[2]
    }
}

struct FourS0Checker {
    b0: u8,
    b1: u8,
    b2: u8,
}
impl FourS0Checker {
    fn new(seed: u32) -> Self {
        Self {
            b0: ((seed >> 24) as u8) ^ seed as u8,
            b1: (seed >> 8) as u8,
            b2: (seed >> 16) as u8,
        }
    }
}
impl TypeChecker for FourS0Checker {
    fn byte_for_type(&mut self, type_code: u8) -> u8 {
        if type_code == 0 {
            return self.b2;
        }
        let t = self.b0 ^ self.b0.wrapping_shl(1);
        self.b0 = self.b1;
        self.b1 = self.b2;
        self.b2 = self.b2 ^ (self.b2 >> 3) ^ t ^ (t >> 5);
        self.b2
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }
    fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).filter(|e| *e <= self.bytes.len());
        let Some(end) = end else { bail!("PBD value stream is truncated") };
        let value = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(value)
    }
    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_exact(1)?[0])
    }
    fn read_u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_exact(4)?.try_into().unwrap()))
    }
    fn read_u64_le(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_exact(8)?.try_into().unwrap()))
    }
}

fn parse_value<C: TypeChecker>(cursor: &mut Cursor<'_>, checker: &mut C, depth: usize) -> Result<PbdValue> {
    ensure!(depth <= MAX_DEPTH, "PBD value nesting exceeds safety limit");
    let type_code = cursor.read_u8()?;
    let check = cursor.read_u8()?;
    let expected = checker.byte_for_type(type_code);
    ensure!(
        check == expected,
        "PBD type check mismatch at byte {}: type=0x{type_code:02x} stored=0x{check:02x} expected=0x{expected:02x}",
        cursor.pos.saturating_sub(1)
    );
    Ok(match type_code {
        0 => PbdValue::Void,
        2 => PbdValue::String(read_utf16_string(cursor)?),
        4 => PbdValue::Integer(cursor.read_u64_le()? as i64),
        5 => PbdValue::Real(f64::from_bits(cursor.read_u64_le()?)),
        0x81 => {
            let count = cursor.read_u32_le()? as usize;
            ensure!(
                count <= MAX_CONTAINER_ITEMS && count <= cursor.remaining() / 2 + 1,
                "PBD array count exceeds safety/remaining-data limit"
            );
            let mut items = Vec::with_capacity(count.min(65536));
            for _ in 0..count {
                items.push(parse_value(cursor, checker, depth + 1)?);
            }
            PbdValue::Array(items)
        }
        0xc1 => {
            let count = cursor.read_u32_le()? as usize;
            ensure!(count <= MAX_CONTAINER_ITEMS, "PBD dictionary count exceeds safety limit");
            let mut entries = Vec::with_capacity(count.min(65536));
            for _ in 0..count {
                let key = read_utf16_string(cursor)?;
                entries.push((key, parse_value(cursor, checker, depth + 1)?));
            }
            PbdValue::Dictionary(entries)
        }
        other => bail!("unsupported PBD value type 0x{other:02x}"),
    })
}

fn read_utf16_string(cursor: &mut Cursor<'_>) -> Result<String> {
    let units = cursor.read_u32_le()? as usize;
    ensure!(units <= MAX_STRING_UNITS, "PBD UTF-16 string exceeds safety limit");
    let bytes = cursor.read_exact(units * 2)?;
    let values: Vec<u16> = bytes.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
    String::from_utf16(&values).map_err(|_| anyhow::anyhow!("PBD string contains invalid UTF-16"))
}

/// PackinOne ChaCha variants: (rounds, 64-byte blocks per keystream batch).
fn crypt_parameters(crypt: u16) -> Result<Option<(usize, usize)>> {
    Ok(match crypt {
        0 => None,
        1 => Some((8, 0x10)),
        2 => Some((12, 8)),
        3 => Some((20, 4)),
        4 => Some((8, 1)),
        5 => Some((12, 1)),
        6 => Some((20, 1)),
        _ => bail!("unsupported TJS/4s0 crypt field {crypt}"),
    })
}

/// Key = BLAKE2s-256 keyed with the seed over the IV; nonce = XXH32(iv, seed)
/// and the seed; keystream batches expand the first block by xorshift.
fn crypt_in_place(data: &mut [u8], seed: u32, crypt: u16, iv: &[u8]) -> Result<()> {
    let Some((rounds, expand_blocks)) = crypt_parameters(crypt)? else {
        return Ok(());
    };
    let mut mac = <Blake2sMac256 as KeyInit>::new_from_slice(&seed.to_le_bytes())
        .map_err(|_| anyhow::anyhow!("failed to initialize BLAKE2s PBD key derivation"))?;
    Mac::update(&mut mac, iv);
    let mut key = [0u8; 32];
    key.copy_from_slice(&mac.finalize().into_bytes());
    let nonce_lo = xxh32(iv, seed);
    let nonce_hi = seed;
    let mut fallback = nonce_hi ^ nonce_lo;
    if fallback == 0 {
        fallback = seed;
    }
    if fallback == 0 {
        fallback = 0xffff_ffff;
    }
    let mut offset = 0usize;
    let mut counter = 0u64;
    while offset < data.len() {
        let base = chacha_block(&key, counter, nonce_lo, nonce_hi, rounds);
        counter = counter.wrapping_add(1);
        let stream = expand_stream_block(&base, expand_blocks, fallback);
        let take = stream.len().min(data.len() - offset);
        for (dst, k) in data[offset..offset + take].iter_mut().zip(&stream[..take]) {
            *dst ^= *k;
        }
        offset += take;
    }
    Ok(())
}

fn chacha_block(key: &[u8; 32], counter: u64, nonce_lo: u32, nonce_hi: u32, rounds: usize) -> [u8; 64] {
    let constants = *b"expand 32-byte k";
    let mut state = [0u32; 16];
    for i in 0..4 {
        state[i] = u32::from_le_bytes(constants[i * 4..i * 4 + 4].try_into().unwrap());
    }
    for i in 0..8 {
        state[4 + i] = u32::from_le_bytes(key[i * 4..i * 4 + 4].try_into().unwrap());
    }
    state[12] = counter as u32;
    state[13] = (counter >> 32) as u32;
    state[14] = nonce_lo;
    state[15] = nonce_hi;
    let original = state;
    for _ in 0..rounds / 2 {
        quarter_round(&mut state, 0, 4, 8, 12);
        quarter_round(&mut state, 1, 5, 9, 13);
        quarter_round(&mut state, 2, 6, 10, 14);
        quarter_round(&mut state, 3, 7, 11, 15);
        quarter_round(&mut state, 0, 5, 10, 15);
        quarter_round(&mut state, 1, 6, 11, 12);
        quarter_round(&mut state, 2, 7, 8, 13);
        quarter_round(&mut state, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&state[i].wrapping_add(original[i]).to_le_bytes());
    }
    out
}

fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn xorshift32(mut x: u32, fallback: u32) -> u32 {
    x ^= x.wrapping_shl(13);
    x ^= x >> 17;
    x ^= x.wrapping_shl(5);
    if x == 0 { fallback } else { x }
}

/// PackinOne expands from a moving source index over the words already
/// produced; it is not one xorshift chain from the final word.
fn expand_stream_block(base: &[u8; 64], blocks: usize, fallback: u32) -> Vec<u8> {
    if blocks <= 1 {
        return base.to_vec();
    }
    let mut words: Vec<u32> = base.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let mut src = 0usize;
    for _ in 0..blocks * 4 - 4 {
        for _ in 0..4 {
            let source = words[src];
            src += 1;
            words.push(xorshift32(source, fallback));
        }
    }
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn decompress_frames(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    while pos < bytes.len() {
        ensure!(bytes.len() - pos >= 2, "TJS/4s0 LZ4 frame has truncated u16 block size");
        let size = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;
        if size == 0 {
            ensure!(bytes[pos..].iter().all(|&b| b == 0), "TJS/4s0 has non-zero data after LZ4 terminator");
            return Ok(out);
        }
        let end = pos + size;
        ensure!(end <= bytes.len(), "TJS/4s0 LZ4 block is truncated");
        let dict = &out[out.len().saturating_sub(LZ4_DICT_SIZE)..];
        let block = match decompress_with_dict(&bytes[pos..end], LZ4_DICT_SIZE, dict) {
            Ok(block) => block,
            Err(first) if !dict.is_empty() => decompress_with_dict(&bytes[pos..end], LZ4_DICT_SIZE, &[])
                .map_err(|second| {
                    anyhow::anyhow!(
                        "TJS/4s0 LZ4 decode failed with dictionary ({first}) and without dictionary ({second})"
                    )
                })?,
            Err(e) => bail!("TJS/4s0 LZ4 decode failed: {e}"),
        };
        ensure!(!block.is_empty() && block.len() <= LZ4_DICT_SIZE, "TJS/4s0 LZ4 block size");
        ensure!(out.len() + block.len() <= MAX_PLAIN_BYTES, "TJS/4s0 decompressed size limit");
        out.extend_from_slice(&block);
        pos = end;
    }
    Ok(out)
}

/// Encode `root` as a `TJS/ns0` pack with checksum `seed` (xp3-brute
/// `encode_pbd`, ns0 branch). Real numbers keep their exact bits.
pub fn encode_ns0(root: &PbdValue, seed: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(NS0_MAGIC);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let mut checker = ByteChecker { seed };
    write_value(&mut out, root, &mut checker, 0)?;
    out.extend_from_slice(&checker.final_check().to_le_bytes());
    ensure!(out.len() <= MAX_PLAIN_BYTES, "PBD exceeds the size limit");
    Ok(out)
}

fn write_value<C: TypeChecker>(out: &mut Vec<u8>, value: &PbdValue, checker: &mut C, depth: usize) -> Result<()> {
    ensure!(depth <= MAX_DEPTH, "PBD value nesting exceeds safety limit");
    let type_code = match value {
        PbdValue::Void => 0,
        PbdValue::String(_) => 2,
        PbdValue::Integer(_) => 4,
        PbdValue::Real(_) => 5,
        PbdValue::Array(_) => 0x81,
        PbdValue::Dictionary(_) => 0xc1,
    };
    out.push(type_code);
    out.push(checker.byte_for_type(type_code));
    match value {
        PbdValue::Void => {}
        PbdValue::String(s) => write_utf16_string(out, s)?,
        PbdValue::Integer(v) => out.extend_from_slice(&v.to_le_bytes()),
        PbdValue::Real(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
        PbdValue::Array(items) => {
            ensure!(items.len() <= MAX_CONTAINER_ITEMS, "PBD array is too large");
            out.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for item in items {
                write_value(out, item, checker, depth + 1)?;
            }
        }
        PbdValue::Dictionary(entries) => {
            ensure!(entries.len() <= MAX_CONTAINER_ITEMS, "PBD dictionary is too large");
            out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
            for (key, value) in entries {
                write_utf16_string(out, key)?;
                write_value(out, value, checker, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn write_utf16_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let utf16: Vec<u16> = value.encode_utf16().collect();
    ensure!(utf16.len() <= MAX_STRING_UNITS, "PBD UTF-16 string is too large");
    out.extend_from_slice(&(utf16.len() as u32).to_le_bytes());
    for unit in utf16 {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn ns0_encode_round_trips_through_decode() {
        let root = PbdValue::Dictionary(vec![
            ("name".into(), PbdValue::String("体験版".into())),
            ("n".into(), PbdValue::Integer(-7)),
            ("r".into(), PbdValue::Real(-0.0)),
            ("list".into(), PbdValue::Array(vec![PbdValue::Void, PbdValue::Integer(3)])),
            ("n".into(), PbdValue::Integer(8)),
        ]);
        for seed in [0, 1, 0xdead_beef] {
            let bytes = encode_ns0(&root, seed).unwrap();
            assert!(bytes.starts_with(NS0_MAGIC));
            let back = decode(&bytes).unwrap();
            assert_eq!(back, root);
            if let PbdValue::Dictionary(e) = &back {
                assert!(matches!(e[2].1, PbdValue::Real(v) if v.to_bits() == (-0.0f64).to_bits()));
            }
        }
    }

    use super::*;
    #[test]
    fn ns0_scalars_round_through_the_checker() {
        // Hand-built ns0: dictionary {"a": 7}.
        let seed = 0x0053_4A54u32;
        let mut c = ByteChecker { seed };
        let mut body = vec![0xc1, c.byte_for_type(0xc1)];
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&u16::from(b'a').to_le_bytes());
        body.extend_from_slice(&[4, c.byte_for_type(4)]);
        body.extend_from_slice(&7i64.to_le_bytes());
        body.extend_from_slice(&c.final_check().to_le_bytes());
        let mut bytes = NS0_MAGIC.to_vec();
        bytes.extend_from_slice(&seed.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(&body);
        assert_eq!(
            decode(&bytes).unwrap(),
            PbdValue::Dictionary(vec![("a".into(), PbdValue::Integer(7))])
        );
        bytes[20] ^= 1;
        assert!(decode(&bytes).is_err());
    }
}
