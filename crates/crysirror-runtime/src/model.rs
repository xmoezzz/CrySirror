// SPDX-License-Identifier: MPL-2.0
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    state::{AudioVoice, Color, Command, Rect, Sprite, TextRun, Visual, VisualNode},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs::File, io::Read, path::Path};

pub const MAX_PROJECT_BYTES: usize = 4 * 1024 * 1024;
pub const SIMPLE_LAYOUT: &str = crysirror_core::service::SIMPLE_TEXT_LAYOUT;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub schema: u32,
    pub name: String,
    pub content_revision: String,
    pub canvas: [u32; 2],
    #[serde(default = "background")]
    pub background: [u8; 4],
    pub assets: String,
    #[serde(default)]
    pub fonts: std::collections::BTreeMap<String, String>,
    pub clients: Vec<ClientSpec>,
    #[serde(default)]
    pub scene3d: Option<Scene3d>,
}
fn background() -> [u8; 4] {
    [16, 20, 30, 255]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientSpec {
    pub name: String,
    /// A factory name. "siglus" is built in; hosts register other original
    /// interpreters such as "rfvp" and "bgi" through their adapter crates.
    pub adapter: String,
    pub game: String,
    #[serde(default)]
    pub asset_prefix: String,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub program: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub systems: Option<crysirror_scene::SystemsSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Sprite {
        id: u32,
        asset: String,
        rect: [i32; 4],
        z: i32,
        #[serde(default = "white")]
        tint: [u8; 4],
    },
    Text {
        id: u32,
        text: String,
        font: String,
        size: u16,
        rect: [i32; 4],
        z: i32,
        #[serde(default = "white")]
        color: [u8; 4],
        #[serde(default = "layout")]
        layout: String,
    },
    Remove {
        id: u32,
    },
    Play {
        channel: u32,
        asset: String,
        #[serde(default)]
        looping: bool,
        #[serde(default = "gain")]
        gain_milli: u16,
    },
    Stop {
        channel: u32,
    },
    Wait {
        ms: u64,
    },
    WaitInput,
    Halt,
}
fn white() -> [u8; 4] {
    [255; 4]
}
fn gain() -> u16 {
    1000
}
fn layout() -> String {
    SIMPLE_LAYOUT.into()
}

fn rect(v: [i32; 4]) -> Result<Rect> {
    ensure!(
        v[2] > 0 && v[3] > 0 && v[2] <= 16384 && v[3] <= 16384,
        "rectangle dimensions must be 1..16384"
    );
    ensure!(
        v[0].abs_diff(0) <= 1_000_000 && v[1].abs_diff(0) <= 1_000_000,
        "rectangle coordinates too large"
    );
    Ok(Rect {
        x: v[0],
        y: v[1],
        width: v[2] as u32,
        height: v[3] as u32,
    })
}
fn path(s: &str) -> Result<VfsPath> {
    VfsPath::new(Mount::Asset, s).map_err(|e| anyhow::anyhow!("{e}"))
}

impl Action {
    pub fn command(&self) -> Result<Option<Command>> {
        Ok(Some(match self {
            Self::Sprite {
                id,
                asset,
                rect: bounds,
                z,
                tint,
            } => Command::PutVisual {
                id: *id,
                node: VisualNode {
                    z: *z,
                    visual: Visual::Sprite(Sprite {
                        asset: path(asset)?,
                        bounds: rect(*bounds)?,
                        tint: Color(*tint),
                    }),
                },
            },
            Self::Text {
                id,
                text,
                font,
                size,
                rect: bounds,
                z,
                color,
                layout,
            } => {
                ensure!(
                    !font.is_empty() && *size > 0 && *size <= 512,
                    "invalid font/size"
                );
                ensure!(
                    text.len() <= 1024 * 1024 && !layout.is_empty(),
                    "invalid text/layout"
                );
                Command::PutVisual {
                    id: *id,
                    node: VisualNode {
                        z: *z,
                        visual: Visual::Text(TextRun {
                            text: text.clone(),
                            font: font.clone(),
                            size_px: *size,
                            bounds: rect(*bounds)?,
                            color: Color(*color),
                            layout_profile: layout.clone(),
                        }),
                    },
                }
            }
            Self::Remove { id } => Command::RemoveVisual { id: *id },
            Self::Play {
                channel,
                asset,
                looping,
                gain_milli,
            } => {
                ensure!(*gain_milli <= 1000, "audio gain exceeds 1000");
                Command::PlayAudio {
                    channel: *channel,
                    voice: AudioVoice {
                        asset: path(asset)?,
                        looping: *looping,
                        gain_milli: *gain_milli,
                    },
                }
            }
            Self::Stop { channel } => Command::StopAudio { channel: *channel },
            Self::Wait { ms } => {
                ensure!(*ms <= 86_400_000, "wait exceeds one day");
                return Ok(None);
            }
            Self::WaitInput | Self::Halt => return Ok(None),
        }))
    }
    pub fn from_visual(id: u32, node: &VisualNode) -> Self {
        let r = |r: Rect| [r.x, r.y, r.width as i32, r.height as i32];
        match &node.visual {
            Visual::Sprite(s) => Self::Sprite {
                id,
                asset: s.asset.relative().into(),
                rect: r(s.bounds),
                z: node.z,
                tint: s.tint.0,
            },
            Visual::Text(t) => Self::Text {
                id,
                text: t.text.clone(),
                font: t.font.clone(),
                size: t.size_px,
                rect: r(t.bounds),
                z: node.z,
                color: t.color.0,
                layout: t.layout_profile.clone(),
            },
        }
    }
}

pub use crysirror_scene::{Camera, Geometry, Mesh, Scene3d};

pub fn identity(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
impl Project {
    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)
            .with_context(|| format!("open {}", path.display()))?
            .take(MAX_PROJECT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_PROJECT_BYTES, "project exceeds 4 MiB");
        let p: Self = serde_json::from_slice(&bytes).context("project JSON")?;
        p.validate()?;
        Ok(p)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 2, "unsupported desktop schema (expected 2)");
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 256
                && !self.name.chars().any(char::is_control),
            "project name"
        );
        ensure!(
            !self.content_revision.is_empty()
                && self.content_revision.len() <= 256
                && !self.content_revision.chars().any(char::is_control),
            "content revision"
        );
        ensure!(
            self.canvas.iter().all(|v| (1..=8192).contains(v)),
            "canvas dimensions"
        );
        path(&self.assets)?;
        ensure!(
            !self.clients.is_empty() && self.clients.len() <= 64,
            "client count"
        );
        let mut names = BTreeSet::new();
        let mut bgi_save_writers = BTreeSet::new();
        let mut systems = crysirror_scene::EngineSystems::default();
        for (i, c) in self.clients.iter().enumerate() {
            ensure!(
                identity(&c.name) && names.insert(&c.name),
                "client names must be unique safe identifiers"
            );
            ensure!(
                identity(&c.game) && identity(&c.adapter),
                "game/adapter identifier"
            );
            if !c.asset_prefix.is_empty() {
                path(&c.asset_prefix)?;
            }
            if c.adapter == "siglus" {
                ensure!(
                    c.program.is_empty(),
                    "Siglus uses original bytecode, not data VM actions"
                );
                let config: crysirror_siglus_vm::adapter::Config =
                    serde_json::from_value(c.config.clone())?;
                config.validate()?;
            }
            if c.adapter == "bgi" {
                ensure!(c.program.is_empty(), "BGI uses original bytecode, not data VM actions");
                ensure!(bgi_save_writers.insert(&c.game),
                    "BGI does not support simultaneous clients writing the same game's save namespace");
            }
            ensure!(c.program.len() <= 100_000, "program length");
            for a in &c.program {
                a.command()?;
            }
            if let Some(spec) = &c.systems {
                systems.install(crysirror_core::state::ClientId((i + 1) as u32), spec)?;
            }
        }
        for (alias, file) in &self.fonts {
            ensure!(identity(alias), "font alias");
            path(file)?;
        }
        if let Some(scene) = &self.scene3d {
            scene.validate()?;
        }
        systems.world.render_scene(self.scene3d.as_ref())?;
        systems.frame(self.canvas)?;
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<String> {
        Ok(blake3::hash(&serde_json::to_vec(self)?)
            .to_hex()
            .to_string())
    }
    pub fn require_builtin_adapters(&self) -> Result<()> {
        for c in &self.clients {
            if !matches!(c.adapter.as_str(), "crysirror-script-v1" | "siglus") {
                bail!("native adapter '{}' is not linked; fetching its repository does not connect its VM", c.adapter);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn demo() -> Project {
        serde_json::from_str(include_str!("../../../examples/desktop/project.json")).unwrap()
    }
    #[test]
    fn demo_is_valid() {
        demo().validate().unwrap();
    }
    #[test]
    fn unknown_fields_rejected() {
        let mut v = serde_json::to_value(demo()).unwrap();
        v["typo"] = true.into();
        assert!(serde_json::from_value::<Project>(v).is_err());
    }
    #[test]
    fn unknown_native_adapter_is_not_a_demo_vm() {
        let mut p = demo();
        p.clients[0].adapter = "rfvp".into();
        p.validate().unwrap();
        assert!(p.require_builtin_adapters().is_err());
    }
    #[test]
    fn paths_are_confined() {
        let mut p = demo();
        p.assets = "../game".into();
        assert!(p.validate().is_err());
    }
    #[test]
    fn camera_parallel_up_rejected() {
        let mut p = demo();
        let c = &mut p.scene3d.as_mut().unwrap().camera;
        c.up = [0.; 3];
        assert!(p.validate().is_err());
    }
    #[test]
    fn duplicate_clients_rejected() {
        let mut p = demo();
        p.clients.push(p.clients[0].clone());
        assert!(p.validate().is_err());
    }
    #[test]
    fn action_rejects_negative_size() {
        let a = Action::Sprite {
            id: 0,
            asset: "a.png".into(),
            rect: [0, 0, -1, 5],
            z: 0,
            tint: [255; 4],
        };
        assert!(a.command().is_err());
    }
    #[test]
    fn index_validation() {
        let mut p = demo();
        p.scene3d.as_mut().unwrap().meshes[0].geometry = Geometry::Triangles {
            vertices: vec![[0.; 3]],
            indices: vec![0, 0, 1],
        };
        assert!(p.validate().is_err());
    }
}
