// SPDX-License-Identifier: MPL-2.0
//! Ring-dictionary LZSS used by many legacy archive formats. Engine-owned so
//! any VM or archive plugin can request it through `TransformSpec`.
use crate::{read_all, ByteTransform, DecodeContext, TransformSpec};
use anyhow::{ensure, Context, Result};
use crysirror_core::service::AssetReader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// `bytewise`: 8-flag control byte (LSB first, 1 = literal), 12-bit position
/// and 4-bit length+3 in two bytes over a 4 KiB ring. `bitwise`: MSB-first bit
/// stream, flag bit 1 = literal, configurable position/length widths.
/// Truncated input stops decoding and the remainder stays zero, which is the
/// behavior legacy decoders rely on for archives with padded size fields.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LzssMode {
    Bytewise,
    Bitwise,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LzssParams {
    /// Byte-control or MSB-first bitstream layout.
    pub mode: LzssMode,
    /// Initial ring write position, wrapped to dictionary size.
    pub initial: usize,
    /// Dictionary position width; bytewise requires 12.
    pub position_bits: u32,
    /// Match length width; bytewise requires 4.
    pub size_bits: u32,
    /// Minimum match length; bytewise requires 3.
    pub min_match: usize,
    /// Bytewise only: the 4-bit length field is stored inverted (GLib).
    pub invert_length: bool,
}
impl Default for LzssParams {
    fn default() -> Self {
        Self {
            mode: LzssMode::Bytewise,
            initial: 0xFEE,
            position_bits: 12,
            size_bits: 4,
            min_match: 3,
            invert_length: false,
        }
    }
}
pub fn lzss_decompress(input: &[u8], size: usize, params: &LzssParams) -> Result<Vec<u8>> {
    match params.mode {
        LzssMode::Bytewise => {
            ensure!(
                params.position_bits == 12 && params.size_bits == 4 && params.min_match == 3,
                "bytewise LZSS requires position_bits=12, size_bits=4, min_match=3"
            );
            Ok(bytewise(input, size, params.initial, params.invert_length))
        }
        LzssMode::Bitwise => bitwise(input, size, params),
    }
}
fn bytewise(input: &[u8], size: usize, initial: usize, invert: bool) -> Vec<u8> {
    let mut dict = [0u8; 0x1000];
    let mut pos = initial & 0xFFF;
    let mut out = vec![0u8; size];
    let (mut at, mut ptr) = (0usize, 0usize);
    let mut control = 0u16;
    while at < size {
        control >>= 1;
        if control & 0x100 == 0 {
            let Some(&b) = input.get(ptr) else { break };
            ptr += 1;
            control = u16::from(b) | 0xFF00;
        }
        if control & 1 != 0 {
            let Some(&b) = input.get(ptr) else { break };
            ptr += 1;
            out[at] = b;
            at += 1;
            dict[pos] = b;
            pos = (pos + 1) & 0xFFF;
        } else {
            if input.len() < ptr + 2 {
                break;
            }
            let (lo, hi) = (usize::from(input[ptr]), usize::from(input[ptr + 1]));
            ptr += 2;
            let mut src = lo | ((hi & 0xF0) << 4);
            let n = if invert { !hi & 0xF } else { hi & 0xF };
            for _ in 0..(n + 3).min(size - at) {
                let b = dict[src];
                src = (src + 1) & 0xFFF;
                out[at] = b;
                at += 1;
                dict[pos] = b;
                pos = (pos + 1) & 0xFFF;
            }
        }
    }
    out
}
fn bitwise(input: &[u8], size: usize, p: &LzssParams) -> Result<Vec<u8>> {
    ensure!(
        (1..=16).contains(&p.position_bits) && (1..=16).contains(&p.size_bits),
        "LZSS bit widths must be 1..=16"
    );
    ensure!(
        !p.invert_length,
        "invert_length applies only to bytewise LZSS"
    );
    ensure!(
        p.min_match <= usize::MAX - (1usize << p.size_bits),
        "LZSS match length overflow"
    );
    let mask = (1usize << p.position_bits) - 1;
    let mut dict = vec![0u8; mask + 1];
    let mut pos = p.initial & mask;
    let mut out = vec![0u8; size];
    let mut at = 0usize;
    let mut bit = 0usize;
    let total = input.len().saturating_mul(8);
    let mut get = |n: u32| -> Option<usize> {
        if bit + n as usize > total {
            return None;
        }
        let mut v = 0usize;
        for _ in 0..n {
            v = (v << 1) | usize::from((input[bit / 8] >> (7 - bit % 8)) & 1);
            bit += 1;
        }
        Some(v)
    };
    while at < size {
        let Some(flag) = get(1) else { break };
        if flag != 0 {
            let Some(b) = get(8) else { break };
            out[at] = b as u8;
            at += 1;
            dict[pos] = b as u8;
            pos = (pos + 1) & mask;
        } else {
            let (Some(mut src), Some(n)) = (get(p.position_bits), get(p.size_bits)) else {
                break;
            };
            for _ in 0..(n + p.min_match).min(size - at) {
                let b = dict[src];
                src = (src + 1) & mask;
                out[at] = b;
                at += 1;
                dict[pos] = b;
                pos = (pos + 1) & mask;
            }
        }
    }
    Ok(out)
}
pub(crate) struct Lzss;
impl ByteTransform for Lzss {
    fn apply(
        &self,
        spec: &TransformSpec,
        input: Arc<dyn AssetReader>,
        _: &serde_json::Value,
        context: &DecodeContext,
    ) -> Result<Arc<dyn AssetReader>> {
        ensure!(spec.key_id.is_none(), "lzss takes no key");
        let params: LzssParams = if spec.parameters.is_null() {
            LzssParams::default()
        } else {
            serde_json::from_value(spec.parameters.clone()).context("lzss parameters")?
        };
        let size = spec.expected_size.context("lzss requires expected_size")?;
        let data = read_all(input.as_ref(), context.limits.max_resident_bytes)?;
        context.decode_with(size, || lzss_decompress(&data, size as usize, &params))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_matches_obey_both_native_layouts_and_reject_inapplicable_options() {
        let bytewise = LzssParams::default();
        assert_eq!(
            lzss_decompress(&[1, b'A', 0xee, 0xf0], 4, &bytewise).unwrap(),
            b"AAAA"
        );
        let bits = "1010000010111011110001";
        let mut packed = vec![0; bits.len().div_ceil(8)];
        for (i, b) in bits.bytes().enumerate() {
            packed[i / 8] |= (b - b'0') << (7 - i % 8);
        }
        let bitwise = LzssParams {
            mode: LzssMode::Bitwise,
            position_bits: 8,
            size_bits: 4,
            min_match: 2,
            initial: 239,
            ..bytewise
        };
        assert_eq!(lzss_decompress(&packed, 4, &bitwise).unwrap(), b"AAAA");
        assert!(lzss_decompress(
            &[],
            1,
            &LzssParams {
                min_match: 2,
                ..bytewise
            }
        )
        .is_err());
        assert!(lzss_decompress(
            &[],
            1,
            &LzssParams {
                invert_length: true,
                ..bitwise
            }
        )
        .is_err());
        assert!(lzss_decompress(
            &[],
            1,
            &LzssParams {
                min_match: usize::MAX,
                ..bitwise
            }
        )
        .is_err());
    }
}
