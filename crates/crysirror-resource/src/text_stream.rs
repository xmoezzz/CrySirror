// SPDX-License-Identifier: MPL-2.0
//! KiriKiri ciphered/compressed text streams (`FE FE <mode> FF FE`).
//!
//! Semantics follow krkrz `base/TextStream.cpp` (fd5c4ba, read path): mode 0 is
//! the per-character XOR, mode 1 swaps adjacent bits of each UTF-16 unit, and
//! mode 2 is zlib with 64-bit compressed/uncompressed sizes. Any other mode or
//! a missing inner UTF-16LE mark is an error. No VM is involved.
use anyhow::{Context, Result, bail, ensure};
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
            return Ok(Some(utf16(
                out.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]])),
            )?));
        }
        _ => bail!("unsupported cipher mode {mode}"),
    };
    for ch in &mut units {
        let c = *ch;
        *ch = if mode == 0 {
            if c >= 0x20 {
                c ^ (((c & 0xfe) << 8) ^ 1)
            } else {
                c
            }
        } else {
            ((c & 0xaaaa) >> 1) | ((c & 0x5555) << 1)
        };
    }
    Ok(Some(utf16(units.into_iter())?))
}

/// Original text stream output encoding. The caller owns save location and
/// atomic publication; this codec only produces bounded immutable bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextWriteMode {
    /// UTF-16 little endian preceded by its ordinary BOM.
    PlainUtf16,
    /// FE FE mode 1, swapping adjacent bits in each UTF-16 unit.
    Cipher1,
    /// FE FE mode 2 with zlib and 64-bit little endian encoded/raw byte lengths.
    Deflate {
        /// zlib level 0..9; level 6 is the original default.
        level: u8,
    },
}

/// Encode one whole text stream without a filesystem, VM or device. Format
/// follows tTVPTextWriteStream in krkrz fd5c4baa6a2ef5978db1bd043634351f48667daf
/// TextStream.cpp:364-634. Offset/update modes belong to storage policy.
pub fn encode(text: &str, mode: TextWriteMode, max_bytes: usize) -> Result<Vec<u8>> {
    let raw_len = text
        .encode_utf16()
        .count()
        .checked_mul(2)
        .context("text output byte length overflow")?;
    ensure!(raw_len <= max_bytes, "text output byte limit");
    let mut raw = Vec::with_capacity(raw_len);
    let mut cipher = mode == TextWriteMode::Cipher1;
    for unit in text.encode_utf16() {
        if unit == 0 {
            cipher = false;
        }
        let unit = if cipher {
            ((unit & 0xaaaa) >> 1) | ((unit & 0x5555) << 1)
        } else {
            unit
        };
        raw.extend_from_slice(&unit.to_le_bytes());
    }
    let mut output = match mode {
        TextWriteMode::PlainUtf16 => vec![0xff, 0xfe],
        TextWriteMode::Cipher1 => vec![0xfe, 0xfe, 1, 0xff, 0xfe],
        TextWriteMode::Deflate { level } => {
            ensure!(level <= 9, "invalid zlib compression level");
            let mut encoder = flate2::write::ZlibEncoder::new(
                Vec::new(),
                flate2::Compression::new(u32::from(level)),
            );
            std::io::Write::write_all(&mut encoder, &raw).context("text deflate")?;
            let encoded = encoder.finish().context("text deflate finish")?;
            ensure!(
                encoded
                    .len()
                    .checked_add(21)
                    .is_some_and(|n| n <= max_bytes),
                "encoded text output byte limit"
            );
            let mut output = vec![0xfe, 0xfe, 2, 0xff, 0xfe];
            output.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
            output.extend_from_slice(&(raw_len as u64).to_le_bytes());
            output.extend_from_slice(&encoded);
            return Ok(output);
        }
    };
    ensure!(
        output
            .len()
            .checked_add(raw.len())
            .is_some_and(|n| n <= max_bytes),
        "encoded text output byte limit"
    );
    output.extend_from_slice(&raw);
    Ok(output)
}

fn utf16(units: impl Iterator<Item = u16>) -> Result<String> {
    let units: Vec<u16> = units.collect();
    // The original stream ends at the first NUL terminator, if any.
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16(&units[..end]).context("invalid UTF-16 in ciphered text")
}

/// Decode a bounded legacy text resource: cipher/compressed streams and BOMs
/// take precedence, followed by valid UTF-8 and finally strict Shift_JIS.
/// This is a codec policy, independent of the syntax or VM consuming the text.
pub fn decode_auto(bytes: &[u8], max_bytes: usize) -> Result<String> {
    ensure!(bytes.len() <= max_bytes, "text stream byte limit");
    if let Some(text) = decode(bytes, max_bytes)? {
        return Ok(text);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        ensure!(rest.len() % 2 == 0, "odd UTF-16LE text length");
        let units: Vec<_> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).context("invalid UTF-16LE text");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8(rest.to_vec()).context("invalid UTF-8 text");
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }
    let (text, _, errors) = encoding_rs::SHIFT_JIS.decode(bytes);
    ensure!(!errors, "text is neither UTF-8 nor Shift_JIS");
    Ok(text.into_owned())
}

/// Owned, bounded text cursor accepting LF, CRLF and CR line endings.
/// It does not invent an extra empty line after a trailing separator. Blank
/// lines within the input remain visible and delivered lines are numbered
/// from one. The cursor owns its source independently of any VM or decoder.
#[derive(Clone, Debug, Default)]
pub struct LineCursor {
    text: String,
    offset: usize,
    line_number: usize,
}
impl LineCursor {
    /// Replace the source and reset the cursor, retaining the old source if
    /// the new text exceeds the caller's resource budget.
    pub fn reset(&mut self, text: String, max_bytes: usize) -> Result<()> {
        ensure!(text.len() <= max_bytes, "line cursor text byte limit");
        self.text = text;
        self.offset = 0;
        self.line_number = 0;
        Ok(())
    }
    /// Number of lines successfully delivered since the most recent reset.
    pub fn line_number(&self) -> usize {
        self.line_number
    }
    /// Drop the source. The delivered-line count remains available.
    pub fn clear(&mut self) {
        self.text.clear();
        self.offset = 0;
    }
    /// Return the next line without its separator, preserving Unicode text.
    pub fn next_line(&mut self) -> Option<&str> {
        if self.offset >= self.text.len() {
            return None;
        }
        let start = self.offset;
        let bytes = self.text.as_bytes();
        let end = bytes[start..]
            .iter()
            .position(|c| matches!(c, b'\r' | b'\n'))
            .map(|n| start + n)
            .unwrap_or(bytes.len());
        self.offset = end;
        if self.offset < bytes.len() {
            self.offset += 1;
            if bytes[end] == b'\r' && bytes.get(self.offset) == Some(&b'\n') {
                self.offset += 1;
            }
        }
        self.line_number += 1;
        Some(&self.text[start..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_text_policy_honors_boms_and_rejects_invalid_or_oversized_input() {
        assert_eq!(decode_auto(&[0xff, 0xfe, b'a', 0], 4).unwrap(), "a");
        assert_eq!(decode_auto(b"\xef\xbb\xbfhello", 8).unwrap(), "hello");
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode("日本");
        assert_eq!(decode_auto(&bytes, 64).unwrap(), "日本");
        assert!(decode_auto(&[0xff, 0xfe, 1], 64).is_err());
        assert!(decode_auto(b"\xef\xbb\xbf\xff", 64).is_err());
        assert!(decode_auto(b"hello", 4).is_err());
    }
    #[test]
    fn line_cursor_mixed_newlines_unicode_and_budget() {
        let mut cursor = LineCursor::default();
        cursor.reset("一\r\n\r二\n三".into(), 64).unwrap();
        assert_eq!(cursor.next_line(), Some("一"));
        assert_eq!(cursor.next_line(), Some(""));
        assert!(cursor.reset("too large".into(), 1).is_err());
        assert_eq!(cursor.next_line(), Some("二"));
        assert_eq!(cursor.next_line(), Some("三"));
        assert_eq!(cursor.next_line(), None);
        assert_eq!(cursor.line_number(), 4);
        cursor.reset("\n".into(), 1).unwrap();
        assert_eq!(cursor.next_line(), Some(""));
        assert_eq!(cursor.next_line(), None);
        cursor.clear();
        assert_eq!(cursor.line_number(), 1);
        cursor.reset(String::new(), 0).unwrap();
        assert_eq!(cursor.line_number(), 0);
        assert_eq!(cursor.next_line(), None);
    }
    fn encode(mode: u8, text: &str) -> Vec<u8> {
        let mut out = vec![0xfe, 0xfe, mode, 0xff, 0xfe];
        for c in text.encode_utf16() {
            let e = if mode == 0 {
                if c >= 0x20 {
                    c ^ (((c & 0xfe) << 8) ^ 1)
                } else {
                    c
                }
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
            assert_eq!(
                decode(&encode(mode, "var a = \"吉里吉里\";\n"), 1 << 20)
                    .unwrap()
                    .unwrap(),
                "var a = \"吉里吉里\";\n"
            );
        }
        let raw: Vec<u8> = "zlib テキスト"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
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

#[cfg(test)]
mod output_tests {
    use super::*;
    const TEXT: &str = "保存テキスト 😀\nvalue=42;\r\n";
    #[test]
    fn writer_preserves_original_bytes_lengths_unicode_and_limits() {
        assert_eq!(
            encode("A", TextWriteMode::PlainUtf16, 4).unwrap(),
            [0xff, 0xfe, 0x41, 0]
        );
        assert_eq!(
            encode("A", TextWriteMode::Cipher1, 7).unwrap(),
            [0xfe, 0xfe, 1, 0xff, 0xfe, 0x82, 0]
        );
        for mode in [
            TextWriteMode::PlainUtf16,
            TextWriteMode::Cipher1,
            TextWriteMode::Deflate { level: 0 },
            TextWriteMode::Deflate { level: 6 },
            TextWriteMode::Deflate { level: 9 },
        ] {
            let bytes = encode(TEXT, mode, 4096).unwrap();
            assert_eq!(decode_auto(&bytes, 4096).unwrap(), TEXT);
            if let TextWriteMode::Deflate { .. } = mode {
                assert_eq!(&bytes[..5], &[0xfe, 0xfe, 2, 0xff, 0xfe]);
                assert_eq!(
                    u64::from_le_bytes(bytes[5..13].try_into().unwrap()),
                    (bytes.len() - 21) as u64
                );
                assert_eq!(
                    u64::from_le_bytes(bytes[13..21].try_into().unwrap()),
                    (TEXT.encode_utf16().count() * 2) as u64
                );
            }
        }
        assert!(encode("A", TextWriteMode::PlainUtf16, 3).is_err());
        assert!(encode(TEXT, TextWriteMode::Deflate { level: 6 }, 4).is_err());
        assert!(encode(TEXT, TextWriteMode::Deflate { level: 10 }, 4096).is_err());
    }
    #[test]
    #[ignore = "requires CRYSIRROR_TEXT_WRITE_FIXTURES; exports authored codec bytes for independent Python zlib verification"]
    fn exports_authored_text_streams_for_external_zlib_reference() {
        let root = std::path::PathBuf::from(
            std::env::var_os("CRYSIRROR_TEXT_WRITE_FIXTURES").expect("explicit fixture directory"),
        );
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("expected.txt"), TEXT).unwrap();
        for (name, mode) in [
            ("plain.bin", TextWriteMode::PlainUtf16),
            ("c1.bin", TextWriteMode::Cipher1),
            ("z0.bin", TextWriteMode::Deflate { level: 0 }),
            ("z6.bin", TextWriteMode::Deflate { level: 6 }),
            ("z9.bin", TextWriteMode::Deflate { level: 9 }),
        ] {
            std::fs::write(root.join(name), encode(TEXT, mode, 4096).unwrap()).unwrap();
        }
    }
}
