// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Device-free decoder/header preparation, with no playback resource or PCM.
use crate::{state::ClientId, Result};
use alloc::{boxed::Box, string::String};
#[derive(Clone, Debug, PartialEq)]
pub struct MediaProbeInfo {
    pub client: ClientId,
    pub size: [u32; 2],
    pub container: String,
    /// Selected engine video decoder family, not a claim about every packet.
    pub video_decoder: String,
    pub header_validated: bool,
    /// Unknown audio identity stays None; no PCM is decoded by this operation.
    pub audio_decoder: Option<String>,
    pub has_audio: Option<bool>,
    pub duration_ms: Option<u64>,
    pub frame_ms: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum MediaProbeState {
    Pending,
    Ready(MediaProbeInfo),
    Failed(String),
}
/// Poll never joins. Completion is retained; dropping the job cancels work.
pub trait MediaProbe: Send {
    fn client(&self) -> ClientId;
    fn poll(&mut self) -> Result<MediaProbeState>;
}
pub type MediaProbeJob = Box<dyn MediaProbe>;
