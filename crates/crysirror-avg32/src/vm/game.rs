//! Resource opening for AVG32 installations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::vm::ini::Ini;
use crysirror_image::pdt::{PdtImage, decode_pdt};
use crate::vm::resource::{Avg32Resources, find_case_insensitive};
use crate::vm::scene::Avg32SceneHeader;

/// CrySirror: the project names the adapter explicitly, so engine detection
/// is replaced by locating `Gameexe.ini` in the configured game root.
#[derive(Debug, Clone)]
pub struct GameLayout {
    pub root: PathBuf,
    pub gameexe_ini: Option<PathBuf>,
}

pub fn detect_game_root(root: impl AsRef<Path>) -> Result<GameLayout> {
    let root = root.as_ref().to_path_buf();
    let gameexe_ini = find_case_insensitive(&root, Path::new("GAMEEXE.INI"))
        .context("AVG32 game root has no Gameexe.ini")?;
    Ok(GameLayout {
        root,
        gameexe_ini: Some(gameexe_ini),
    })
}

#[derive(Debug)]
pub struct Avg32Game {
    pub layout: GameLayout,
    pub ini: Ini,
    pub resources: Avg32Resources,
}

/// Decodes a text file: UTF-8 when valid, then the NLS encoding, then
/// Shift-JIS.
pub fn decode_text(bytes: &[u8]) -> String {
    crate::vm::nls::decode_file_text(bytes)
}

impl Avg32Game {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let layout = detect_game_root(root)?;
        let gameexe_path = layout
            .gameexe_ini
            .as_ref()
            .expect("AVG32 layout has Gameexe.ini");
        let gameexe = decode_text(
            &crate::vm::fs::read(gameexe_path)
                .with_context(|| format!("failed to read {}", gameexe_path.display()))?,
        );
        let setup = find_case_insensitive(&layout.root, Path::new("SETUP.INI"))
            .and_then(|path| crate::vm::fs::read(path).ok())
            .map(|bytes| decode_text(&bytes));
        let ini = Ini::parse(&gameexe, setup.as_deref());
        let resources = Avg32Resources::from_ini(&layout.root, &ini);
        Ok(Self {
            layout,
            ini,
            resources,
        })
    }

    /// Scene names in the scenario archive (or loose `SEEN###.TXT` files).
    pub fn scene_names(&self) -> Vec<String> {
        let Some(route) = self.resources.route("TXT") else {
            return Vec::new();
        };
        if let Some(archive) = &route.archive {
            if let Some(path) = find_case_insensitive(&route.directory, archive) {
                if let Ok(archive) = crate::vm::archive::PaclArchive::from_path(path) {
                    return archive
                        .entries()
                        .iter()
                        .map(|entry| entry.name.clone())
                        .collect();
                }
            }
        }
        let mut names: Vec<String> = crate::vm::fs::list(&route.directory)
            .into_iter()
            .flatten()
            .filter(|name| {
                let upper = name.to_ascii_uppercase();
                upper.starts_with("SEEN") && upper.ends_with(".TXT") && upper.len() > 8
            })
            .collect();
        names.sort();
        names
    }

    pub fn read_scene(&self, name: &str) -> Result<Vec<u8>> {
        self.resources.read("TXT", name)
    }

    /// `SEEN%03d.TXT` for scene number `seen`.
    pub fn read_seen(&self, seen: i32) -> Result<Vec<u8>> {
        self.read_scene(&format!("SEEN{seen:03}.TXT"))
    }

    pub fn scene_header(&self, name: &str) -> Result<Avg32SceneHeader> {
        Avg32SceneHeader::parse(&self.read_scene(name)?)
            .with_context(|| format!("failed to parse AVG32 scene {name}"))
    }

    /// The `#SEEN_START` scene name.
    pub fn configured_start_scene(&self) -> Option<String> {
        let name = format!("SEEN{:03}.TXT", self.ini.start_seen);
        self.resources.exists("TXT", &name).then_some(name)
    }

    pub fn load_pdt(&self, name: &str) -> Result<PdtImage> {
        decode_pdt(&self.resources.read("PDT", name)?)
            .with_context(|| format!("failed to decode PDT {name}"))
    }
}
