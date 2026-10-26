// SPDX-License-Identifier: MPL-2.0
use crate::{context::Resident, io::AssetIo, read_all, DecodeContext, KeyMaterial};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::{AssetReader, MemoryAssetReader};
use ctr::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformSpec {
    /// Registered byte-transform identifier.
    pub algorithm: String,
    #[serde(default)]
    /// Optional client-scoped host key alias; never raw secret bytes.
    pub key_id: Option<String>,
    #[serde(default)]
    /// Explicit algorithm options; unknown options fail.
    pub parameters: serde_json::Value,
    #[serde(default)]
    /// Optional exact logical output size, required by LZSS.
    pub expected_size: Option<u64>,
}
impl TransformSpec {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            algorithm: id.into(),
            key_id: None,
            parameters: serde_json::Value::Null,
            expected_size: None,
        }
    }
}
pub trait ByteTransform: Send + Sync {
    fn apply(
        &self,
        spec: &TransformSpec,
        input: Arc<dyn AssetReader>,
        metadata: &serde_json::Value,
        context: &DecodeContext,
    ) -> Result<Arc<dyn AssetReader>>;
}
pub struct TransformRegistry {
    transforms: BTreeMap<String, Arc<dyn ByteTransform>>,
}
impl Default for TransformRegistry {
    fn default() -> Self {
        let mut out = Self {
            transforms: BTreeMap::new(),
        };
        for id in [
            "identity",
            "zlib",
            "deflate",
            "xor",
            "aes-ctr128be",
            "aes-ctr128le",
            "aes-cbc",
            "aes256-cbc",
            "rc4",
        ] {
            out.transforms.insert(id.into(), Arc::new(Builtin));
        }
        out.transforms
            .insert("lzss".into(), Arc::new(crate::lzss::Lzss));
        out
    }
}
impl TransformRegistry {
    pub fn register(&mut self, id: &str, transform: Arc<dyn ByteTransform>) -> Result<()> {
        ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"/_-".contains(&c)),
            "invalid transform identifier"
        );
        ensure!(
            !self.transforms.contains_key(id),
            "duplicate resource transform"
        );
        self.transforms.insert(id.into(), transform);
        Ok(())
    }
    pub fn names(&self) -> Vec<&str> {
        self.transforms.keys().map(String::as_str).collect()
    }
    pub fn apply(
        &self,
        spec: &TransformSpec,
        input: Arc<dyn AssetReader>,
        metadata: &serde_json::Value,
        context: &DecodeContext,
    ) -> Result<Arc<dyn AssetReader>> {
        self.transforms
            .get(&spec.algorithm)
            .with_context(|| format!("unsupported resource transform: {}", spec.algorithm))?
            .apply(spec, input, metadata, context)
    }
}
struct Builtin;
fn no_parameters(spec: &TransformSpec) -> Result<()> {
    ensure!(
        spec.key_id.is_none()
            && (spec.parameters.is_null()
                || spec.parameters.as_object().is_some_and(|v| v.is_empty())),
        "unexpected key or parameters for {}",
        spec.algorithm
    );
    Ok(())
}
impl ByteTransform for Builtin {
    fn apply(
        &self,
        spec: &TransformSpec,
        input: Arc<dyn AssetReader>,
        _: &serde_json::Value,
        context: &DecodeContext,
    ) -> Result<Arc<dyn AssetReader>> {
        match spec.algorithm.as_str() {
            "aes-cbc" | "aes256-cbc" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    iv: [u8; 16],
                    #[serde(default = "pkcs7")]
                    padding: String,
                }
                fn pkcs7() -> String {
                    "pkcs7".into()
                }
                let p: Params = serde_json::from_value(spec.parameters.clone())
                    .context("AES CBC parameters")?;
                ensure!(
                    ["none", "pkcs7"].contains(&p.padding.as_str()),
                    "unsupported CBC padding"
                );
                let key = context.keys.key(
                    context.client,
                    spec.key_id.as_deref().context("AES CBC requires key_id")?,
                )?;
                if spec.algorithm == "aes256-cbc" {
                    ensure!(
                        key.with_bytes(|b| b.len() == 32),
                        "AES256 CBC needs 32-byte key"
                    );
                }
                let n = input.len().map_err(|e| anyhow::anyhow!("{e}"))?;
                context.decode_bounded(n, || {
                    let bytes = read_all(input.as_ref(), n)?;
                    key.with_bytes(|key| {
                        crate::aes_cbc_decrypt(&bytes, &p.iv, key, p.padding == "pkcs7")
                    })
                })
            }
            "rc4" => {
                ensure!(
                    spec.parameters.is_null()
                        || spec.parameters.as_object().is_some_and(|m| m.is_empty()),
                    "RC4 takes no parameters"
                );
                let key = context.keys.key(
                    context.client,
                    spec.key_id.as_deref().context("RC4 requires key_id")?,
                )?;
                let n = input.len().map_err(|e| anyhow::anyhow!("{e}"))?;
                context.decode_with(n, || {
                    let bytes = read_all(input.as_ref(), n)?;
                    key.with_bytes(|key| crate::rc4_crypt(&bytes, key))
                })
            }
            "identity" => {
                no_parameters(spec)?;
                Ok(input)
            }
            "zlib" | "deflate" => {
                no_parameters(spec)?;
                let stream = AssetIo::new(input)?;
                if spec.algorithm == "zlib" {
                    inflate(flate2::read::ZlibDecoder::new(stream), spec, context)
                } else {
                    inflate(flate2::read::DeflateDecoder::new(stream), spec, context)
                }
            }
            "xor" => {
                #[derive(Deserialize, Default)]
                #[serde(default, deny_unknown_fields)]
                struct Params {
                    offset: u64,
                }
                let params: Params = if spec.parameters.is_null() {
                    Params::default()
                } else {
                    serde_json::from_value(spec.parameters.clone())?
                };
                let key = context.keys.key(
                    context.client,
                    spec.key_id.as_deref().context("xor requires key_id")?,
                )?;
                let len = input.len().map_err(|e| anyhow::anyhow!("{e}"))?;
                ensure!(
                    params.offset.checked_add(len).is_some(),
                    "xor offset overflow"
                );
                Ok(Arc::new(XorReader {
                    input,
                    key,
                    offset: params.offset,
                }))
            }
            "aes-ctr128be" | "aes-ctr128le" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    iv: [u8; 16],
                    #[serde(default)]
                    offset: u64,
                }
                let params: Params = serde_json::from_value(spec.parameters.clone())
                    .context("AES-CTR needs a 16-byte iv")?;
                let key = context.keys.key(
                    context.client,
                    spec.key_id.as_deref().context("AES-CTR requires key_id")?,
                )?;
                ensure!(
                    key.with_bytes(|b| matches!(b.len(), 16 | 24 | 32)),
                    "AES key must be 16, 24 or 32 bytes"
                );
                let len = input.len().map_err(|e| anyhow::anyhow!("{e}"))?;
                ensure!(
                    params.offset.checked_add(len).is_some(),
                    "AES byte offset overflow"
                );
                Ok(Arc::new(AesReader {
                    input,
                    key,
                    iv: params.iv,
                    offset: params.offset,
                    little: spec.algorithm.ends_with("le"),
                }))
            }
            _ => bail!("unsupported builtin transform"),
        }
    }
}
fn inflate(
    mut reader: impl Read,
    spec: &TransformSpec,
    context: &DecodeContext,
) -> Result<Arc<dyn AssetReader>> {
    let limit = spec.expected_size.unwrap_or(context.limits.max_entry_size);
    ensure!(
        limit <= context.limits.max_entry_size && limit <= usize::MAX as u64,
        "decoded entry size limit"
    );
    let mut reservation = context.reserve(limit)?;
    let mut out = Vec::new();
    let mut block = [0u8; 16 * 1024];
    loop {
        if out.len() as u64 == limit {
            let mut extra = [0u8; 1];
            ensure!(
                reader.read(&mut extra)? == 0,
                "decompressed entry exceeds declared size or limit"
            );
            break;
        }
        let count = block.len().min((limit - out.len() as u64) as usize);
        let n = reader.read(&mut block[..count])?;
        if n == 0 {
            break;
        }
        if out.capacity() - out.len() < n {
            let target = out
                .capacity()
                .saturating_mul(2)
                .max(out.len() + n)
                .min(limit as usize);
            out.try_reserve_exact(target - out.len())?;
        }
        out.extend_from_slice(&block[..n]);
    }
    if let Some(size) = spec.expected_size {
        ensure!(out.len() as u64 == size, "decompressed entry size mismatch");
    }
    reservation.shrink(out.len() as u64);
    Ok(Arc::new(Resident {
        reader: MemoryAssetReader(Arc::from(out)),
        _reservation: reservation,
    }))
}
struct XorReader {
    input: Arc<dyn AssetReader>,
    key: KeyMaterial,
    offset: u64,
}
impl AssetReader for XorReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.input.len()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let n = self.input.read_at(offset, out)?;
        if n > out.len() {
            return Err(crysirror_core::Error::Invalid(
                "transform source read length",
            ));
        }
        if n == 0 {
            return Ok(0);
        }
        let at = self
            .offset
            .checked_add(offset)
            .ok_or(crysirror_core::Error::Invalid("xor offset overflow"))?;
        self.key.with_bytes(|key| {
            let start = (at % key.len() as u64) as usize;
            for (i, b) in out[..n].iter_mut().enumerate() {
                *b ^= key[(start + i) % key.len()];
            }
        });
        Ok(n)
    }
}
struct AesReader {
    input: Arc<dyn AssetReader>,
    key: KeyMaterial,
    iv: [u8; 16],
    offset: u64,
    little: bool,
}
fn cipher<C: KeyIvInit + StreamCipher + StreamCipherSeek>(
    key: &[u8],
    iv: &[u8],
    offset: u64,
    out: &mut [u8],
) -> Result<()> {
    let mut cipher =
        C::new_from_slices(key, iv).map_err(|_| anyhow::anyhow!("invalid cipher key or IV"))?;
    cipher
        .try_seek(offset)
        .map_err(|_| anyhow::anyhow!("cipher counter exhausted"))?;
    cipher
        .try_apply_keystream(out)
        .map_err(|_| anyhow::anyhow!("cipher counter exhausted"))?;
    Ok(())
}
impl AssetReader for AesReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.input.len()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let n = self.input.read_at(offset, out)?;
        if n > out.len() {
            return Err(crysirror_core::Error::Invalid(
                "transform source read length",
            ));
        }
        if n == 0 {
            return Ok(0);
        }
        let at = self
            .offset
            .checked_add(offset)
            .ok_or(crysirror_core::Error::Invalid("cipher offset overflow"))?;
        self.key
            .with_bytes(|key| match (key.len(), self.little) {
                (16, false) => {
                    cipher::<ctr::Ctr128BE<aes::Aes128>>(key, &self.iv, at, &mut out[..n])
                }
                (24, false) => {
                    cipher::<ctr::Ctr128BE<aes::Aes192>>(key, &self.iv, at, &mut out[..n])
                }
                (32, false) => {
                    cipher::<ctr::Ctr128BE<aes::Aes256>>(key, &self.iv, at, &mut out[..n])
                }
                (16, true) => {
                    cipher::<ctr::Ctr128LE<aes::Aes128>>(key, &self.iv, at, &mut out[..n])
                }
                (24, true) => {
                    cipher::<ctr::Ctr128LE<aes::Aes192>>(key, &self.iv, at, &mut out[..n])
                }
                (32, true) => {
                    cipher::<ctr::Ctr128LE<aes::Aes256>>(key, &self.iv, at, &mut out[..n])
                }
                _ => bail!("invalid AES key length"),
            })
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))?;
        Ok(n)
    }
}
