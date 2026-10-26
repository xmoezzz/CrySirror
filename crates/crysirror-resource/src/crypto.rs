// SPDX-License-Identifier: MPL-2.0
//! Generic caller-key crypto primitives for resource profiles. No game key
//! catalogue, generated executable code, filesystem or implicit title choice.
use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
use anyhow::{ensure, Result};
/// RSA public operation with PKCS#1 v1.5 type-1 padding. This is for legacy
/// resource decoding, not an authentication API. Modulus bytes are big endian;
/// only initialized, unpadded payload bytes are returned.
pub fn rsa_public_decrypt(input: &[u8], modulus: &[u8], exponent: u32) -> Result<Vec<u8>> {
    use num_bigint::BigUint;
    ensure!(
        (64..=512).contains(&modulus.len()) && input.len() == modulus.len(),
        "RSA block/modulus size"
    );
    ensure!(exponent >= 3 && exponent & 1 == 1, "RSA public exponent");
    let n = BigUint::from_bytes_be(modulus);
    let value = BigUint::from_bytes_be(input);
    ensure!(value < n, "RSA block outside modulus");
    let result = value.modpow(&BigUint::from(exponent), &n).to_bytes_be();
    let mut padded = vec![0u8; modulus.len()];
    let start = padded
        .len()
        .checked_sub(result.len())
        .ok_or_else(|| anyhow::anyhow!("RSA output size"))?;
    padded[start..].copy_from_slice(&result);
    ensure!(padded.starts_with(&[0, 1]), "invalid RSA type-1 padding");
    let separator = padded[2..]
        .iter()
        .position(|b| *b != 255)
        .map(|i| i + 2)
        .ok_or_else(|| anyhow::anyhow!("RSA missing padding separator"))?;
    ensure!(
        separator >= 10 && padded[separator] == 0,
        "invalid RSA padding length/separator"
    );
    Ok(padded[separator + 1..].to_vec())
}
/// Native little-endian word Blowfish used by legacy containers. Full blocks
/// in the requested prefix are decrypted; trailing bytes remain unchanged.
pub fn blowfish_le_decrypt(input: &[u8], key: &[u8], prefix: usize) -> Result<Vec<u8>> {
    ensure!(
        (4..=56).contains(&key.len()),
        "Blowfish key must contain 4..56 bytes"
    );
    // The public RustCrypto default uses big-endian words. Reverse each native
    // u32 before/after block decryption to preserve upstream BF_LONG semantics.
    let cipher: blowfish::Blowfish = blowfish::Blowfish::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("Blowfish key length"))?;
    let mut out = input.to_vec();
    let n = prefix.min(out.len()) / 8 * 8;
    for block in out[..n].chunks_exact_mut(8) {
        block[..4].reverse();
        block[4..].reverse();
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
        block[..4].reverse();
        block[4..].reverse();
    }
    Ok(out)
}

pub fn aes_cbc_decrypt(input: &[u8], iv: &[u8; 16], key: &[u8], pkcs7: bool) -> Result<Vec<u8>> {
    ensure!(
        input.len().is_multiple_of(16),
        "AES CBC input must contain complete blocks"
    );
    ensure!(!pkcs7 || !input.is_empty(), "AES CBC PKCS7 input is empty");
    let mut out = input.to_vec();
    let mut previous = *iv;
    macro_rules! blocks {
        ($t:ty) => {{
            let cipher =
                <$t>::new_from_slice(key).map_err(|_| anyhow::anyhow!("AES CBC key length"))?;
            for block in out.chunks_exact_mut(16) {
                let original: [u8; 16] = block.try_into().unwrap();
                cipher.decrypt_block(GenericArray::from_mut_slice(block));
                for (b, prev) in block.iter_mut().zip(previous) {
                    *b ^= prev;
                }
                previous = original;
            }
        }};
    }
    match key.len() {
        16 => blocks!(aes::Aes128),
        24 => blocks!(aes::Aes192),
        32 => blocks!(aes::Aes256),
        _ => anyhow::bail!("AES CBC key must contain 16,24 or32 bytes"),
    }
    if pkcs7 {
        let n = usize::from(*out.last().unwrap());
        ensure!(
            (1..=16).contains(&n) && out[out.len() - n..].iter().all(|b| usize::from(*b) == n),
            "invalid AES CBC PKCS7 padding"
        );
        out.truncate(out.len() - n);
    }
    Ok(out)
}
pub fn aes256_cbc_decrypt(input: &[u8], iv: &[u8; 16], key: &[u8]) -> Result<Vec<u8>> {
    ensure!(key.len() == 32, "AES256 CBC needs a 32-byte key");
    aes_cbc_decrypt(input, iv, key, true)
}
pub fn rc4_crypt(input: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        (1..=256).contains(&key.len()),
        "RC4 key must contain 1..256 bytes"
    );
    let mut s = std::array::from_fn::<_, 256, _>(|i| i as u8);
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + usize::from(s[i]) + usize::from(key[i % key.len()])) & 255;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    let mut out = input.to_vec();
    for b in &mut out {
        i = (i + 1) & 255;
        j = (j + usize::from(s[i])) & 255;
        s.swap(i, j);
        *b ^= s[usize::from(s[i].wrapping_add(s[j]))];
    }
    Ok(out)
}
/// MD5 including standard padding, with caller-specified chaining IV.
/// Used by legacy resource metadata, never for authentication.
pub fn md5_with_iv(input: &[u8], iv: [u32; 4]) -> [u32; 4] {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut state = iv;
    let compress = |state: &mut [u32; 4], block: &[u8]| {
        let m = std::array::from_fn::<_, 16, _>(|i| {
            u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let [mut a, mut b, mut c, mut d] = *state;
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(SHIFTS[i]),
            );
            a = d;
            d = c;
            c = b;
            b = next;
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    };
    let mut chunks = input.chunks_exact(64);
    for block in &mut chunks {
        compress(&mut state, block);
    }
    let tail = chunks.remainder();
    let mut pad = [0u8; 128];
    pad[..tail.len()].copy_from_slice(tail);
    pad[tail.len()] = 0x80;
    let n = if tail.len() < 56 { 64 } else { 128 };
    pad[n - 8..n].copy_from_slice(&(input.len() as u64).wrapping_mul(8).to_le_bytes());
    for block in pad[..n].chunks_exact(64) {
        compress(&mut state, block);
    }
    state
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_hash_stream_and_cipher_vectors() {
        let iv = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
        assert_eq!(
            md5_with_iv(b"", iv),
            [0xd98c1dd4, 0x04b2008f, 0x980980e9, 0x7e42f8ec]
        );
        assert_eq!(
            md5_with_iv(b"abc", iv),
            [0x98500190, 0xb04fd23c, 0x7d3f96d6, 0x727fe128]
        );
        assert_eq!(
            rc4_crypt(b"Plaintext", b"Key").unwrap(),
            [0xbb, 0xf3, 0x16, 0xe8, 0xd9, 0x40, 0xaf, 0x0a, 0xd3]
        );
        let key = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let iv = std::array::from_fn(|i| i as u8);
        let input = [
            0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9,
            0x19, 0x7d,
        ];
        assert_eq!(
            aes_cbc_decrypt(&input, &iv, &key, false).unwrap(),
            [
                0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
                0x17, 0x2a
            ]
        );
        assert!(aes_cbc_decrypt(&input[..15], &iv, &key, false).is_err());
        assert!(aes_cbc_decrypt(&input, &iv, &key, true).is_err());
    }
}
