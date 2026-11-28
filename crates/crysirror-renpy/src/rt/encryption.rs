// SPDX-License-Identifier: MPL-2.0
//! `renpy.encryption` (7.6/8.1+), after Ren'Py's `encryption.pyx` (MIT),
//! which wraps libhydrogen's `hydro_secretbox` (ISC, vendored upstream as
//! `module/libhydrogen`): a Gimli-based SIV authenticated encryption, used
//! by Ren'Py Sync with message id 0 and context `b"shirocat"`.
//!
//! The ciphertext is `siv (20) || mac (16) || encrypted message`; the IV
//! is random, so encryption is not deterministic.
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.encryption";
pub const KEYBYTES: usize = 32;
const IVBYTES: usize = 20;
const SIVBYTES: usize = 20;
const MACBYTES: usize = 16;
pub const HEADERBYTES: usize = SIVBYTES + MACBYTES;
const RATE: usize = 16;
const BLOCK: usize = 48;
const TAG_HEADER: u8 = 0x01;
const TAG_PAYLOAD: u8 = 0x02;
const TAG_FINAL: u8 = 0x08;
const TAG_FINAL0: u8 = 0xf8;
const TAG_KEY0: u8 = 0xfe;
const TAG_KEY: u8 = 0xff;
const DOMAIN_AEAD: u8 = 0x0;
const DOMAIN_XOF: u8 = 0xf;
const CONTEXT: &[u8; 8] = b"shirocat";

fn gimli(state: &mut [u32; 12]) {
    for round in (1..=24u32).rev() {
        for column in 0..4 {
            let x = state[column].rotate_left(24);
            let y = state[4 + column].rotate_left(9);
            let z = state[8 + column];
            state[8 + column] = x ^ (z << 1) ^ ((y & z) << 2);
            state[4 + column] = y ^ x ^ ((x | z) << 1);
            state[column] = z ^ y ^ ((x & y) << 3);
        }
        match round & 3 {
            0 => {
                state.swap(0, 1);
                state.swap(2, 3);
                state[0] ^= 0x9e37_7900 | round;
            }
            2 => {
                state.swap(0, 2);
                state.swap(1, 3);
            }
            _ => {}
        }
    }
}

fn core(buf: &mut [u8; BLOCK], tag: u8) {
    buf[BLOCK - 1] ^= tag;
    let mut s = [0u32; 12];
    for (i, w) in s.iter_mut().enumerate() {
        *w = u32::from_le_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap());
    }
    gimli(&mut s);
    for (i, w) in s.iter().enumerate() {
        buf[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
}

fn pad(buf: &mut [u8; BLOCK], pos: usize, domain: u8) {
    buf[pos] ^= (domain << 1) | 1;
    buf[RATE - 1] ^= 0x80;
}

fn xor(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d ^= s;
    }
}

fn setup(buf: &mut [u8; BLOCK], msg_id: u64, ctx: &[u8; 8], key: &[u8; KEYBYTES], iv: &[u8], key_tag: u8) {
    const PREFIX: [u8; 8] = [6, b's', b'b', b'x', b'2', b'5', b'6', 8];
    *buf = [0; BLOCK];
    buf[..8].copy_from_slice(&PREFIX);
    buf[8..16].copy_from_slice(ctx);
    core(buf, TAG_HEADER);
    xor(&mut buf[..RATE], &key[..RATE]);
    core(buf, key_tag);
    xor(&mut buf[..RATE], &key[RATE..]);
    core(buf, key_tag);
    buf[0] ^= IVBYTES as u8;
    xor(&mut buf[1..RATE], &iv[..RATE - 1]);
    core(buf, TAG_HEADER);
    xor(&mut buf[..IVBYTES - (RATE - 1)], &iv[RATE - 1..IVBYTES]);
    xor(&mut buf[IVBYTES - RATE..IVBYTES - RATE + 8], &msg_id.to_le_bytes());
    core(buf, TAG_HEADER);
}

fn finalize(buf: &mut [u8; BLOCK], key: &[u8; KEYBYTES], tag: u8) {
    xor(&mut buf[RATE..], key);
    core(buf, tag);
    xor(&mut buf[RATE..], key);
    core(buf, tag);
}

/// `hydro_secretbox_encrypt_iv`.
pub fn encrypt_iv(m: &[u8], msg_id: u64, ctx: &[u8; 8], key: &[u8; KEYBYTES], iv: &[u8; IVBYTES]) -> Vec<u8> {
    let mut buf = [0u8; BLOCK];
    setup(&mut buf, msg_id, ctx, key, iv, TAG_KEY0);
    let mut chunks = m.chunks_exact(RATE);
    for c in &mut chunks {
        xor(&mut buf[..RATE], c);
        core(&mut buf, TAG_PAYLOAD);
    }
    let rest = chunks.remainder();
    xor(&mut buf[..rest.len()], rest);
    pad(&mut buf, rest.len(), DOMAIN_XOF);
    core(&mut buf, TAG_PAYLOAD);
    finalize(&mut buf, key, TAG_FINAL0);
    let mut out = vec![0u8; HEADERBYTES + m.len()];
    let siv: [u8; SIVBYTES] = buf[RATE..RATE + SIVBYTES].try_into().unwrap();
    out[..SIVBYTES].copy_from_slice(&siv);

    setup(&mut buf, msg_id, ctx, key, &siv, TAG_KEY);
    let ct = &mut out[HEADERBYTES..];
    let mut i = 0;
    while i + RATE <= m.len() {
        for k in 0..RATE {
            ct[i + k] = m[i + k] ^ buf[k];
        }
        buf[..RATE].copy_from_slice(&ct[i..i + RATE]);
        core(&mut buf, TAG_PAYLOAD);
        i += RATE;
    }
    let left = m.len() - i;
    for k in 0..left {
        ct[i + k] = m[i + k] ^ buf[k];
    }
    buf[..left].copy_from_slice(&ct[i..i + left]);
    pad(&mut buf, left, DOMAIN_AEAD);
    core(&mut buf, TAG_PAYLOAD);
    finalize(&mut buf, key, TAG_FINAL);
    out[SIVBYTES..HEADERBYTES].copy_from_slice(&buf[RATE..RATE + MACBYTES]);
    out
}

/// `hydro_secretbox_decrypt`: None when the MAC does not verify.
pub fn decrypt(c: &[u8], msg_id: u64, ctx: &[u8; 8], key: &[u8; KEYBYTES]) -> Option<Vec<u8>> {
    if c.len() < HEADERBYTES {
        return None;
    }
    let (siv, mac, ct) = (&c[..SIVBYTES], &c[SIVBYTES..HEADERBYTES], &c[HEADERBYTES..]);
    let mut buf = [0u8; BLOCK];
    setup(&mut buf, msg_id, ctx, key, siv, TAG_KEY);
    let mut m = vec![0u8; ct.len()];
    let mut i = 0;
    while i + RATE <= ct.len() {
        for k in 0..RATE {
            m[i + k] = ct[i + k] ^ buf[k];
        }
        buf[..RATE].copy_from_slice(&ct[i..i + RATE]);
        core(&mut buf, TAG_PAYLOAD);
        i += RATE;
    }
    let left = ct.len() - i;
    for k in 0..left {
        m[i + k] = ct[i + k] ^ buf[k];
    }
    buf[..left].copy_from_slice(&ct[i..]);
    pad(&mut buf, left, DOMAIN_AEAD);
    core(&mut buf, TAG_PAYLOAD);
    finalize(&mut buf, key, TAG_FINAL);
    let diff = buf[RATE..RATE + MACBYTES].iter().zip(mac).fold(0u8, |a, (x, y)| a | (x ^ y));
    (diff == 0).then_some(m)
}

fn key_arg(vm: &mut Vm, v: &Value) -> PyResult<[u8; KEYBYTES]> {
    let b: Vec<u8> = ops::bytes_of(v).ok_or_else(|| vm.type_error("key must be bytes"))?.to_vec();
    <[u8; KEYBYTES]>::try_from(b.as_slice()).map_err(|_| vm.value_error(format!("Key must be {KEYBYTES} bytes")))
}

fn f_encrypt(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let m = py::arg_or(args, kw, 0, "message", Value::None);
    let k = py::arg_or(args, kw, 1, "key", Value::None);
    let m = ops::bytes_of(&m).ok_or_else(|| vm.type_error("message must be bytes"))?.to_vec();
    let key = key_arg(vm, &k)?;
    let mut iv = [0u8; IVBYTES];
    getrandom::fill(&mut iv).map_err(|_| vm.value_error("Encryption failed"))?;
    Ok(vm.types.new_bytes(encrypt_iv(&m, 0, CONTEXT, &key, &iv)))
}

fn f_decrypt(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = py::arg_or(args, kw, 0, "cyphertext", Value::None);
    let k = py::arg_or(args, kw, 1, "key", Value::None);
    let c = ops::bytes_of(&c).ok_or_else(|| vm.type_error("cyphertext must be bytes"))?.to_vec();
    let key = key_arg(vm, &k)?;
    match decrypt(&c, 0, CONTEXT, &key) {
        Some(m) => Ok(vm.types.new_bytes(m)),
        None => Err(vm.value_error("Decryption failed")),
    }
}

fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    if !super::state::rt(vm).profile.since_pair(6, 1) {
        return Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{MODULE}'")));
    }
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[("secretbox_encrypt", f_encrypt), ("secretbox_decrypt", f_decrypt)],
    );
    crysirror_python::modules::set(vm, &m, "SECRETBOX_KEYBYTES", Value::Int(KEYBYTES as i64));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// libhydrogen's `hydro_secretbox_encrypt_iv` (upstream module/libhydrogen,
    /// compiled from the 8.3.7 sources) on a fixed key, IV and message.
    #[test]
    fn matches_libhydrogen() {
        let key: [u8; KEYBYTES] = std::array::from_fn(|i| (i * 7 + 1) as u8);
        let iv: [u8; IVBYTES] = std::array::from_fn(|i| (i * 13 + 5) as u8);
        let m: Vec<u8> = (0..40).collect();
        let hex: String = encrypt_iv(&m, 0, CONTEXT, &key, &iv).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "55c0cf232054410ad7a7f20b5f7092f09d3662fdfc41e57f8015c0d1573c309c558bfd75766b00f25f320f21b6ce4928924fad4c578bb2815ded34d6babb35a76f397e7f1c4509cc5c8ec319"
        );
    }

    #[test]
    fn round_trip_and_tamper() {
        let key = [7u8; KEYBYTES];
        let iv = [3u8; IVBYTES];
        for len in [0, 1, 15, 16, 17, 100] {
            let m: Vec<u8> = (0..len as u8).collect();
            let c = encrypt_iv(&m, 0, CONTEXT, &key, &iv);
            assert_eq!(c.len(), HEADERBYTES + len);
            assert_eq!(decrypt(&c, 0, CONTEXT, &key).as_deref(), Some(&m[..]));
            let mut bad = c.clone();
            *bad.last_mut().unwrap_or(&mut 0) ^= 1;
            if len > 0 {
                assert!(decrypt(&bad, 0, CONTEXT, &key).is_none());
            }
        }
    }
}
