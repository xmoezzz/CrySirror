// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Explicit asset loop metadata with a declared timebase. A decoded frame
//! contains one sample for each source channel, before resampling.
use crate::state::ClientId;
use alloc::string::String;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioLoopTimebase {
    DecodedFrames,
    /// Coordinates counted at this fixed reference rate. Callers explicitly
    /// resolve them for a source rate; no playback path automatically applies it.
    ReferenceFrames {
        sample_rate: u32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioLoopRange {
    pub timebase: AudioLoopTimebase,
    pub start_frame: u64,
    /// Exclusive end; None means the source's decoded end.
    pub end_frame: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioLoopMetadata {
    /// Canonical asset-relative path; names are exact, not implicitly case-folded.
    pub asset: String,
    pub range: AudioLoopRange,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioLoopCatalogReceipt {
    pub client: ClientId,
    pub revision: u64,
    pub entries: u32,
}
