// SPDX-License-Identifier: MPL-2.0
//! Numeric readers used by FCString. These residents link different Windows CRTs.
fn sign(bytes: &[u8]) -> (bool, &[u8]) {
    let raw = bytes.trim_ascii_start();
    match raw.split_first() {
        Some((b'-', tail)) => (true, tail),
        Some((b'+', tail)) => (false, tail),
        _ => (false, raw),
    }
}

fn wrapping_digits(digits: &[u8], radix: u32) -> u32 {
    digits
        .iter()
        .map_while(|&b| char::from(b).to_digit(radix))
        .fold(0u32, |n, d| n.wrapping_mul(radix).wrapping_add(d))
}

/// V1 RVA 0x9ae70. The bundled MSVCRT atoi accumulates in EAX with
/// two LEAs, then negates once (CRT RVAs 0x8fed–0x9024).
pub(super) fn integer(bytes: &[u8]) -> i32 {
    if let Some(hex) = bytes.strip_prefix(b"0x") {
        return wrapping_digits(hex, 16) as i32;
    }
    let (negative, digits) = sign(bytes);
    let n = wrapping_digits(digits, 10) as i32;
    if negative { n.wrapping_neg() } else { n }
}

/// 2011 RVA 0x14fcb0 uses custom binary and hexadecimal scanners, then
/// MSVCR80 atoi/atol/strtol for decimal. Decimal overflow saturates.
pub(super) fn integer2011(bytes: &[u8]) -> i32 {
    if let Some(hex) = bytes.strip_prefix(b"0x") {
        return wrapping_digits(hex, 16) as i32;
    }
    if let Some(bits) = bytes.strip_prefix(b"0b") {
        return wrapping_digits(bits, 2) as i32;
    }
    let (negative, digits) = sign(bytes);
    digits
        .iter()
        .map_while(|&b| char::from(b).to_digit(10))
        .fold(0i32, |n, d| {
            let n = n.saturating_mul(10);
            if negative {
                n.saturating_sub(d as i32)
            } else {
                n.saturating_add(d as i32)
            }
        })
}

/// RVA 0x14e230 passes the complete string to MSVCR80 strtoul.
/// Its base-2 scanner does not consume the "0b" prefix. On overflow
/// the CRT sets ULONG_MAX before applying the sign (0x4cfed–0x4d012).
pub(super) fn unsigned2011(raw: &[u8]) -> u32 {
    let hex = raw
        .strip_prefix(b"0x")
        .filter(|s| s.first().is_some_and(u8::is_ascii_hexdigit));
    let (negative, digits, base) = match hex {
        Some(digits) => (false, digits, 16),
        None => {
            let (negative, digits) = sign(raw);
            (negative, digits, 10)
        }
    };
    let value = digits
        .iter()
        .map_while(|&b| char::from(b).to_digit(base))
        .try_fold(0u32, |n, d| n.checked_mul(base)?.checked_add(d))
        .unwrap_or(u32::MAX);
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_crt_overflow_and_prefixes_are_profile_specific() {
        assert_eq!(integer(b"4294967297"), 1);
        assert_eq!(integer(b"-2147483649"), i32::MAX);
        assert_eq!(integer2011(b"4294967297"), i32::MAX);
        assert_eq!(integer2011(b"-2147483649"), i32::MIN);
        assert_eq!(integer2011(b"0x100000001"), 1);
        assert_eq!(integer(b"0b01110"), 0);
        assert_eq!(integer2011(b"0b01110"), 14);
        assert_eq!(unsigned2011(b"0b01110"), 0);
        assert_eq!(unsigned2011(b"0x100000001"), u32::MAX);
        assert_eq!(unsigned2011(b"-4294967297"), 1);
    }
}
