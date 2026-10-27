// SPDX-License-Identifier: MPL-2.0
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct AudioHub;
pub use crate::bgm_bridge::BgmEngine;
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct KoeEngine {
    volume: u8,
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct PcmEngine {
    volume: u8,
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SeEngine {
    volume: u8,
}

impl KoeEngine {
    pub fn volume_raw(&self) -> u8 {
        self.volume
    }
    pub fn set_volume_raw(&mut self, _: &mut AudioHub, v: u8) -> anyhow::Result<()> {
        self.volume = v;
        Ok(())
    }
}

impl PcmEngine {
    pub fn volume_raw(&self) -> u8 {
        self.volume
    }
    pub fn set_volume_raw(&mut self, _: &mut AudioHub, v: u8) -> anyhow::Result<()> {
        self.volume = v;
        Ok(())
    }
}

impl SeEngine {
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
impl SeEngine {
    pub fn stop(&mut self, _: Option<i64>) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn is_playing_any(&mut self) -> bool {
        false
    }
}
impl PcmEngine {
    pub fn stop_all(&mut self, _: Option<i64>) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn is_playing_any(&mut self) -> bool {
        false
    }
    pub fn is_playing_slot(&mut self, _: usize) -> bool {
        false
    }
    pub fn is_fading_slot(&mut self, _: usize) -> bool {
        false
    }
    pub fn slot_volume_raw(&self, _: usize) -> i32 {
        1000
    }
    pub fn slot_resume_delay_ms(&self, _: usize) -> i64 {
        0
    }
}

impl Default for KoeEngine {
    fn default() -> Self {
        Self { volume: 255 }
    }
}

impl Default for PcmEngine {
    fn default() -> Self {
        Self { volume: 255 }
    }
}

impl Default for SeEngine {
    fn default() -> Self {
        Self { volume: 255 }
    }
}
