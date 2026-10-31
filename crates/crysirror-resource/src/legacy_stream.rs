// Copyright (c) 2026 CrySirror contributors.
// MPL-2.0; legacy MT profile follows the published 1998 MT19937 algorithm.
//! Deterministic legacy resource transforms. These are format compatibility
//! kernels, not cryptographic security or game authentication primitives.
/// XOR one low output byte per byte using the 1998 MT initialization
/// recurrence `state[i] = 69069 * state[i-1]`. This deliberately differs from
/// both MT2002 and the split-high-word initialization used by some INT files.
pub fn mt19937_1998_xor(bytes: &mut [u8], seed: u32) {
    let mut state = [0u32; 624];
    state[0] = seed;
    for i in 1..624 {
        state[i] = state[i - 1].wrapping_mul(69069);
    }
    let mut cursor = 624;
    for byte in bytes {
        if cursor == 624 {
            for i in 0..624 {
                let pair = (state[i] & 0x8000_0000) | (state[(i + 1) % 624] & 0x7fff_ffff);
                state[i] = state[(i + 397) % 624]
                    ^ (pair >> 1)
                    ^ if pair & 1 != 0 { 0x9908_b0df } else { 0 };
            }
            cursor = 0;
        }
        let mut value = state[cursor];
        cursor += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^= value >> 18;
        *byte ^= value as u8;
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn published_1998_profile_matches_reference_c_oracle() {
        // Unmodified libsys4 mt19937int.c, b14839f, seed 0x5d3e3;
        // one genrand low byte per byte. No game-derived fixture.
        let expected = [
            0x28, 0xb0, 0x50, 0xe9, 0xd6, 0xcf, 0x93, 0x5b, 0xe4, 0xed, 0x7e, 0xa1, 0xfe, 0x72,
            0xc1, 0xae, 0x25, 0x65, 0xda, 0xde, 0x68, 0x5e, 0x1b, 0x6d, 0xbb, 0x95, 0x88, 0x11,
            0xf1, 0xbe, 0x72, 0xa4,
        ];
        let mut bytes = [0; 32];
        super::mt19937_1998_xor(&mut bytes, 0x5d3e3);
        assert_eq!(bytes, expected);
        super::mt19937_1998_xor(&mut bytes, 0x5d3e3);
        assert_eq!(bytes, [0; 32]);
    }
}
/// Decode complete little-endian words with independently advancing XOR and
/// additive masks. Arithmetic wraps exactly at 32 bits. Used by native script
/// and resource profiles; the caller supplies all policy and seed values.
pub fn rolling_word_xor_add(
    bytes: &mut [u8],
    mut xor: u32,
    xor_increment: u32,
    add_increment: u32,
) -> anyhow::Result<()> {
    anyhow::ensure!(bytes.len() % 4 == 0, "rolling word transform alignment");
    let mut add = 0u32;
    for word in bytes.chunks_exact_mut(4) {
        let value = u32::from_le_bytes(word.try_into().unwrap());
        word.copy_from_slice(&(value ^ xor).wrapping_add(add).to_le_bytes());
        xor = xor.wrapping_add(xor_increment);
        add = add.wrapping_add(add_increment);
    }
    Ok(())
}
