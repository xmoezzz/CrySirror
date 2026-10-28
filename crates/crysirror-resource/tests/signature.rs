// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use base64::{Engine, engine::general_purpose::STANDARD};
use crysirror_core::service::AssetReader;
use crysirror_resource::signature::{
    MAX_PUBLIC_KEY_BYTES, MAX_SIGNATURE_BLOB_BYTES, RsaPssSha256Verifier, SIGNATURE_HEADER,
    SignatureError, parse_signature_blob,
};
use rsa::{
    RsaPrivateKey,
    pkcs1::EncodeRsaPublicKey,
    pkcs8::{EncodePublicKey, LineEnding},
    pss::SigningKey,
    rand_core::{CryptoRng, Error, RngCore},
    signature::{RandomizedSigner, SignatureEncoding},
};
use sha2::{Sha256, Sha384};
use std::{io::Cursor, sync::OnceLock};

// Deterministic runtime-generated test keys only. This predictable RNG is never
// used by production code and must not be used for actual key generation.
struct TestRng(u64);
impl RngCore for TestRng {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for part in dest.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            part.copy_from_slice(&bytes[..part.len()]);
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl CryptoRng for TestRng {}

struct Fixture {
    private: RsaPrivateKey,
    other: RsaPrivateKey,
    message: Vec<u8>,
    signature: Vec<u8>,
    der: Vec<u8>,
}
fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let mut rng = TestRng(0xf3f9_9e21_24b8_7811);
        let private = RsaPrivateKey::new(&mut rng, 1024).unwrap();
        let other = RsaPrivateKey::new(&mut rng, 1024).unwrap();
        let message = (0..100_003).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        let signature = SigningKey::<Sha256>::new_with_salt_len(private.clone(), 32)
            .sign_with_rng(&mut rng, &message)
            .to_vec();
        let der = private
            .to_public_key()
            .to_pkcs1_der()
            .unwrap()
            .as_bytes()
            .to_vec();
        Fixture {
            private,
            other,
            message,
            signature,
            der,
        }
    })
}
fn wrap(signature: &[u8]) -> Vec<u8> {
    let mut out = format!("{SIGNATURE_HEADER}\r\n");
    for chunk in STANDARD.encode(signature).as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push_str("\r\n");
    }
    out.into_bytes()
}
fn verifier() -> RsaPssSha256Verifier {
    RsaPssSha256Verifier::from_pkcs1_der(&fixture().der).unwrap()
}

#[test]
fn generated_pss_signatures_verify_as_bytes_streams_and_assets() {
    let f = fixture();
    let verifier = verifier();
    let blob = wrap(&f.signature);
    assert_eq!(parse_signature_blob(&blob).unwrap(), f.signature);
    assert!(verifier.verify(&f.message, &blob).unwrap());
    assert!(
        verifier
            .verify_reader(&mut Cursor::new(&f.message), f.message.len() as u64, &blob)
            .unwrap()
    );
    let asset = PartialAsset {
        bytes: &f.message,
        chunk: 29,
        advertised: f.message.len() as u64,
    };
    assert!(
        verifier
            .verify_asset(&asset, asset.advertised, &blob)
            .unwrap()
    );
    // No digest state from a previous verification may leak into the next one.
    assert!(!verifier.verify(b"another message", &blob).unwrap());
    assert!(verifier.verify(&f.message, &blob).unwrap());
}

#[test]
fn altered_messages_signatures_keys_hash_and_salt_do_not_authenticate() {
    let f = fixture();
    let verifier = verifier();
    let blob = wrap(&f.signature);
    let mut changed = f.message.clone();
    changed[16_385] ^= 0x80;
    assert!(!verifier.verify(&changed, &blob).unwrap());
    let mut signature = f.signature.clone();
    signature[31] ^= 1;
    assert!(!verifier.verify(&f.message, &wrap(&signature)).unwrap());
    assert!(
        !verifier
            .verify(&f.message, &wrap(&signature[..127]))
            .unwrap()
    );
    let wrong_der = f.other.to_public_key().to_pkcs1_der().unwrap();
    assert!(
        !RsaPssSha256Verifier::from_pkcs1_der(wrong_der.as_bytes())
            .unwrap()
            .verify(&f.message, &blob)
            .unwrap()
    );
    let mut rng = TestRng(0x52c2_ef47_b138_32a1);
    for salt_len in [0, 20] {
        let wrong_salt = SigningKey::<Sha256>::new_with_salt_len(f.private.clone(), salt_len)
            .sign_with_rng(&mut rng, &f.message);
        assert!(
            !verifier
                .verify(&f.message, &wrap(&wrong_salt.to_vec()))
                .unwrap()
        );
    }
    let wrong_hash = SigningKey::<Sha384>::new_with_salt_len(f.private.clone(), 32)
        .sign_with_rng(&mut rng, &f.message);
    assert!(
        !verifier
            .verify(&f.message, &wrap(&wrong_hash.to_vec()))
            .unwrap()
    );
}

#[test]
fn pem_labels_require_complete_pkcs1_payloads() {
    let f = fixture();
    for label in ["RSA PUBLIC KEY", "PUBLIC KEY"] {
        let pem = format!(
            "-----BEGIN {label}-----\r\n{}\r\n-----END {label}-----\r\n",
            STANDARD.encode(&f.der)
        );
        assert!(
            RsaPssSha256Verifier::from_pkcs1_pem(pem.as_bytes())
                .unwrap()
                .verify(&f.message, &wrap(&f.signature))
                .unwrap()
        );
    }
    let spki = f
        .private
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    assert!(RsaPssSha256Verifier::from_pkcs1_pem(spki.as_bytes()).is_err());
    let mut der_junk = f.der.clone();
    der_junk.push(0);
    assert!(RsaPssSha256Verifier::from_pkcs1_der(&der_junk).is_err());
    for bad in [
        b"".as_slice(),
        b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----",
        b"-----BEGIN PUBLIC KEY-----AA==\n-----END PUBLIC KEY-----",
    ] {
        assert!(RsaPssSha256Verifier::from_pkcs1_pem(bad).is_err());
    }
    assert!(matches!(
        RsaPssSha256Verifier::from_pkcs1_der(&vec![0; MAX_PUBLIC_KEY_BYTES + 1]),
        Err(SignatureError::LimitExceeded(_))
    ));
}

#[test]
fn malformed_wrappers_noncanonical_base64_and_parser_limits_fail() {
    for blob in [
        b"".to_vec(),
        b"AA==".to_vec(),
        format!("{SIGNATURE_HEADER}\n").into_bytes(),
        format!("{SIGNATURE_HEADER}\nAB==\n").into_bytes(), // non-zero trailing bits
        format!("{SIGNATURE_HEADER}\nAA==junk\n").into_bytes(),
        format!("{SIGNATURE_HEADER}\nAA==\n-- unknown footer --\n").into_bytes(),
        format!("{SIGNATURE_HEADER}\nAA\n").into_bytes(), // required padding
        format!("{SIGNATURE_HEADER}\nAA==\0").into_bytes(),
        b"-- SIGNATURE - SHA1/PSS/RSA --\nAA==\n".to_vec(),
        wrap(&vec![1; 1025]),
    ] {
        assert!(parse_signature_blob(&blob).is_err());
    }
    assert!(matches!(
        parse_signature_blob(&vec![b' '; MAX_SIGNATURE_BLOB_BYTES + 1]),
        Err(SignatureError::LimitExceeded(_))
    ));
    assert_eq!(
        parse_signature_blob(format!("{SIGNATURE_HEADER}\n A A = = \t\r\n").as_bytes()).unwrap(),
        [0]
    );
}

struct PartialAsset<'a> {
    bytes: &'a [u8],
    chunk: usize,
    advertised: u64,
}
impl AssetReader for PartialAsset<'_> {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.advertised)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let Some(remaining) = self.bytes.get(offset as usize..) else {
            return Ok(0);
        };
        let count = out.len().min(self.chunk).min(remaining.len());
        out[..count].copy_from_slice(&remaining[..count]);
        Ok(count)
    }
}

#[test]
fn message_limits_short_assets_and_io_errors_do_not_become_success() {
    let f = fixture();
    let verifier = verifier();
    let blob = wrap(&f.signature);
    assert!(matches!(
        verifier.verify_reader(
            &mut Cursor::new(&f.message),
            f.message.len() as u64 - 1,
            &blob
        ),
        Err(SignatureError::LimitExceeded(_))
    ));
    let too_large = PartialAsset {
        bytes: &f.message,
        chunk: 7,
        advertised: f.message.len() as u64,
    };
    assert!(matches!(
        verifier.verify_asset(&too_large, 5, &blob),
        Err(SignatureError::LimitExceeded(_))
    ));
    let short = PartialAsset {
        bytes: &f.message[..20],
        chunk: 7,
        advertised: 21,
    };
    assert!(matches!(
        verifier.verify_asset(&short, 21, &blob),
        Err(SignatureError::Read(_))
    ));
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("synthetic read failure"))
        }
    }
    assert!(matches!(
        verifier.verify_reader(&mut Broken, 5, &blob),
        Err(SignatureError::Read(_))
    ));
    let empty_sig = SigningKey::<Sha256>::new_with_salt_len(f.private.clone(), 32)
        .sign_with_rng(&mut TestRng(0x89ea_a510_24d4_3b5f), b"")
        .to_vec();
    assert!(
        verifier
            .verify_reader(&mut Cursor::new([]), 0, &wrap(&empty_sig))
            .unwrap()
    );
}
