// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Bounded delimiter splitting of encoded bytes, without decoding or fonts.
//!
//! Delimiters are a byte set and repeated delimiters collapse. A delimiter is
//! examined at a character's first byte; a protected second byte cannot split
//! that character. The returned spans reference the immutable caller input.
//! `nul_bytes` records one NUL when a nonempty token meets a delimiter, and
//! ends with two further NULs. Thus a trailing delimiter leaves three NULs.
//! This explicit layout is useful for native multi-string buffers; it is not
//! an implicit text encoding or a replacement for Unicode word segmentation.
use anyhow::{ensure, Result};
use std::ops::Range;

pub const MAX_BYTE_SPLIT_INPUT: usize = 1024 * 1024;
pub const MAX_BYTE_SPLIT_OUTPUT: usize = MAX_BYTE_SPLIT_INPUT + 2;
pub const MAX_BYTE_SPLIT_TOKENS: usize = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteSplitBoundary {
    SingleByte,
    /// Protect a pair only for lead81..9F/E0..EF and trail40..7E/80..FC.
    /// F0..FC leads are deliberately not protected. This is a byte-boundary
    /// profile, not validation or normalization of an entire encoded string.
    ShiftJis,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteSplitLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_tokens: usize,
}
impl Default for ByteSplitLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: MAX_BYTE_SPLIT_INPUT,
            max_output_bytes: MAX_BYTE_SPLIT_OUTPUT,
            max_tokens: MAX_BYTE_SPLIT_TOKENS,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteSplitOutput {
    pub spans: Vec<Range<usize>>,
    pub nul_bytes: Vec<u8>,
}

/// Prepare an immutable result, or fail before publishing any output. The
/// output extent and token budget are checked before allocating `nul_bytes`.
/// `check` belongs to the caller's task/cancellation scope, never a global.
pub fn split_delimited_bytes(
    source: &[u8],
    delimiters: &[u8],
    boundary: ByteSplitBoundary,
    limits: ByteSplitLimits,
    check: &dyn Fn() -> Result<()>,
) -> Result<ByteSplitOutput> {
    check()?;
    ensure!(
        limits.max_input_bytes <= MAX_BYTE_SPLIT_INPUT
            && limits.max_output_bytes <= MAX_BYTE_SPLIT_OUTPUT
            && limits.max_tokens <= MAX_BYTE_SPLIT_TOKENS,
        "byte split configured budget"
    );
    ensure!(
        source.len() <= limits.max_input_bytes,
        "byte split input budget"
    );
    ensure!(delimiters.len() <= 256, "byte split delimiter budget");
    ensure!(!source.contains(&0), "byte split input contains NUL");
    let mut set = [false; 256];
    for byte in delimiters {
        set[usize::from(*byte)] = true;
    }
    let mut spans = Vec::new();
    let mut start = None;
    let mut at = 0;
    let mut next_check = 0;
    while at < source.len() {
        if at >= next_check {
            check()?;
            next_check = at + 4096;
        }
        let byte = source[at];
        if set[usize::from(byte)] {
            if let Some(begin) = start.take() {
                ensure!(spans.len() < limits.max_tokens, "byte split token budget");
                spans.push(begin..at);
            }
            at += 1;
        } else {
            start.get_or_insert(at);
            at += if boundary == ByteSplitBoundary::ShiftJis && shift_jis_pair(&source[at..]) {
                2
            } else {
                1
            };
        }
    }
    if let Some(begin) = start {
        ensure!(spans.len() < limits.max_tokens, "byte split token budget");
        spans.push(begin..source.len());
    }
    let mut bytes = 2usize;
    for span in &spans {
        bytes += span.len() + usize::from(span.end < source.len());
    }
    ensure!(
        bytes <= limits.max_output_bytes,
        "byte split output capacity"
    );
    check()?;
    let mut nul_bytes = Vec::with_capacity(bytes);
    for (index, span) in spans.iter().enumerate() {
        if index & 255 == 0 {
            check()?;
        }
        nul_bytes.extend_from_slice(&source[span.clone()]);
        if span.end < source.len() {
            nul_bytes.push(0);
        }
    }
    nul_bytes.extend_from_slice(&[0, 0]);
    debug_assert_eq!(nul_bytes.len(), bytes);
    check()?;
    Ok(ByteSplitOutput { spans, nul_bytes })
}

fn shift_jis_pair(bytes: &[u8]) -> bool {
    bytes.len() >= 2
        && matches!(bytes[0], 0x81..=0x9f | 0xe0..=0xef)
        && matches!(bytes[1], 0x40..=0x7e | 0x80..=0xfc)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn split(bytes: &[u8], separators: &[u8], boundary: ByteSplitBoundary) -> ByteSplitOutput {
        split_delimited_bytes(bytes, separators, boundary, Default::default(), &|| Ok(())).unwrap()
    }
    #[test]
    fn repeated_delimiters_collapse_and_native_nul_extent_is_explicit() {
        let output = split(b",a,,b,", b",", ByteSplitBoundary::SingleByte);
        assert_eq!(output.spans, vec![1..2, 4..5]);
        assert_eq!(output.nul_bytes, b"a\0b\0\0\0");
        assert_eq!(
            split(b"a,b", b",", ByteSplitBoundary::SingleByte).nul_bytes,
            b"a\0b\0\0"
        );
        assert_eq!(
            split(b"a,b", b"", ByteSplitBoundary::SingleByte).nul_bytes,
            b"a,b\0\0"
        );
        for source in [b"".as_slice(), b",,,".as_slice()] {
            let output = split(source, b",", ByteSplitBoundary::SingleByte);
            assert!(output.spans.is_empty());
            assert_eq!(output.nul_bytes, [0, 0]);
        }
    }
    #[test]
    fn shift_jis_boundary_protects_only_the_reviewed_lead_and_trail_ranges() {
        let bytes = [0x81, b'\\', b',', 0xef, 0xfc, b',', 0xf0, b'\\'];
        let output = split(&bytes, b",\\", ByteSplitBoundary::ShiftJis);
        assert_eq!(output.spans, vec![0..2, 3..5, 6..7]);
        assert_eq!(
            output.nul_bytes,
            [0x81, b'\\', 0, 0xef, 0xfc, 0, 0xf0, 0, 0, 0]
        );
        assert_eq!(
            split(&bytes, b",\\", ByteSplitBoundary::SingleByte).spans,
            vec![0..1, 3..5, 6..7]
        );
        assert_eq!(
            split(&[0x81, 0x7f, b'x'], &[0x7f], ByteSplitBoundary::ShiftJis).spans,
            vec![0..1, 2..3]
        );
        assert_eq!(
            split(&[0x81], b"", ByteSplitBoundary::ShiftJis).spans,
            vec![0..1]
        );
        // Delimiter membership precedes character-pair protection.
        assert_eq!(
            split(&[0x81, b'A'], &[0x81], ByteSplitBoundary::ShiftJis).spans,
            vec![1..2]
        );
    }
    #[test]
    fn budgets_and_cancellation_never_publish_a_partial_output() {
        let run = |source: &[u8], limits: ByteSplitLimits| {
            split_delimited_bytes(source, b",", ByteSplitBoundary::SingleByte, limits, &|| {
                Ok(())
            })
        };
        assert!(run(
            b"a,b",
            ByteSplitLimits {
                max_output_bytes: 4,
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("capacity"));
        assert!(run(
            b"a,b",
            ByteSplitLimits {
                max_tokens: 1,
                ..Default::default()
            }
        )
        .is_err());
        assert!(run(
            b"ab",
            ByteSplitLimits {
                max_input_bytes: 1,
                ..Default::default()
            }
        )
        .is_err());
        assert!(run(b"a\0b", Default::default()).is_err());
        let calls = std::cell::Cell::new(0);
        let check = || {
            let count = calls.get() + 1;
            calls.set(count);
            ensure!(count < 3, "cancelled split");
            Ok(())
        };
        assert!(split_delimited_bytes(
            &vec![b'x'; 8192],
            b",",
            ByteSplitBoundary::ShiftJis,
            Default::default(),
            &check
        )
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    }
}
