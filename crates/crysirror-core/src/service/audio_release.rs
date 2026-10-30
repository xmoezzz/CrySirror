// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Owner-clock retired audio voices and bounded stepped logarithmic release.
/// Immutable release policy. Volume units are hundredths of one dB (db100).
/// Source/cursor transfer and all interpolation are owned by the audio service.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioReleaseSpec {
    /// Channel vacated by the live decoder without reopening or seeking.
    pub source_channel: u32,
    /// Independent channel receiving the retired nonlooping voice.
    pub tail_channel: u32,
    /// Captured levels at/above this select target_above_db100.
    pub threshold_db100: i32,
    /// Target for captured levels at or above the threshold.
    pub target_above_db100: i32,
    /// Target for captured levels below the threshold.
    pub target_below_db100: i32,
    /// Divisor of target-start; bounded1..100000. This is not milliseconds.
    pub steps: u32,
    /// Signed stereo pan -1..1; None preserves the current balance.
    pub pan: Option<f32>,
    /// Playback multiplier0.125..4; None preserves the current rate.
    pub playback_rate: Option<f32>,
    /// Bake current native gain-group attenuation into the retired voice.
    /// Engine bus/master controls remain live. False retains the group.
    pub freeze_group_gain: bool,
    /// Without a source, release the existing tail, or complete if both absent.
    /// False rejects an absent source instead of touching the destination.
    pub release_tail_when_source_missing: bool,
}
impl AudioReleaseSpec {
    pub fn validate(&self) -> crate::Result<()> {
        if self.source_channel == self.tail_channel
            || self.source_channel > 65535
            || self.tail_channel > 65535
        {
            return Err(crate::Error::Invalid("audio release channels"));
        }
        if self.steps == 0
            || self.steps > 100000
            || [
                self.threshold_db100,
                self.target_above_db100,
                self.target_below_db100,
            ]
            .iter()
            .any(|&v| !(-10000..=0).contains(&v))
        {
            return Err(crate::Error::Invalid("audio release bounds"));
        }
        if self.target_above_db100 > self.threshold_db100
            || self.target_below_db100 > self.threshold_db100
        {
            return Err(crate::Error::Invalid(
                "audio release cannot amplify below threshold",
            ));
        }
        if self
            .pan
            .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
            || self
                .playback_rate
                .is_some_and(|v| !v.is_finite() || !(0.125..=4.0).contains(&v))
        {
            return Err(crate::Error::Invalid("audio release pan/rate"));
        }
        Ok(())
    }
}
/// Client capability whose generation is not reused across resets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioReleaseHandle {
    /// Owning client namespace.
    pub client: crate::state::ClientId,
    /// Unique owner-issued operation identity.
    pub generation: u64,
}
/// Publication phase of the actual release actor, never a declaration alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioReleasePhase {
    /// A render owner or preparation dependency still holds the source.
    Pending,
    /// Transfer and current gain have been committed by the audio owner.
    Active,
    /// Stop or source exhaustion has completed at an owner boundary.
    Complete,
    /// Another owner control replaced this voice/track.
    Cancelled,
}
/// Read-only owner-published completion; no device or decoder pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioReleaseInfo {
    /// Actual actor phase.
    pub phase: AudioReleasePhase,
    /// Latest monotonic external update requested by the caller.
    pub submitted_updates: u64,
    /// External update through which work has actually been owner-committed.
    pub completed_updates: u64,
    /// Current integer hundredths of a dB, truncated toward zero.
    pub volume_db100: Option<i32>,
    /// Last consumed source position, excluding decoder read-ahead.
    pub source_position_frames: Option<u64>,
    /// First update that stopped the voice; zero means silent at admission.
    pub stopped_at_update: Option<u64>,
}
