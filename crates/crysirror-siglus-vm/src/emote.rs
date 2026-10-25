// SPDX-License-Identifier: MPL-2.0
//! Unavailable native backends are uninhabited: no object can silently emulate one.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SiglusEmoteRuntime {}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EmoteRenderPacket {}
impl SiglusEmoteRuntime {
    pub fn progress_ms(&mut self, _: i32) -> anyhow::Result<()> {
        match *self {}
    }
    pub fn is_animating(&self) -> bool {
        match *self {}
    }
    pub fn clone_for_object(&self) -> Self {
        match *self {}
    }
}
impl SiglusEmoteRuntime {
    pub fn pass(&mut self) -> anyhow::Result<()> {
        match *self {}
    }
}
