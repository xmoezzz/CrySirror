// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded, cancellable inference of a constant XOR byte using Adler-32.
//!
//! Returns every checksum-compatible key; a checksum collision is deliberately
//! not resolved by guessing a file type or choosing the first candidate. The
//! caller must obtain independent evidence before publishing decoded resources.
use anyhow::{ensure, Result};

/// Maximum input to one checksum-assisted key search (64 MiB).
pub const MAX_XOR_RECOVERY_BYTES: usize = 64 * 1024 * 1024;
const MODULUS: u64 = 65521;

/// Find all single-byte XOR keys whose decoded bytes have `expected_adler32`.
///
/// The input stays immutable. A byte-frequency and position-weighted histogram
/// reduces the work to O(input length + 256²), using bounded stack storage.
/// `check_cancelled` runs before work, every 64 KiB, during key enumeration and
/// before returning a complete result. An empty input with checksum 1 returns
/// all 256 keys, expressing the absence of key evidence.
pub fn single_byte_xor_adler32_candidates(
    ciphertext: &[u8],
    expected_adler32: u32,
    check_cancelled: &dyn Fn() -> Result<()>,
) -> Result<Vec<u8>> {
    check_cancelled()?;
    ensure!(
        ciphertext.len() <= MAX_XOR_RECOVERY_BYTES,
        "XOR recovery input byte limit"
    );
    ensure!(
        u64::from(expected_adler32 & 0xffff) < MODULUS
            && u64::from(expected_adler32 >> 16) < MODULUS,
        "invalid Adler-32 components"
    );
    let mut counts = [0u64; 256];
    let mut weighted = [0u64; 256];
    for (index, chunk) in ciphertext.chunks(65536).enumerate() {
        check_cancelled()?;
        let start = index * 65536;
        for (j, &byte) in chunk.iter().enumerate() {
            let slot = usize::from(byte);
            counts[slot] += 1;
            weighted[slot] = (weighted[slot] + (ciphertext.len() - start - j) as u64) % MODULUS;
        }
    }
    let mut candidates = Vec::new();
    for key in 0..=255u8 {
        check_cancelled()?;
        let mut a = 1u64;
        let mut b = ciphertext.len() as u64 % MODULUS;
        for byte in 0..256 {
            let plain = u64::from(byte as u8 ^ key);
            a += counts[byte] * plain;
            b += weighted[byte] * plain;
        }
        let checksum = ((b % MODULUS) << 16) | (a % MODULUS);
        if checksum == u64::from(expected_adler32) {
            candidates.push(key);
        }
    }
    check_cancelled()?;
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn adler(input: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &v in input {
            a = (a + u32::from(v)) % 65521;
            b = (b + a) % 65521;
        }
        b << 16 | a
    }
    #[test]
    fn histogram_matches_independent_exhaustive_checksum_search() {
        for length in [0, 1, 2, 31, 256, 1027, 65522] {
            let plain: Vec<u8> = (0..length)
                .map(|i| ((i * 73 + i / 7) % 256) as u8)
                .collect();
            for key in [0u8, 1, 0x58, 0xd0, 255] {
                let encoded: Vec<_> = plain.iter().map(|b| b ^ key).collect();
                let expected = adler(&plain);
                let candidates =
                    single_byte_xor_adler32_candidates(&encoded, expected, &|| Ok(())).unwrap();
                let reference: Vec<_> = (0..=255u8)
                    .filter(|k| {
                        adler(&encoded.iter().map(|b| b ^ k).collect::<Vec<_>>()) == expected
                    })
                    .collect();
                assert_eq!(candidates, reference, "length={length} key={key}");
                assert!(candidates.contains(&key));
            }
        }
    }
    #[test]
    fn ambiguity_empty_stream_and_invalid_components_remain_explicit() {
        assert_eq!(
            single_byte_xor_adler32_candidates(&[], 1, &|| Ok(())).unwrap(),
            (0..=255u8).collect::<Vec<_>>()
        );
        assert!(single_byte_xor_adler32_candidates(&[], 0, &|| Ok(()))
            .unwrap()
            .is_empty());
        // Both the sum and position-weighted sum are invariant under every
        // constant XOR key for this nonempty, balanced four-byte stream.
        let balanced = [0u8, 255, 255, 0];
        assert_eq!(
            single_byte_xor_adler32_candidates(&balanced, adler(&balanced), &|| Ok(())).unwrap(),
            (0..=255u8).collect::<Vec<_>>()
        );
        assert!(single_byte_xor_adler32_candidates(&[7], 0xffff0001, &|| Ok(())).is_err());
        let too_large = vec![0; MAX_XOR_RECOVERY_BYTES + 1];
        assert!(single_byte_xor_adler32_candidates(&too_large, 1, &|| Ok(())).is_err());
    }
    #[test]
    fn cancellation_does_not_modify_or_publish_partial_input() {
        let input = vec![0x5a; 130000];
        for cancel_at in [1, 3, 12, 260] {
            let calls = Cell::new(0);
            let result = single_byte_xor_adler32_candidates(&input, adler(&input), &|| {
                calls.set(calls.get() + 1);
                ensure!(calls.get() < cancel_at, "cancelled");
                Ok(())
            });
            assert!(result.is_err());
            assert!(input.iter().all(|b| *b == 0x5a));
        }
    }
}
