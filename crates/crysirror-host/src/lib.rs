// SPDX-License-Identifier: MPL-2.0
//! In-memory reference host, for service contracts only. Not a GPU/audio backend.
#![forbid(unsafe_code)]
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{FileSystem, HostServices},
    state::{ClientId, Command, Limits, SceneState},
    Error, Result,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub engine: String,
    pub game: String,
}

pub struct ReferenceHost {
    pub scene: SceneState,
    pub limits: Limits,
    clients: BTreeMap<ClientId, ClientInfo>,
    next_client: u32,
    now_ms: u64,
    assets: BTreeMap<(String, String, String), Vec<u8>>,
    writable: BTreeMap<(String, String, Mount, String), Vec<u8>>,
}

impl Default for ReferenceHost {
    fn default() -> Self {
        Self {
            scene: SceneState::default(),
            limits: Limits::default(),
            clients: BTreeMap::new(),
            next_client: 1,
            now_ms: 0,
            assets: BTreeMap::new(),
            writable: BTreeMap::new(),
        }
    }
}

impl ReferenceHost {
    pub fn register(&mut self, engine: &str, game: &str) -> Result<ClientId> {
        if engine.is_empty() || game.is_empty() || engine.len() > 256 || game.len() > 256 {
            return Err(Error::Invalid("client identity"));
        }
        let id = ClientId(self.next_client);
        self.next_client = self
            .next_client
            .checked_add(1)
            .ok_or(Error::Limit("client ids"))?;
        self.clients.insert(
            id,
            ClientInfo {
                engine: engine.into(),
                game: game.into(),
            },
        );
        Ok(id)
    }
    pub fn client(&self, id: ClientId) -> Result<&ClientInfo> {
        self.clients.get(&id).ok_or(Error::NotFound)
    }
    pub fn set_time(&mut self, now_ms: u64) -> Result<()> {
        if now_ms < self.now_ms {
            return Err(Error::Invalid("clock moved backwards"));
        }
        self.now_ms = now_ms;
        Ok(())
    }
    pub fn mount_asset(&mut self, client: ClientId, path: &VfsPath, bytes: Vec<u8>) -> Result<()> {
        if path.mount() != Mount::Asset {
            return Err(Error::Invalid("not an asset path"));
        }
        let info = self.client(client)?;
        let key = (
            info.engine.clone(),
            info.game.clone(),
            path.relative().into(),
        );
        self.assets.insert(key, bytes);
        Ok(())
    }
}

impl FileSystem for ReferenceHost {
    fn read(&mut self, client: ClientId, path: &VfsPath) -> Result<Vec<u8>> {
        let info = self.client(client)?;
        let value = match path.mount() {
            Mount::Asset => self.assets.get(&(
                info.engine.clone(),
                info.game.clone(),
                path.relative().into(),
            )),
            mount => self.writable.get(&(
                info.engine.clone(),
                info.game.clone(),
                mount,
                path.relative().into(),
            )),
        };
        value.cloned().ok_or(Error::NotFound)
    }
    fn write_atomic(&mut self, client: ClientId, path: &VfsPath, data: &[u8]) -> Result<()> {
        let info = self.client(client)?;
        if path.mount() == Mount::Asset {
            return Err(Error::ReadOnly);
        }
        if data.len() > 64 * 1024 * 1024 {
            return Err(Error::Limit("file size"));
        }
        let key = (
            info.engine.clone(),
            info.game.clone(),
            path.mount(),
            path.relative().into(),
        );
        self.writable.insert(key, data.to_vec());
        Ok(())
    }
}

impl HostServices for ReferenceHost {
    fn submit(&mut self, client: ClientId, commands: &[Command]) -> Result<()> {
        self.client(client)?;
        self.scene.apply_batch(client, commands, self.limits)
    }
    fn now_ms(&self) -> u64 {
        self.now_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_compat::{Bridge, CompatibilityProfile, EngineKind};
    use crysirror_core::state::{Color, Rect, TextRun, Visual, VisualNode};
    fn text() -> Command {
        Command::PutVisual {
            id: 0,
            node: VisualNode {
                z: 0,
                visual: Visual::Text(TextRun {
                    text: "synthetic".into(),
                    font: "external-font".into(),
                    size_px: 20,
                    bounds: Rect {
                        x: 0,
                        y: 0,
                        width: 640,
                        height: 40,
                    },
                    color: Color([255; 4]),
                    layout_profile: "fixture".into(),
                }),
            },
        }
    }
    #[test]
    fn two_compatibility_clients_share_services_without_sharing_object_ids() {
        let mut h = ReferenceHost::default();
        for kind in [EngineKind::Rfvp, EngineKind::Siglus] {
            let id = h.register(kind.id(), "synthetic").unwrap();
            let b = Bridge::new(
                id,
                CompatibilityProfile::new(kind, kind.id().into()).unwrap(),
            );
            b.submit(&mut h, &[text()]).unwrap();
        }
        assert_eq!(h.scene.visuals().len(), 2);
    }
    #[test]
    fn save_namespace_includes_engine_and_game() {
        let mut h = ReferenceHost::default();
        let a = h.register("rfvp", "game-a").unwrap();
        let b = h.register("siglus", "game-a").unwrap();
        let c = h.register("rfvp", "game-b").unwrap();
        let path = VfsPath::new(Mount::Save, "slot-1").unwrap();
        h.write_atomic(a, &path, b"rfvp-state").unwrap();
        assert_eq!(h.read(a, &path).unwrap(), b"rfvp-state");
        assert!(h.read(b, &path).is_err());
        assert!(h.read(c, &path).is_err());
    }
    #[test]
    fn assets_are_read_only_to_clients() {
        let mut h = ReferenceHost::default();
        let id = h.register("rfvp", "game-a").unwrap();
        let path = VfsPath::new(Mount::Asset, "image/A").unwrap();
        h.mount_asset(id, &path, b"fixture".to_vec()).unwrap();
        assert_eq!(h.write_atomic(id, &path, b"bad"), Err(Error::ReadOnly));
        assert_eq!(h.read(id, &path).unwrap(), b"fixture");
    }
    #[test]
    fn time_is_monotonic() {
        let mut h = ReferenceHost::default();
        h.set_time(10).unwrap();
        assert!(h.set_time(9).is_err());
    }
    #[test]
    fn unknown_client_is_rejected() {
        assert!(ReferenceHost::default().submit(ClientId(42), &[]).is_err());
    }
}
