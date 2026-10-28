// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CPU-only bounded RSA-PSS/SHA-256 verification, independent of any VM.
//!
//! The wrapped signature profile is an exact `SIGNATURE_HEADER` line followed
//! by canonical padded Base64, with ASCII space/tab/CR/LF permitted as wrapping.
//! Key input is PKCS#1 DER, including the protected-bootstrap PEM variant whose
//! label says PUBLIC KEY although its payload is PKCS#1 rather than SPKI.
//! Protocol review: cxdec-hxv4-static-analysis at
//! 3c6dee001df011fa7ed557c7463635c6f67698f6,
//! docs/core/FileSignatureCheck.md sections 3–8 (whole document reviewed).
//! Native path lookup, verifier lifecycle and capability ownership belong to the
//! caller. Submit these synchronous primitives to resource workers for large data.

use base64::{Engine, engine::general_purpose::STANDARD};
use crysirror_core::service::AssetReader;
use rsa::{
    BigUint, RsaPublicKey,
    pss::{Signature, VerifyingKey},
    signature::DigestVerifier,
    traits::PublicKeyParts,
};
use sha2::{Digest, Sha256};
use std::{fmt, io::Read};

/// Supported text signature scheme; other schemes must be selected explicitly.
pub const SIGNATURE_HEADER: &str = "-- SIGNATURE - SHA256/PSS/RSA --";
/// Upper bound for PEM/DER public-key input before parsing/allocation.
pub const MAX_PUBLIC_KEY_BYTES: usize = 16 * 1024;
/// Upper bound for wrapped signature input before parsing/allocation.
pub const MAX_SIGNATURE_BLOB_BYTES: usize = 16 * 1024;
const MAX_SIGNATURE_BYTES: usize = 1024;
const CHUNK_BYTES: usize = 16 * 1024;

/// Malformed inputs and read failures are distinct from a validly encoded
/// signature that does not authenticate the message (`Ok(false)`).
#[derive(Debug)]
pub enum SignatureError {
    /// Input exceeded its fixed parser bound or selected message budget.
    LimitExceeded(&'static str),
    /// Public key was malformed, unsupported, or outside 1024–8192 bits.
    InvalidPublicKey,
    /// Signature wrapper/Base64 was malformed or unsupported.
    InvalidSignatureBlob,
    /// The source failed or violated its read/length contract.
    Read(String),
}

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded(kind) => write!(f, "signature {kind} byte limit exceeded"),
            Self::InvalidPublicKey => f.write_str("invalid or unsupported PKCS#1 RSA public key"),
            Self::InvalidSignatureBlob => f.write_str("invalid SHA256/PSS/RSA signature wrapper"),
            Self::Read(error) => write!(f, "signature resource read failed: {error}"),
        }
    }
}

impl std::error::Error for SignatureError {}

/// Parse the evidenced header-plus-Base64 format. There is no implicit raw-RSA,
/// headerless Base64, alternate algorithm, or unknown-footer fallback.
pub fn parse_signature_blob(blob: &[u8]) -> Result<Vec<u8>, SignatureError> {
    if blob.len() > MAX_SIGNATURE_BLOB_BYTES {
        return Err(SignatureError::LimitExceeded("blob"));
    }
    let split = blob
        .iter()
        .position(|&byte| byte == b'\n')
        .ok_or(SignatureError::InvalidSignatureBlob)?;
    let header = blob[..split].strip_suffix(b"\r").unwrap_or(&blob[..split]);
    if header != SIGNATURE_HEADER.as_bytes() {
        return Err(SignatureError::InvalidSignatureBlob);
    }
    let signature =
        decode_base64(&blob[split + 1..]).map_err(|_| SignatureError::InvalidSignatureBlob)?;
    if signature.is_empty() || signature.len() > MAX_SIGNATURE_BYTES {
        return Err(SignatureError::InvalidSignatureBlob);
    }
    Ok(signature)
}

fn decode_base64(bytes: &[u8]) -> Result<Vec<u8>, ()> {
    let mut compact = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            continue;
        }
        if !byte.is_ascii_alphanumeric() && !matches!(byte, b'+' | b'/' | b'=') {
            return Err(());
        }
        compact.push(byte);
    }
    STANDARD.decode(compact).map_err(|_| ())
}

/// Immutable validated public key. Each verification uses its own digest and
/// signature state. SHA-256, MGF1(SHA-256) and salt length 32 are fixed.
#[derive(Clone)]
pub struct RsaPssSha256Verifier {
    key: VerifyingKey<Sha256>,
    signature_bytes: usize,
}

impl RsaPssSha256Verifier {
    /// Parse a complete PKCS#1 RSAPublicKey DER object (not SPKI).
    pub fn from_pkcs1_der(der: &[u8]) -> Result<Self, SignatureError> {
        if der.len() > MAX_PUBLIC_KEY_BYTES {
            return Err(SignatureError::LimitExceeded("public key"));
        }
        let parsed = rsa::pkcs1::RsaPublicKey::try_from(der)
            .map_err(|_| SignatureError::InvalidPublicKey)?;
        let key = RsaPublicKey::new_with_max_size(
            BigUint::from_bytes_be(parsed.modulus.as_bytes()),
            BigUint::from_bytes_be(parsed.public_exponent.as_bytes()),
            8192,
        )
        .map_err(|_| SignatureError::InvalidPublicKey)?;
        if !(1024..=8192).contains(&key.n().bits()) {
            return Err(SignatureError::InvalidPublicKey);
        }
        let signature_bytes = key.size();
        Ok(Self {
            key: VerifyingKey::new_with_salt_len(key, 32),
            signature_bytes,
        })
    }

    /// Accept RSA PUBLIC KEY or the evidenced mislabeled PUBLIC KEY envelope;
    /// both must contain strict PKCS#1 DER, not SPKI/certificates/private keys.
    pub fn from_pkcs1_pem(pem: &[u8]) -> Result<Self, SignatureError> {
        if pem.len() > MAX_PUBLIC_KEY_BYTES {
            return Err(SignatureError::LimitExceeded("public key"));
        }
        if !pem.is_ascii() {
            return Err(SignatureError::InvalidPublicKey);
        }
        let text = std::str::from_utf8(pem).map_err(|_| SignatureError::InvalidPublicKey)?;
        let text = text.trim_matches(|c| matches!(c, ' ' | '\t' | '\r' | '\n'));
        for label in ["RSA PUBLIC KEY", "PUBLIC KEY"] {
            let header = format!("-----BEGIN {label}-----");
            let footer = format!("-----END {label}-----");
            if let Some(body) = text
                .strip_prefix(&header)
                .and_then(|s| s.strip_suffix(&footer))
            {
                // Envelope delimiters must occupy separate lines, not occur
                // inside Base64 or attach to the same line as the payload.
                if !body.starts_with('\n') && !body.starts_with("\r\n") {
                    return Err(SignatureError::InvalidPublicKey);
                }
                if !body.ends_with('\n') {
                    return Err(SignatureError::InvalidPublicKey);
                }
                let der =
                    decode_base64(body.as_bytes()).map_err(|_| SignatureError::InvalidPublicKey)?;
                return Self::from_pkcs1_der(&der);
            }
        }
        Err(SignatureError::InvalidPublicKey)
    }

    /// Verify bytes without retaining them. Invalid cryptographic signatures,
    /// including a wrong modulus-sized length, produce `Ok(false)`.
    pub fn verify(&self, message: &[u8], blob: &[u8]) -> Result<bool, SignatureError> {
        let signature = parse_signature_blob(blob)?;
        Ok(self.verify_hash(Sha256::new_with_prefix(message), &signature))
    }

    /// Hash a bounded reader in fixed chunks. This performs blocking CPU/I/O
    /// work; the caller owns worker scheduling and completion publication.
    pub fn verify_reader<R: Read + ?Sized>(
        &self,
        reader: &mut R,
        max_bytes: u64,
        blob: &[u8],
    ) -> Result<bool, SignatureError> {
        let signature = parse_signature_blob(blob)?;
        let mut digest = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; CHUNK_BYTES];
        loop {
            // Read one byte past the remaining budget to detect an overrun,
            // including when the budget is exactly zero or already exhausted.
            let count = (max_bytes - total)
                .saturating_add(1)
                .min(CHUNK_BYTES as u64) as usize;
            let read = match reader.read(&mut buffer[..count]) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result.map_err(|e| SignatureError::Read(e.to_string()))?,
            };
            if read > count {
                return Err(SignatureError::Read("invalid reader count".into()));
            }
            if read == 0 {
                break;
            }
            if read as u64 > max_bytes - total {
                return Err(SignatureError::LimitExceeded("message"));
            }
            total += read as u64;
            digest.update(&buffer[..read]);
        }
        Ok(self.verify_hash(digest, &signature))
    }

    /// Verify a scoped engine asset without allocating its complete contents.
    /// The capability root is enforced when the caller obtains the AssetReader.
    /// Partial reads are supported; early EOF and changing length fail visibly.
    pub fn verify_asset(
        &self,
        source: &dyn AssetReader,
        max_bytes: u64,
        blob: &[u8],
    ) -> Result<bool, SignatureError> {
        let signature = parse_signature_blob(blob)?;
        let len = source
            .len()
            .map_err(|e| SignatureError::Read(e.to_string()))?;
        if len > max_bytes {
            return Err(SignatureError::LimitExceeded("message"));
        }
        let mut digest = Sha256::new();
        let mut offset = 0u64;
        let mut buffer = [0u8; CHUNK_BYTES];
        while offset < len {
            let count = (len - offset).min(CHUNK_BYTES as u64) as usize;
            let read = source
                .read_at(offset, &mut buffer[..count])
                .map_err(|e| SignatureError::Read(e.to_string()))?;
            if read == 0 || read > count {
                return Err(SignatureError::Read("short or invalid asset read".into()));
            }
            offset += read as u64;
            digest.update(&buffer[..read]);
        }
        if source
            .len()
            .map_err(|e| SignatureError::Read(e.to_string()))?
            != len
        {
            return Err(SignatureError::Read("asset length changed".into()));
        }
        Ok(self.verify_hash(digest, &signature))
    }

    fn verify_hash(&self, digest: Sha256, signature: &[u8]) -> bool {
        if signature.len() != self.signature_bytes {
            return false;
        }
        let Ok(signature) = Signature::try_from(signature) else {
            return false;
        };
        self.key.verify_digest(digest, &signature).is_ok()
    }
}
