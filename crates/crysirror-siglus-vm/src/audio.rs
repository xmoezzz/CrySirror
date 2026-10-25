// SPDX-License-Identifier: MPL-2.0
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct AudioHub;
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct BgmEngine {
    volume: u8,
}
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

impl BgmEngine {
    pub fn volume_raw(&self) -> u8 {
        self.volume
    }
    pub fn set_volume_raw(&mut self, _: &mut AudioHub, v: u8) -> anyhow::Result<()> {
        self.volume = v;
        Ok(())
    }
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
impl BgmEngine {
    pub fn current_name(&self) -> Option<&str> {
        None
    }
    pub fn save_delay_time_ms(&self) -> i64 {
        0
    }
    pub fn save_loop_flag(&self) -> bool {
        false
    }
    pub fn save_pause_flag(&self) -> bool {
        false
    }
    pub fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn is_playing(&mut self) -> bool {
        false
    }
    pub fn is_fade_out_doing(&mut self) -> bool {
        false
    }
    pub fn play_name_script(
        &mut self,
        _: &mut AudioHub,
        _: &str,
        _: bool,
        _: i64,
        _: i64,
        _: i64,
        _: bool,
        _: i64,
    ) -> anyhow::Result<()> {
        anyhow::bail!("native BGM not bound")
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

impl Default for BgmEngine {
    fn default() -> Self {
        Self { volume: 255 }
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
