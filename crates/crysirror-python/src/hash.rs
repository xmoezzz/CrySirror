// SPDX-License-Identifier: MPL-2.0
//! Numeric hashes follow CPython (`sys.hash_info`: modulus 2**61 - 1) so that
//! equal ints, floats and bools share a hash. String hashing is deterministic
//! (CPython randomises it; no script can depend on the value).

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::ToPrimitive;

pub const MODULUS: u64 = (1u64 << 61) - 1;

fn fix(h: i64) -> i64 {
    if h == -1 { -2 } else { h }
}

pub fn hash_int(i: i64) -> i64 {
    let a = i.unsigned_abs() % MODULUS;
    let h = if i < 0 { -(a as i64) } else { a as i64 };
    fix(h)
}

pub fn hash_bigint(b: &BigInt) -> i64 {
    if let Some(i) = b.to_i64() {
        return hash_int(i);
    }
    let m = BigInt::from(MODULUS);
    let r = b.magnitude().clone() % m.magnitude();
    let a = r.to_u64().unwrap_or(0) as i64;
    fix(if b.sign() == Sign::Minus { -a } else { a })
}

pub fn hash_float(v: f64) -> i64 {
    if v.is_nan() {
        return 0;
    }
    if v.is_infinite() {
        return if v > 0.0 { 314159 } else { -314159 };
    }
    if v.fract() == 0.0 && v.abs() < 9.2e18 {
        return hash_int(v as i64);
    }
    // CPython _Py_HashDouble.
    let (mut m, mut e) = frexp(v.abs());
    let mut x: u64 = 0;
    while m != 0.0 {
        x = ((x << 28) & MODULUS) | (x >> (61 - 28));
        m *= 268435456.0;
        e -= 28;
        let y = m as u64;
        m -= y as f64;
        x += y;
        if x >= MODULUS {
            x -= MODULUS;
        }
    }
    let e = if e >= 0 { e % 61 } else { 61 - 1 - ((-1 - e) % 61) };
    x = ((x << e) & MODULUS) | (x >> (61 - e));
    let h = if v < 0.0 { -(x as i64) } else { x as i64 };
    fix(h)
}

pub fn frexp(v: f64) -> (f64, i32) {
    if v == 0.0 || !v.is_finite() {
        return (v, 0);
    }
    let bits = v.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        let (m, e) = frexp(v * (1u64 << 54) as f64);
        return (m, e - 54);
    }
    let e = exp - 1022;
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, e)
}

pub fn hash_str(s: &str) -> i64 {
    // FNV-1a over UTF-8 with a final avalanche.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    fix(h as i64)
}

pub fn hash_bytes(b: &[u8]) -> i64 {
    // Bytes hash like their latin-1 text in Python 2 semantics would differ;
    // a distinct seed keeps b"a" and "a" apart as in Python 3.
    let mut h: u64 = 0x84222325_cbf29ce4;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    fix((h ^ (h >> 29)) as i64)
}

pub fn hash_tuple(items: &[i64]) -> i64 {
    // CPython 3.8+ xxHash-based tuple hash.
    const P1: u64 = 11400714785074694791;
    const P2: u64 = 14029467366897019727;
    const P5: u64 = 2870177450012600261;
    let mut acc: u64 = P5;
    for h in items {
        acc = acc.wrapping_add((*h as u64).wrapping_mul(P2));
        acc = acc.rotate_left(31);
        acc = acc.wrapping_mul(P1);
    }
    acc = acc.wrapping_add(items.len() as u64 ^ (P5 ^ 3527539));
    if acc == u64::MAX {
        return 1546275796;
    }
    acc as i64
}

pub fn big_divmod(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    a.div_mod_floor(b)
}
