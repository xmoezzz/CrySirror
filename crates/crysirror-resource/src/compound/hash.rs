// Portions adapted from xmoezzz/xp3-brute, revision c82cb14302138125640478556723705aa720007b
// (src/hxv4.rs). Copyright (c) xp3-brute contributors; modifications CrySirror
// contributors. MIT License: see NOTICE-MIT in this directory.
//! Keyed-name hashes of the compound storage media. Inputs are UTF-16LE text
//! followed by the UTF-16LE media name; the effective hash key is empty.

/// `pathHash`: SipHash-2-4 with a zero key over UTF-16LE `path + media`, as
/// little-endian bytes. Directories are spelled with their trailing `/`; the
/// root is `""`. (The pinned xp3-brute revision returned big-endian bytes of
/// slashless spellings, which no authenticated directory record matches.)
pub fn path_hash(path: &str, media: &str) -> [u8; 8] {
    let mut text = path.to_string();
    text.push_str(media);
    let bytes = utf16le_bytes(&text);
    siphash24_zero(&bytes).to_le_bytes()
}

/// `fileHash`: the BLAKE2s-like compression over UTF-16LE `name + media`.
pub fn file_hash(name: &str, media: &str) -> [u8; 32] {
    let mut text = name.to_string();
    text.push_str(media);
    let data = utf16le_bytes(&text);
    let mut state = [
        0x6B08E647u32,
        0xBB67AE85,
        0x3C6EF372,
        0xA54FF53A,
        0x510E527F,
        0x9B05688C,
        0x1F83D9AB,
        0x5BE0CD19,
        0,
        0,
        0,
        0,
    ];
    let mut total = 0u32;
    let mut offset = 0usize;
    while offset < data.len() {
        let take = (data.len() - offset).min(64);
        total = total.wrapping_add(take as u32);
        state[8] = total;
        state[10] = if offset + take == data.len() {
            u32::MAX
        } else {
            0
        };
        let mut block = [0u8; 64];
        block[..take].copy_from_slice(&data[offset..offset + take]);
        blake2s_compress_custom(&mut state, &block);
        offset += take;
    }
    let mut out = [0u8; 32];
    for (i, word) in state[..8].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

fn utf16le_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for word in text.encode_utf16() {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out
}

fn blake2s_compress_custom(state: &mut [u32; 12], block: &[u8; 64]) {
    const IV: [u32; 8] = [
        0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB,
        0x5BE0CD19,
    ];
    const SIGMA: [[usize; 16]; 10] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
        [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
        [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
        [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
        [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
        [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
        [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
        [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
        [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    ];
    let mut v = [0u32; 16];
    v[..8].copy_from_slice(&state[..8]);
    v[8..].copy_from_slice(&IV);
    v[12] ^= state[8];
    v[13] ^= state[9];
    v[14] ^= state[10];
    v[15] ^= state[11];
    let mut m = [0u32; 16];
    for i in 0..16 {
        m[i] = u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
    }
    for s in SIGMA {
        g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
        g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }
    for i in 0..8 {
        state[i] ^= v[i] ^ v[i + 8];
    }
}
fn g(v: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, x: u32, y: u32) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(12);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(8);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(7);
}

fn siphash24_zero(data: &[u8]) -> u64 {
    let mut v0 = 0x736f6d6570736575u64;
    let mut v1 = 0x646f72616e646f6du64;
    let mut v2 = 0x6c7967656e657261u64;
    let mut v3 = 0x7465646279746573u64;
    let full = data.len() / 8 * 8;
    for chunk in data[..full].chunks_exact(8) {
        let m = u64::from_le_bytes(chunk.try_into().unwrap());
        v3 ^= m;
        sipround(&mut v0, &mut v1, &mut v2, &mut v3);
        sipround(&mut v0, &mut v1, &mut v2, &mut v3);
        v0 ^= m;
    }
    let tail = &data[full..];
    let mut b = (data.len() as u64) << 56;
    for (i, &x) in tail.iter().enumerate() {
        b |= (x as u64) << (8 * i);
    }
    v3 ^= b;
    sipround(&mut v0, &mut v1, &mut v2, &mut v3);
    sipround(&mut v0, &mut v1, &mut v2, &mut v3);
    v0 ^= b;
    v2 ^= 0xff;
    for _ in 0..4 {
        sipround(&mut v0, &mut v1, &mut v2, &mut v3);
    }
    v0 ^ v1 ^ v2 ^ v3
}
fn sipround(v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64) {
    *v0 = v0.wrapping_add(*v1);
    *v1 = v1.rotate_left(13) ^ *v0;
    *v0 = v0.rotate_left(32);
    *v2 = v2.wrapping_add(*v3);
    *v3 = v3.rotate_left(16) ^ *v2;
    *v0 = v0.wrapping_add(*v3);
    *v3 = v3.rotate_left(21) ^ *v0;
    *v2 = v2.wrapping_add(*v1);
    *v1 = v1.rotate_left(17) ^ *v2;
    *v2 = v2.rotate_left(32);
}

