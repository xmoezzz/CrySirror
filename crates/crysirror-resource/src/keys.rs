// SPDX-License-Identifier: MPL-2.0
use anyhow::{ensure, Context, Result};
use crysirror_core::state::ClientId;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct KeyMaterial(Arc<Zeroizing<Vec<u8>>>);
impl KeyMaterial {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= 1024 * 1024,
            "key material size must be 1..1048576"
        );
        Ok(Self(Arc::new(Zeroizing::new(bytes))))
    }
    pub fn with_bytes<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        f(self.0.as_slice())
    }
}
impl std::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyMaterial([redacted])")
    }
}
pub trait KeyProvider: Send + Sync {
    fn key(&self, client: ClientId, key_id: &str) -> Result<KeyMaterial>;
}
#[derive(Default)]
pub struct MemoryKeyProvider {
    keys: RwLock<BTreeMap<(ClientId, String), KeyMaterial>>,
}
impl MemoryKeyProvider {
    pub fn insert(&self, client: ClientId, id: &str, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control),
            "invalid key identifier"
        );
        let material = KeyMaterial::new(bytes)?;
        self.keys
            .write()
            .map_err(|_| anyhow::anyhow!("key provider poisoned"))?
            .insert((client, id.into()), material);
        Ok(())
    }
    pub fn remove(&self, client: ClientId, id: &str) -> Result<bool> {
        Ok(self
            .keys
            .write()
            .map_err(|_| anyhow::anyhow!("key provider poisoned"))?
            .remove(&(client, id.into()))
            .is_some())
    }
}
impl KeyProvider for MemoryKeyProvider {
    fn key(&self, client: ClientId, id: &str) -> Result<KeyMaterial> {
        self.keys
            .read()
            .map_err(|_| anyhow::anyhow!("key provider poisoned"))?
            .get(&(client, id.into()))
            .cloned()
            .with_context(|| format!("key unavailable for client {}: {id}", client.0))
    }
}
