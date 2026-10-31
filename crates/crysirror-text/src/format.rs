// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Bounded first-compatible byte formatting, independent of any VM or font.
//! ASCII format syntax does not reinterpret surrounding encoded text. This
//! deliberately differs from printf's escaping/multiple-argument contract.
use anyhow::{bail, ensure, Result};
pub const MAX_BYTE_FORMAT_BYTES: usize = 1024 * 1024;
pub const MAX_BYTE_FORMAT_FIELD: usize = 510;
#[derive(Clone, Copy, Debug)]
pub enum ByteFormatValue<'a> {
    Integer(i32),
    Bytes(&'a [u8]),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteFormatOutput {
    pub bytes: Vec<u8>,
    /// Exactly one compatible placeholder consumed this argument.
    pub matched: bool,
}
#[derive(Clone, Copy)]
enum Kind {
    Integer,
    Bytes,
    Float,
    OtherInteger,
}
fn number(template: &[u8], at: &mut usize) -> Result<usize> {
    let mut n = 0usize;
    while let Some(b'0'..=b'9') = template.get(*at) {
        n = n
            .checked_mul(10)
            .and_then(|v| v.checked_add(usize::from(template[*at] - b'0')))
            .filter(|v| *v <= MAX_BYTE_FORMAT_FIELD)
            .ok_or_else(|| anyhow::anyhow!("byte format field budget"))?;
        *at += 1;
    }
    Ok(n)
}
/// Replace the first placeholder compatible with the value, preserving suffix
/// bytes for a later call. Supports `%d` and `%s`, decimal width and zero-padding.
/// Integer and string precision are parsed but ignored in this explicit legacy
/// profile. String width counts declared bytes, including embedded NUL values;
/// zero-padding does not affect strings. `%D/%c/%b` for integers and unknown
/// syntax fail visibly; known type-incompatible specifiers remain unchanged.
/// Embedded NUL in the template is outside this profile. Output is immutable
/// and all bounds are checked before allocation/publication.
pub fn format_one_bytes(template: &[u8], value: ByteFormatValue<'_>) -> Result<ByteFormatOutput> {
    ensure!(
        template.len() <= MAX_BYTE_FORMAT_BYTES,
        "byte format template budget"
    );
    ensure!(
        !template.contains(&0),
        "byte format template NUL profile unsupported"
    );
    for start in 0..template.len() {
        if template[start] != b'%' {
            continue;
        }
        let mut at = start + 1;
        let mut width = 0;
        let mut zero = false;
        let (kind, code) = loop {
            match template.get(at).copied() {
                Some(b'0') => {
                    zero = true;
                    at += 1;
                }
                Some(b'1'..=b'9') => {
                    width = number(template, &mut at)?;
                }
                Some(b'.') => {
                    at += 1;
                    let _precision = number(template, &mut at)?;
                }
                Some(c @ (b'd' | b'D')) => break (Kind::Integer, c),
                Some(c @ b's') => break (Kind::Bytes, c),
                Some(c @ (b'f' | b'F')) => break (Kind::Float, c),
                Some(c @ (b'c' | b'b')) => break (Kind::OtherInteger, c),
                // A second percent is examined again by the outer scan. It is
                // not treated as printf's %% escape in this profile.
                Some(b'%') => break (Kind::Float, b'%'),
                _ => bail!("unsupported byte format syntax"),
            }
        };
        let compatible = matches!(
            (kind, value),
            (
                Kind::Integer | Kind::OtherInteger,
                ByteFormatValue::Integer(_)
            ) | (Kind::Bytes, ByteFormatValue::Bytes(_))
        );
        if !compatible {
            continue;
        }
        ensure!(
            matches!(code, b'd' | b's'),
            "unsupported byte format conversion"
        );
        let number = match value {
            ByteFormatValue::Integer(v) => Some(v.to_string()),
            ByteFormatValue::Bytes(_) => None,
        };
        let raw = match value {
            ByteFormatValue::Integer(_) => number.as_ref().unwrap().as_bytes(),
            ByteFormatValue::Bytes(v) => v,
        };
        let pad = width.saturating_sub(raw.len());
        let suffix = &template[at + 1..];
        let length = start
            .checked_add(raw.len())
            .and_then(|n| n.checked_add(pad))
            .and_then(|n| n.checked_add(suffix.len()))
            .filter(|n| *n <= MAX_BYTE_FORMAT_BYTES)
            .ok_or_else(|| anyhow::anyhow!("byte format output budget"))?;
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(&template[..start]);
        if zero && matches!(value, ByteFormatValue::Integer(_)) {
            let negative = raw.first() == Some(&b'-');
            if negative {
                bytes.push(b'-');
            }
            bytes.resize(bytes.len() + pad, b'0');
            bytes.extend_from_slice(&raw[usize::from(negative)..]);
        } else {
            bytes.resize(bytes.len() + pad, b' ');
            bytes.extend_from_slice(&raw);
        }
        bytes.extend_from_slice(suffix);
        return Ok(ByteFormatOutput {
            bytes,
            matched: true,
        });
    }
    Ok(ByteFormatOutput {
        bytes: template.to_vec(),
        matched: false,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integer_profile_matches_reviewed_native_format_and_snprintf_cases() {
        // xsystem4 a85467d format.c + libsys4 b14839f int_to_cstr:
        // only the first compatible placeholder changes; .precision is ignored.
        for (template, value, expected) in [
            ("mixer%02d", 3, "mixer03"),
            ("%05d", -7, "-0007"),
            ("%5.1d", -7, "   -7"),
            ("%d", i32::MIN, "-2147483648"),
            ("%f_%03d_tail%d", 7, "%f_007_tail%d"),
            ("%%d", 3, "%3"),
        ] {
            let got =
                format_one_bytes(template.as_bytes(), ByteFormatValue::Integer(value)).unwrap();
            assert!(got.matched);
            assert_eq!(got.bytes, expected.as_bytes());
        }
    }
    #[test]
    fn byte_strings_keep_encoding_byte_width_and_unmatched_argument_identity() {
        let result =
            format_one_bytes(b"\x82\xa0[%5s]:%d", ByteFormatValue::Bytes(b"\x82\xa2")).unwrap();
        assert_eq!(result.bytes, b"\x82\xa0[   \x82\xa2]:%d");
        assert!(result.matched);
        let result = format_one_bytes(b"unchanged%d", ByteFormatValue::Bytes(b"value")).unwrap();
        assert!(!result.matched);
        assert_eq!(result.bytes, b"unchanged%d");
    }
    #[test]
    fn unsupported_conversion_and_resource_bounds_fail_before_output() {
        for source in [
            b"%D".as_slice(),
            b"%b",
            b"%c",
            b"%x",
            b"%511d",
            b"%.511d",
            b"NUL\0%d",
        ] {
            assert!(
                format_one_bytes(source, ByteFormatValue::Integer(3)).is_err(),
                "{source:?}"
            );
        }
        let source = [vec![b'a'; MAX_BYTE_FORMAT_BYTES - 2], b"%s".to_vec()].concat();
        assert!(format_one_bytes(&source, ByteFormatValue::Bytes(b"abc")).is_err());
        assert!(format_one_bytes(
            &vec![b'a'; MAX_BYTE_FORMAT_BYTES + 1],
            ByteFormatValue::Integer(1)
        )
        .is_err());
    }
}
