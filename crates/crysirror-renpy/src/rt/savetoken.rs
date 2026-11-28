// SPDX-License-Identifier: MPL-2.0
//! `renpy.savetoken` (8.1+), after Ren'Py 8.3 `savetoken.py` (MIT): save
//! and persistent signatures. Keys and signatures use python-ecdsa's
//! defaults: NIST P-256, SHA-1 message digest, raw `r || s` signatures,
//! SEC1 DER signing keys and SubjectPublicKeyInfo DER verifying keys,
//! base64 in `security_keys.txt` lines. Signatures are deterministic
//! (RFC 6979) where python-ecdsa draws a random nonce; both verify alike.
//!
//! The token directory is `tokens/` in the game's Save mount: CrySirror
//! scopes save storage per game, so keys are not shared between games as
//! upstream's per-user token directory is.
use super::{
    py::{self, *},
    savestore::{self, SaveStore},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw};
use p256::ecdsa::{
    Signature, SigningKey, VerifyingKey,
    signature::hazmat::{PrehashSigner, PrehashVerifier},
};
use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use sha1::{Digest, Sha1};
use std::{cell::RefCell, rc::Rc};

const MODULE: &str = "renpy.savetoken";
const TOKEN_DIR: &str = "tokens";

#[derive(Default)]
pub struct Tokens {
    pub active: bool,
    pub signing: Vec<SigningKey>,
    pub verifying: Vec<Vec<u8>>,
    pub should_upgrade: bool,
}

fn tokens(vm: &mut Vm) -> Rc<RefCell<Tokens>> {
    if let Some(t) = vm.ext_get::<Rc<RefCell<Tokens>>>() {
        return t.clone();
    }
    let t: Rc<RefCell<Tokens>> = Rc::default();
    vm.ext_set(t.clone());
    t
}

pub fn encode_line(key: &str, a: &[u8], b: Option<&[u8]>) -> String {
    match b {
        None => format!("{key} {}\n", B64.encode(a)),
        Some(b) => format!("{key} {} {}\n", B64.encode(a), B64.encode(b)),
    }
}

pub fn decode_line(line: &str) -> (String, Vec<u8>, Option<Vec<u8>>) {
    let line = line.trim();
    let none = (String::new(), Vec::new(), None);
    if line.is_empty() || line.starts_with('#') {
        return none;
    }
    let parts: Vec<&str> = line
        .splitn(3, char::is_whitespace)
        .filter(|p| !p.is_empty())
        .collect();
    let dec = |s: &str| B64.decode(s.trim()).ok();
    match parts.as_slice() {
        [k, a] => match dec(a) {
            Some(a) => (k.to_string(), a, None),
            None => none,
        },
        [k, a, b] => match (dec(a), dec(b)) {
            (Some(a), Some(b)) => (k.to_string(), a, Some(b)),
            _ => none,
        },
        _ => none,
    }
}

fn digest(data: &[u8]) -> [u8; 20] {
    Sha1::digest(data).into()
}

fn verifying_der(sk: &SigningKey) -> Option<Vec<u8>> {
    sk.verifying_key()
        .to_public_key_der()
        .ok()
        .map(|d| d.as_bytes().to_vec())
}

fn signing_from_der(der: &[u8]) -> Option<SigningKey> {
    let sk = p256::SecretKey::from_sec1_der(der).ok()?;
    Some(SigningKey::from(sk))
}

pub fn sign_data(vm: &mut Vm, data: &[u8]) -> String {
    let t = tokens(vm);
    let t = t.borrow();
    let h = digest(data);
    let mut rv = String::new();
    for sk in &t.signing {
        let Some(vk) = verifying_der(sk) else {
            continue;
        };
        let Ok(sig): Result<Signature, _> = sk.sign_prehash(&h) else {
            continue;
        };
        rv.push_str(&encode_line("signature", &vk, Some(&sig.to_bytes())));
    }
    rv
}

pub fn verify_data(vm: &mut Vm, data: &[u8], signatures: &str, check_verifying: bool) -> bool {
    let t = tokens(vm);
    let t = t.borrow();
    let h = digest(data);
    for line in signatures.lines() {
        let (kind, key, sig) = decode_line(line);
        if kind != "signature" {
            continue;
        }
        let Some(sig) = sig else { continue };
        if check_verifying && !t.verifying.contains(&key) {
            continue;
        }
        let (Ok(vk), Ok(sig)) = (
            VerifyingKey::from_public_key_der(&key),
            Signature::from_slice(&sig),
        ) else {
            continue;
        };
        if vk.verify_prehash(&h, &sig).is_ok() {
            return true;
        }
    }
    false
}

fn keys_from_signatures(signatures: &str) -> Vec<Vec<u8>> {
    signatures
        .lines()
        .filter_map(|l| {
            let (k, key, _) = decode_line(l);
            (k == "signature").then_some(key)
        })
        .collect()
}

fn store(vm: &Vm) -> Rc<SaveStore> {
    savestore::store(vm)
}

fn ask(vm: &mut Vm, prompt: &str) -> PyResult<bool> {
    let layout = py::import_attr(vm, "store", "layout")?;
    let yn = py::get(vm, &layout, "yesno_prompt")?;
    let gui = py::import_attr(vm, "store", "gui")?;
    let p = py::get(vm, &gui, prompt)?;
    let f = py::import_attr(vm, "renpy.exports", "invoke_in_new_context")?;
    let r = vm.call(&f, vec![yn, Value::None, p], Vec::new())?;
    py::truthy(vm, &r)
}

/// `check_load(log, signatures)`: whether a save may be loaded, asking
/// the player about unknown keys as upstream does.
pub fn check_load(vm: &mut Vm, log: &[u8], signatures: &str) -> PyResult<bool> {
    let (active, has_keys) = {
        let t = tokens(vm);
        let t = t.borrow();
        (t.active, !t.signing.is_empty())
    };
    if !active || !has_keys {
        return Ok(true);
    }
    if verify_data(vm, log, signatures, true) {
        return Ok(true);
    }
    if !ask(vm, "UNKNOWN_TOKEN")? {
        return Ok(false);
    }
    let known = tokens(vm).borrow().verifying.clone();
    let new_keys: Vec<Vec<u8>> = keys_from_signatures(signatures)
        .into_iter()
        .filter(|k| !known.contains(k))
        .collect();
    if !new_keys.is_empty() && ask(vm, "TRUST_TOKEN")? {
        let s = store(vm);
        let name = format!("{TOKEN_DIR}/security_keys.txt");
        let mut text = s
            .read(&name)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        for k in &new_keys {
            text.push_str(&encode_line("verifying-key", k, None));
        }
        s.write(&name, text.as_bytes())
            .map_err(|e| py::error(vm, e))?;
        tokens(vm).borrow_mut().verifying.extend(new_keys);
    }
    if signatures.is_empty() {
        return Ok(true);
    }
    Ok(verify_data(vm, log, signatures, false))
}

pub fn check_persistent(vm: &mut Vm, data: &[u8], signatures: &str) -> bool {
    if tokens(vm).borrow().should_upgrade {
        return true;
    }
    verify_data(vm, data, signatures, true)
}

fn random_key() -> Option<SigningKey> {
    for _ in 0..16 {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).ok()?;
        if let Ok(k) = SigningKey::from_slice(&b) {
            return Some(k);
        }
    }
    None
}

fn create_token(vm: &mut Vm, name: &str) -> Result<(), String> {
    let sk = random_key().ok_or("no random source for the save token")?;
    let secret = p256::SecretKey::from(&sk);
    let der = secret.to_sec1_der().map_err(|e| e.to_string())?;
    let vk = verifying_der(&sk).ok_or("verifying key encoding")?;
    store(vm).write(name, encode_line("signing-key", &der, Some(&vk)).as_bytes())
}

/// `init_tokens()`.
fn init_tokens(vm: &mut Vm) -> PyResult<()> {
    let t = tokens(vm);
    let save_directory = py::config(vm, "save_directory").unwrap_or(Value::None);
    if py::is_none(&save_directory) {
        t.borrow_mut().should_upgrade = true;
        return Ok(());
    }
    let keys = format!("{TOKEN_DIR}/security_keys.txt");
    let s = store(vm);
    if s.read(&keys).is_none() {
        create_token(vm, &keys).map_err(|e| py::error(vm, e))?;
    }
    let text = s
        .read(&keys)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    {
        let mut t = t.borrow_mut();
        t.active = true;
        for l in text.lines() {
            let (kind, key, _) = decode_line(l);
            match kind.as_str() {
                "signing-key" => {
                    if let Some(sk) = signing_from_der(&key) {
                        if let Some(vk) = verifying_der(&sk) {
                            t.verifying.push(vk);
                        }
                        t.signing.push(sk);
                    }
                }
                "verifying-key" => t.verifying.push(key),
                _ => {}
            }
        }
    }
    let stk = py::config(vm, "save_token_keys").unwrap_or(Value::None);
    if !py::is_none(&stk) {
        for k in py::items(vm, &stk)? {
            let ks = py::string(vm, &k)?;
            let raw = B64.decode(ks.trim()).unwrap_or_default();
            if VerifyingKey::from_public_key_der(&raw).is_ok() {
                t.borrow_mut().verifying.push(raw);
            } else if let Some(sk) = signing_from_der(&raw) {
                let vk = verifying_der(&sk)
                    .map(|v| B64.encode(v))
                    .unwrap_or_default();
                return Err(py::error(
                    vm,
                    format!(
                        "In config.save_token_keys, the signing key '{ks}' was provided, but the verifying key '{vk}' is required."
                    ),
                ));
            } else {
                return Err(py::error(
                    vm,
                    format!("In config.save_token_keys, the key '{ks}' is not a valid key."),
                ));
            }
        }
    }
    let up = format!("{TOKEN_DIR}/upgraded.txt");
    let upgraded = s
        .read(&up)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let sd = py::string(vm, &save_directory)?;
    if !upgraded.lines().any(|l| l == sd) {
        t.borrow_mut().should_upgrade = true;
    }
    Ok(())
}

/// `upgrade_all_savefiles()`: signs saves made before tokens existed.
pub fn upgrade_all_savefiles(vm: &mut Vm) -> PyResult<()> {
    let (active, upgrade) = {
        let t = tokens(vm);
        let t = t.borrow();
        (t.active, t.should_upgrade)
    };
    if !active || !upgrade {
        return Ok(());
    }
    let s = store(vm);
    let suffix = py::import_attr(vm, "renpy", "savegame_suffix")?;
    let suffix = py::string(vm, &suffix)?;
    for f in s.list("") {
        if !f.ends_with(&suffix) {
            continue;
        }
        let Some(bytes) = s.read(&f) else { continue };
        let Ok(entries) = savestore::zip_read(&bytes) else {
            continue;
        };
        if entries.iter().any(|e| e.name == "signatures") {
            continue;
        }
        let Some(log) = entries.iter().find(|e| e.name == "log") else {
            continue;
        };
        let sig = sign_data(vm, &log.data);
        let mut parts: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.data.as_slice()))
            .collect();
        parts.push(("signatures", sig.as_bytes()));
        let z = savestore::zip_write(&parts, savestore::dos_time(s.mtime(&f).unwrap_or(0.0)));
        let _ = s.write(&f, &z);
    }
    let up = format!("{TOKEN_DIR}/upgraded.txt");
    let mut text = s
        .read(&up)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let sd = py::config(vm, "save_directory")?;
    text.push_str(&py::string(vm, &sd)?);
    text.push('\n');
    s.write(&up, text.as_bytes())
        .map_err(|e| py::error(vm, e))?;
    Ok(())
}

fn bytes_arg(vm: &mut Vm, v: &Value) -> PyResult<Vec<u8>> {
    match v.payload() {
        Some(crysirror_python::Payload::Bytes(b)) => Ok(b.to_vec()),
        _ => Ok(py::string(vm, v)?.into_bytes()),
    }
}

fn f_sign_data(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &args[0])?;
    let s = sign_data(vm, &d);
    Ok(vm.types.new_str(&s))
}

fn f_verify_data(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &args[0])?;
    let s = py::string(vm, &args[1])?;
    let c = arg_or(&args[2..], kw, 0, "check_verifying", Value::Bool(true));
    let c = py::truthy(vm, &c)?;
    Ok(Value::Bool(verify_data(vm, &d, &s, c)))
}

fn f_check_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &args[0])?;
    let s = py::string(vm, &args[1])?;
    Ok(Value::Bool(check_load(vm, &d, &s)?))
}

fn f_check_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &args[0])?;
    let s = py::string(vm, &args[1])?;
    Ok(Value::Bool(check_persistent(vm, &d, &s)))
}

fn f_encode_line(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = py::string(vm, &args[0])?;
    let a = bytes_arg(vm, &args[1])?;
    let b = match args.get(2) {
        Some(v) if !py::is_none(v) => Some(bytes_arg(vm, v)?),
        _ => None,
    };
    Ok(vm.types.new_str(&encode_line(&k, &a, b.as_deref())))
}

fn f_decode_line(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = py::string(vm, &args[0])?;
    let (k, a, b) = decode_line(&l);
    let k = vm.types.new_str(&k);
    let a = vm.types.new_bytes(a);
    let b = b.map(|b| vm.types.new_bytes(b)).unwrap_or(Value::None);
    Ok(vm.types.new_tuple(vec![k, a, b]))
}

fn f_get_keys_from_signatures(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    let v = keys_from_signatures(&s)
        .into_iter()
        .map(|k| vm.types.new_bytes(k))
        .collect();
    Ok(vm.types.new_list(v))
}

fn f_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if let Err(e) = init_tokens(vm) {
        let msg = py::error_text(vm, &e);
        super::log::write(vm, &format!("Initializing save token: {msg}"));
    }
    Ok(Value::None)
}

fn f_init_tokens(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    init_tokens(vm)?;
    Ok(Value::None)
}

fn f_upgrade_all_savefiles(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    upgrade_all_savefiles(vm)?;
    Ok(Value::None)
}

fn f_get_save_token_keys(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let t = tokens(vm);
    let keys: Vec<String> = t
        .borrow()
        .signing
        .iter()
        .filter_map(verifying_der)
        .map(|v| B64.encode(v))
        .collect();
    Ok(vm
        .types
        .new_list(keys.iter().map(|k| vm.types.new_str(k)).collect()))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("encode_line", f_encode_line),
            ("decode_line", f_decode_line),
            ("sign_data", f_sign_data),
            ("verify_data", f_verify_data),
            ("get_keys_from_signatures", f_get_keys_from_signatures),
            ("check_load", f_check_load),
            ("check_persistent", f_check_persistent),
            ("init_tokens", f_init_tokens),
            ("init", f_init),
            ("upgrade_all_savefiles", f_upgrade_all_savefiles),
            ("get_save_token_keys", f_get_save_token_keys),
        ],
    );
    crysirror_python::modules::set(vm, &m, "token_dir", vm.types.new_str(TOKEN_DIR));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_lines_round_trip() {
        let (k, a, b) = decode_line(&encode_line("signature", b"abc", Some(b"de")));
        assert_eq!(
            (k.as_str(), a.as_slice(), b.as_deref()),
            ("signature", &b"abc"[..], Some(&b"de"[..]))
        );
        let sk = random_key().unwrap();
        let der = p256::SecretKey::from(&sk).to_sec1_der().unwrap();
        let back = signing_from_der(&der).unwrap();
        assert_eq!(verifying_der(&sk), verifying_der(&back));
        let h = digest(b"log");
        let sig: Signature = sk.sign_prehash(&h).unwrap();
        let vk = VerifyingKey::from_public_key_der(&verifying_der(&sk).unwrap()).unwrap();
        assert!(vk.verify_prehash(&h, &sig).is_ok());
    }
}
