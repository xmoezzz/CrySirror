// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! Bounded text document encoding, independent of any VM or filesystem.
use crysirror_core::{Error, Result};
#[derive(Clone, Copy, Debug)]
pub enum TextEncoding {
    Utf8,
    Cp932,
}
#[derive(Clone, Copy, Debug)]
pub enum LineEnding {
    Lf,
    CrLf,
}
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Encode terminated lines without a BOM. Unsupported codepage characters are
/// rejected before storage admission, never silently replaced.
pub fn encode_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    encoding: TextEncoding,
    ending: LineEnding,
) -> Result<Vec<u8>> {
    let mut text = String::new();
    let newline = match ending {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
    };
    for line in lines {
        if text
            .len()
            .saturating_add(line.len())
            .saturating_add(newline.len())
            > MAX_TEXT_BYTES
        {
            return Err(Error::Limit("text document bytes"));
        }
        text.push_str(line);
        text.push_str(newline);
    }
    match encoding {
        TextEncoding::Utf8 => Ok(text.into_bytes()),
        TextEncoding::Cp932 => {
            let (bytes, _, bad) = encoding_rs::SHIFT_JIS.encode(&text);
            if bad {
                return Err(Error::Invalid("text is not representable in CP932"));
            }
            Ok(bytes.into_owned())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_bytes_and_rejection() {
        assert_eq!(
            encode_lines(["日本", "abc"], TextEncoding::Cp932, LineEnding::CrLf).unwrap(),
            b"\x93\xfa\x96\x7b\r\nabc\r\n"
        );
        assert_eq!(
            encode_lines(["日本"], TextEncoding::Utf8, LineEnding::Lf).unwrap(),
            "日本\n".as_bytes()
        );
        assert!(encode_lines(["😀"], TextEncoding::Cp932, LineEnding::Lf).is_err());
        assert!(
            encode_lines(
                ["x".repeat(MAX_TEXT_BYTES).as_str()],
                TextEncoding::Utf8,
                LineEnding::Lf
            )
            .is_err()
        );
    }
}
