// SPDX-License-Identifier: MPL-2.0
//! KiriKiri ciphered/compressed text streams (`FE FE <mode> FF FE`).
//!
//! Semantics follow krkrz `base/TextStream.cpp` (fd5c4ba, read path): mode 0 is
//! the per-character XOR, mode 1 swaps adjacent bits of each UTF-16 unit, and
//! mode 2 is zlib with 64-bit compressed/uncompressed sizes. Any other mode or
//! a missing inner UTF-16LE mark is an error. No VM is involved.
use anyhow::{bail, ensure, Context, Result};
use std::io::Read;

/// Header of a ciphered or compressed stream.
pub const MARK: [u8; 2] = [0xfe, 0xfe];

/// `Some(text)` for a ciphered/compressed stream, `None` for any other bytes.
pub fn decode(bytes: &[u8], max_bytes: usize) -> Result<Option<String>> {
    if !bytes.starts_with(&MARK) {
        return Ok(None);
    }
    ensure!(bytes.len() >= 5, "truncated ciphered text header");
    let mode = bytes[2];
    ensure!(bytes[3..5] == [0xff, 0xfe], "unsupported cipher mode");
    let body = &bytes[5..];
    let mut units: Vec<u16> = match mode {
        0 | 1 => {
            ensure!(body.len() % 2 == 0, "odd ciphered text length");
            ensure!(body.len() <= max_bytes, "ciphered text byte limit");
            body.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect()
        }
        2 => {
            ensure!(body.len() >= 16, "truncated compressed text header");
            let compressed = u64::from_le_bytes(body[..8].try_into()?);
            let uncompressed = u64::from_le_bytes(body[8..16].try_into()?);
            ensure!(
                uncompressed <= max_bytes as u64 && uncompressed % 2 == 0,
                "compressed text byte limit"
            );
            let data = body
                .get(16..)
                .and_then(|d| d.get(..usize::try_from(compressed).ok()?))
                .context("truncated compressed text")?;
            let mut out = Vec::with_capacity(uncompressed as usize);
            flate2::read::ZlibDecoder::new(data)
                .take(uncompressed + 1)
                .read_to_end(&mut out)
                .context("compressed text inflate")?;
            ensure!(out.len() as u64 == uncompressed, "compressed text size");
            return Ok(Some(utf16(out.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])))?));
        }
        _ => bail!("unsupported cipher mode {mode}"),
    };
    for ch in &mut units {
        let c = *ch;
        *ch = if mode == 0 {
            if c >= 0x20 { c ^ (((c & 0xfe) << 8) ^ 1) } else { c }
        } else {
            ((c & 0xaaaa) >> 1) | ((c & 0x5555) << 1)
        };
    }
    Ok(Some(utf16(units.into_iter())?))
}

fn utf16(units: impl Iterator<Item = u16>) -> Result<String> {
    let units: Vec<u16> = units.collect();
    // The original stream ends at the first NUL terminator, if any.
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16(&units[..end]).context("invalid UTF-16 in ciphered text")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encode(mode: u8, text: &str) -> Vec<u8> {
        let mut out = vec![0xfe, 0xfe, mode, 0xff, 0xfe];
        for c in text.encode_utf16() {
            let e = if mode == 0 {
                if c >= 0x20 { c ^ (((c & 0xfe) << 8) ^ 1) } else { c }
            } else {
                ((c & 0xaaaa) >> 1) | ((c & 0x5555) << 1)
            };
            out.extend_from_slice(&e.to_le_bytes());
        }
        out
    }
    #[test]
    fn modes_round_trip_and_reject_unknown() {
        for mode in [0, 1] {
            assert_eq!(decode(&encode(mode, "var a = \"吉里吉里\";\n"), 1 << 20).unwrap().unwrap(), "var a = \"吉里吉里\";\n");
        }
        let raw: Vec<u8> = "zlib テキスト".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut z, &raw).unwrap();
        let z = z.finish().unwrap();
        let mut s = vec![0xfe, 0xfe, 2, 0xff, 0xfe];
        s.extend_from_slice(&(z.len() as u64).to_le_bytes());
        s.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        s.extend_from_slice(&z);
        assert_eq!(decode(&s, 1 << 20).unwrap().unwrap(), "zlib テキスト");
        assert!(decode(&s, 4).is_err());
        assert!(decode(&[0xfe, 0xfe, 3, 0xff, 0xfe], 64).is_err());
        assert!(decode(b"plain", 64).unwrap().is_none());
    }
}
