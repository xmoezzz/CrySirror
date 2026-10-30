// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared sRGB byte/linear conversion. Encoding reproduces the engine's f32
//! transfer and rounding boundaries; a small table predicts the interval and
//! comparisons against the unquantized input select the exact byte.
use std::sync::OnceLock;

const BINS: usize = 4096;
pub struct Srgb8Transfer {
    linear: [f32; 256],
    cuts: [f32; 255],
    lower: [u8; BINS],
}
/// The immutable process-shared transfer, reusable across a pixel batch.
pub fn srgb8_transfer() -> &'static Srgb8Transfer {
    static TABLE: OnceLock<Srgb8Transfer> = OnceLock::new();
    TABLE.get_or_init(|| {
        let decode = |s: f32| {
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        // Search the positive f32 bit ordering for each transition of the
        // original transfer, including its intermediate f32 rounding. This is
        // deliberately stronger than mathematical sRGB midpoint thresholds.
        let cuts = std::array::from_fn(|byte| {
            let (mut lo, mut hi) = (0u32, 1f32.to_bits());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if reference(f32::from_bits(mid)) as usize > byte {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            f32::from_bits(lo)
        });
        let lower = std::array::from_fn(|i| {
            cuts.partition_point(|&threshold| threshold <= i as f32 / BINS as f32) as u8
        });
        Srgb8Transfer {
            linear: std::array::from_fn(|i| decode(i as f32 / 255.)),
            cuts,
            lower,
        }
    })
}

/// Decode an sRGB byte into linear light, without applying alpha.
#[inline]
pub fn srgb8_to_linear(value: u8) -> f32 {
    srgb8_transfer().decode(value)
}

/// Quantize linear light into an sRGB byte, round-to-nearest with positive
/// half-way values rounded up. Out-of-range values clamp; NaN maps to zero.
#[inline]
pub fn linear_to_srgb8(value: f32) -> u8 {
    srgb8_transfer().encode(value)
}

impl Srgb8Transfer {
    #[inline]
    pub fn decode(&self, value: u8) -> f32 {
        self.linear[value as usize]
    }

    /// Encode one sample using this batch's already-initialized transfer.
    #[inline]
    pub fn encode(&self, value: f32) -> u8 {
        let value = value.clamp(0., 1.);
        let mut candidate = self.lower[((value * BINS as f32) as usize).min(BINS - 1)] as usize;
        // The transfer is monotone. Correct using the unquantized input: no bin's
        // representative value is used as the final colour. These loops normally
        // take zero/one steps and remain correct at rounded bucket boundaries.
        while candidate < 255 && value >= self.cuts[candidate] {
            candidate += 1;
        }
        while candidate > 0 && value < self.cuts[candidate - 1] {
            candidate -= 1;
        }
        candidate as u8
    }
}

fn reference(value: f32) -> u8 {
    let value = value.clamp(0., 1.);
    let s = if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    };
    (s * 255.).round().clamp(0., 255.) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_srgb_byte_round_trips_and_dense_linear_samples_match_the_transfer() {
        for byte in 0..=255 {
            assert_eq!(linear_to_srgb8(srgb8_to_linear(byte)), byte);
        }
        for i in 0..=65536 {
            let v = i as f32 / 65536.;
            assert_eq!(linear_to_srgb8(v), reference(v), "{v}");
        }
        assert_eq!(linear_to_srgb8(-1.), 0);
        assert_eq!(linear_to_srgb8(2.), 255);
        assert_eq!(linear_to_srgb8(f32::NAN), 0);
    }
    #[test]
    fn byte_midpoint_neighbours_preserve_the_quantization_boundary() {
        for &cut in &srgb8_transfer().cuts {
            let bits = (cut as f32).to_bits();
            for i in bits - 8..=bits + 8 {
                let v = f32::from_bits(i);
                assert_eq!(linear_to_srgb8(v), reference(v), "{v}");
            }
        }
    }
}
