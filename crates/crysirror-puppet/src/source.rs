// SPDX-License-Identifier: MPL-2.0
//! The interface the engine drives. Rigs implement it natively; other
//! puppet runtimes implement it in their own bridge crates and only supply
//! frames and parameter values.
use crate::{eval::Puppet, PuppetFrame};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub id: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Values where the source has authored keys (sampling hints for
    /// [`crate::bake`]); empty when unknown.
    pub keys: Vec<f32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionInfo {
    pub id: String,
    /// `None` when the source cannot tell.
    pub duration_ms: Option<f32>,
    pub looping: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayOptions {
    /// `None` keeps the motion's own setting.
    pub looping: Option<bool>,
    pub weight: f32,
}
impl Default for PlayOptions {
    fn default() -> Self {
        Self { looping: None, weight: 1. }
    }
}

/// A running 2D puppet. Errors are visible: unknown parameters and motions
/// fail instead of being ignored.
pub trait PuppetSource {
    /// Canvas size of the frames.
    fn size(&self) -> [f32; 2];
    fn parameters(&self) -> Vec<ParameterInfo>;
    fn motions(&self) -> Vec<MotionInfo>;
    /// Current (evaluated) value.
    fn parameter_value(&self, id: &str) -> Option<f32>;
    fn set_parameter(&mut self, id: &str, value: f32, duration_ms: f32) -> Result<()>;
    fn release_parameter(&mut self, id: &str) -> Result<()>;
    fn play(&mut self, motion: &str, options: PlayOptions) -> Result<()>;
    /// An empty id stops every motion.
    fn stop(&mut self, motion: &str) -> Result<()>;
    fn advance(&mut self, dt_ms: f32) -> Result<()>;
    fn frame(&self) -> Result<PuppetFrame>;
    /// Texture names indexed by `FramePart::texture`.
    fn texture_names(&self) -> Vec<String>;
    /// Decoded RGBA8 pixels of a texture, for sources that own their pixels
    /// (rigs reference image files instead and return `None`).
    fn texture_rgba(&self, _index: u32) -> Option<(u32, u32, Vec<u8>)> {
        None
    }
}

impl PuppetSource for Puppet {
    fn size(&self) -> [f32; 2] {
        self.rig().size
    }
    fn parameters(&self) -> Vec<ParameterInfo> {
        let rig = self.rig();
        rig.parameters
            .iter()
            .map(|p| {
                let mut keys: Vec<f32> = rig
                    .parts
                    .iter()
                    .flat_map(|x| &x.keyforms)
                    .filter(|k| k.parameter == p.id)
                    .flat_map(|k| k.keys.iter().map(|k| k.value))
                    .chain(rig.deformers.iter().flat_map(|d| &d.keyforms).filter(|k| k.parameter == p.id).flat_map(|k| k.keys.iter().map(|k| k.value)))
                    .collect();
                keys.sort_by(f32::total_cmp);
                keys.dedup();
                ParameterInfo { id: p.id.clone(), min: p.min, max: p.max, default: p.default, keys }
            })
            .collect()
    }
    fn motions(&self) -> Vec<MotionInfo> {
        self.rig()
            .motions
            .iter()
            .map(|m| MotionInfo { id: m.id.clone(), duration_ms: Some(m.duration_ms), looping: m.looping })
            .collect()
    }
    fn parameter_value(&self, id: &str) -> Option<f32> {
        self.value(id)
    }
    fn set_parameter(&mut self, id: &str, value: f32, duration_ms: f32) -> Result<()> {
        Puppet::set_parameter(self, id, value, duration_ms)
    }
    fn release_parameter(&mut self, id: &str) -> Result<()> {
        Puppet::release_parameter(self, id)
    }
    fn play(&mut self, motion: &str, options: PlayOptions) -> Result<()> {
        Puppet::play(self, motion, options.looping, options.weight)
    }
    fn stop(&mut self, motion: &str) -> Result<()> {
        Puppet::stop(self, motion)
    }
    fn advance(&mut self, dt_ms: f32) -> Result<()> {
        Puppet::advance(self, dt_ms)
    }
    fn frame(&self) -> Result<PuppetFrame> {
        Ok(Puppet::frame(self))
    }
    fn texture_names(&self) -> Vec<String> {
        self.rig().textures.clone()
    }
}
