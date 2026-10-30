// SPDX-License-Identifier: MPL-2.0
//! CrySirror port change 7: read/drive wrappers so the engine's puppet
//! bridge (crysirror-puppet-eluna) can treat an E-mote player as a generic
//! puppet source. Only delegates to existing player calls; no behaviour of
//! the Siglus embedding changes.

use std::collections::HashMap;

use anyhow::{Context, Result};
use eluna::{EmoteStaticScene, EmoteVariableInfo, TimelinePlayMode, VariableWrite};

use super::{EmoteDecodedTexture, EmoteRuntime};

/// One authored timeline: name, length in E-mote ticks (60/s) and loop flag.
#[derive(Debug, Clone, PartialEq)]
pub struct EmoteTimelineSummary {
    pub name: String,
    pub total_ticks: Option<f32>,
    pub looping: bool,
}

impl EmoteRuntime {
    pub fn variable_infos(&self) -> Vec<EmoteVariableInfo> {
        self.runtime
            .inner
            .variables()
            .values()
            .map(|state| state.info.clone())
            .collect()
    }

    pub fn variable_value(&self, name: &str) -> Option<f32> {
        self.runtime.inner.variable_value(name)
    }

    /// `SetVariable(name, value[, ticks, easing])`, then a scene rebuild.
    pub fn set_variable(&mut self, name: &str, value: f32, ticks: f32, easing: f32) -> Result<()> {
        if ticks > 0.0 {
            self.runtime
                .inner
                .apply_write(VariableWrite::timed(name, value, ticks, easing));
        } else {
            self.runtime.inner.set_variable_immediate(name, value);
        }
        self.runtime
            .rebuild_scene(0.0)
            .with_context(|| format!("Eluna SetVariable({name:?}) failed"))?;
        self.bump_version();
        Ok(())
    }

    pub fn timeline_summaries(&self) -> Vec<EmoteTimelineSummary> {
        let inner = &self.runtime.inner;
        inner
            .timelines()
            .keys()
            .map(|name| EmoteTimelineSummary {
                name: name.clone(),
                total_ticks: inner.timeline_total_frame_count(name),
                looping: inner.is_loop_timeline(name),
            })
            .collect()
    }

    pub fn play_timeline_mode(&mut self, name: &str, mode: TimelinePlayMode) -> Result<()> {
        self.runtime
            .play_timeline(name, mode)
            .with_context(|| format!("Eluna PlayTimeline({name:?}) failed"))?;
        self.bump_version();
        Ok(())
    }

    pub fn is_timeline_playing(&self, name: &str) -> bool {
        self.runtime.inner.is_timeline_playing(name)
    }

    pub fn current_scene(&self) -> &EmoteStaticScene {
        self.runtime.scene()
    }

    pub fn decoded_textures(&self) -> &HashMap<u32, EmoteDecodedTexture> {
        &self.decoded_textures
    }
}
