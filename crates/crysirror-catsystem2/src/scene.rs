// SPDX-License-Identifier: MPL-2.0
//! Native CatScene line decoding; CST is distinct from KCS system bytecode.
use anyhow::{ensure, Context, Result};
use std::io::Read;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: u8,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Scene {
    pub lines: Vec<Line>,
    pub input_ranges: Vec<(u32, u32)>,
}
fn word(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .context("truncated CatScene word")?
            .try_into()?,
    ))
}
impl Scene {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() >= 16 && bytes.len() <= super::kcs::MAX_SCRIPT_BYTES,
            "CatScene source budget"
        );
        ensure!(&bytes[..8] == b"CatScene", "CatScene signature");
        let packed = word(bytes, 8)? as usize;
        let size = word(bytes, 12)? as usize;
        ensure!(
            (16..=super::kcs::MAX_SCRIPT_BYTES).contains(&size),
            "CatScene inflate budget"
        );
        let body = if packed == 0 {
            ensure!(bytes.len() == 16 + size, "CatScene plain size");
            bytes[16..].to_vec()
        } else {
            ensure!(bytes.len() == 16 + packed, "CatScene compressed size");
            let mut z = flate2::read::ZlibDecoder::new(&bytes[16..]);
            let mut out = Vec::new();
            z.by_ref().take(size as u64 + 1).read_to_end(&mut out)?;
            ensure!(
                out.len() == size && z.total_in() as usize == packed,
                "CatScene zlib size"
            );
            out
        };
        ensure!(
            word(&body, 0)? as usize == size - 16,
            "CatScene script extent"
        );
        let count = word(&body, 4)? as usize;
        let offsets = word(&body, 8)? as usize;
        let strings = word(&body, 12)? as usize;
        ensure!(
            count <= 65536
                && offsets == count * 8
                && strings >= offsets
                && (strings - offsets) % 4 == 0
                && 16 + strings <= size,
            "CatScene tables"
        );
        let n = (strings - offsets) / 4;
        ensure!(n <= 1_000_000, "CatScene line budget");
        let mut lines = Vec::new();
        let mut decoded_line_bytes = 0usize;
        for i in 0..n {
            let at = word(&body, 16 + offsets + i * 4)? as usize;
            let rest = body
                .get(16 + strings + at..)
                .context("CatScene line offset")?;
            ensure!(rest.len() >= 3 && rest[0] == 1, "CatScene line prefix");
            let end = rest[2..]
                .iter()
                .position(|b| *b == 0)
                .context("CatScene unterminated line")?;
            ensure!(end <= 65536, "CatScene string budget");
            decoded_line_bytes = decoded_line_bytes
                .checked_add(end)
                .context("CatScene cumulative string overflow")?;
            ensure!(
                decoded_line_bytes <= super::kcs::MAX_SCRIPT_BYTES,
                "CatScene cumulative string budget"
            );
            lines.push(Line {
                kind: rest[1],
                bytes: rest[2..2 + end].to_vec(),
            });
        }
        let mut input_ranges = Vec::new();
        for i in 0..count {
            let length = word(&body, 16 + i * 8)?;
            let index = word(&body, 20 + i * 8)?;
            ensure!(
                (index as usize) <= n && (length as usize) <= n - index as usize,
                "CatScene input range"
            );
            input_ranges.push((length, index));
        }
        Ok(Self {
            lines,
            input_ranges,
        })
    }
}
