// SPDX-License-Identifier: MPL-2.0
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct AudioHub;
pub use crate::bgm_bridge::BgmEngine;
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct KoeEngine {
    volume: u8,
}
pub use crate::sfx_bridge::{PcmEngine, SeEngine};

impl KoeEngine {
    pub fn volume_raw(&self) -> u8 {
        self.volume
    }
    pub fn set_volume_raw(&mut self, _: &mut AudioHub, v: u8) -> anyhow::Result<()> {
        self.volume = v;
        Ok(())
    }
}

impl KoeEngine {
    pub fn stop(&mut self, _: Option<i64>) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn is_playing_any(&mut self) -> bool {
        false
    }
}
impl Default for KoeEngine {
    fn default() -> Self {
        Self { volume: 255 }
    }
}
