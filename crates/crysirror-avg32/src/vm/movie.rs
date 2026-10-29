// SPDX-License-Identifier: MPL-2.0
//! CrySirror: AVG32 Cinepak AVI playback is not mapped onto the shared video
//! service yet. The scenario keeps its original movie commands; parsing
//! fails visibly (the scenario reports it as a warning) instead of decoding
//! frames with a private codec.
use anyhow::{Result, bail};
use crysirror_image::avg32::{PdtManager, Rect};

pub struct AviMovie {
    pub audio: Option<AviAudio>,
}

pub struct AviAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub pcm: Vec<u8>,
}

impl AviMovie {
    pub fn parse(_bytes: &[u8]) -> Result<Self> {
        bail!("AVG32 Cinepak movies need a shared engine video decoder")
    }
}

#[derive(Debug)]
pub struct MoviePlayer;

impl MoviePlayer {
    pub fn start(
        _movie: &AviMovie,
        _looped: bool,
        _rect: Rect,
        _gfx: &mut PdtManager,
    ) -> Result<Option<Self>> {
        bail!("AVG32 Cinepak movies need a shared engine video decoder")
    }

    pub fn active(&self) -> bool {
        false
    }

    pub fn stop(&mut self) {}

    pub fn tick(&mut self, _gfx: &mut PdtManager) -> Result<()> {
        Ok(())
    }
}
