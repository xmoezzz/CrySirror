// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! getSample.dll: WaveSoundBuffer sample accessors used for lip sync.
//!
//! Surface and algorithms follow wtnbgo/getSample `main.cpp`:
//! `getSample([n=100])` averages the non-negative mono int16 samples at the
//! play cursor (0 when none); `sampleValue` is the largest `(s/32768)^2` of
//! `sampleCount` samples taken `sampleAhead` frames ahead; both window sizes
//! default from `setDefaultCounts` / `setDefaultAheads` (100 / 0) when an
//! object first uses them. Linking sets `useVisBuffer` on first use, as the
//! reference add-on constructor does.
//!
//! CrySirror change: the samples come from the shared source-PCM service
//! (`HostServices::begin_audio_visual_samples`) on a worker. Script callbacks cannot
//! wait for it, so each call answers from the latest completed window for
//! that buffer (the reference's "no samples" 0 before the first one) and
//! schedules a read when the cursor leaves the prepared PCM range.
//! A stopped buffer answers 0 like the reference.
use crate::natives::{Rt, this_of};
use crysirror_core::{
    service::{AudioResourceHandle, AudioSampleQuery, AudioSampleRequest, AudioSampleState, HostServices},
    state::ClientId,
};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use std::collections::BTreeMap;

const DEFAULT_COUNT: i64 = 100;
const DEFAULT_AHEAD: i64 = 0;
const MAX_COUNT: i64 = 65_536;

#[derive(Default)]
pub(crate) struct GetSample {
    defaults: Option<(i64, i64)>,
    objects: BTreeMap<usize, Window>,
}

#[derive(Default)]
struct Window {
    count: i64,
    ahead: i64,
    /// Requested `(resource, start frame, frames)` not yet read.
    want: Option<(AudioResourceHandle, u64, u32)>,
    job: Option<(u32, Box<dyn AudioSampleQuery>)>,
    resource: Option<AudioResourceHandle>,
    first: u64,
    eof: bool,
    /// Contiguous mono PCM from the independent visualization decoder.
    samples: Vec<i16>,
}

impl GetSample {
    pub(crate) fn invalidate(&mut self, object: usize) {
        self.objects.remove(&object);
    }
}

impl Window {
    fn observe_resource(&mut self, resource: Option<AudioResourceHandle>) {
        if self.resource != resource {
            self.resource = resource;
            self.want = None;
            self.job = None;
            self.samples.clear();
            self.first = 0;
            self.eof = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_voice_discards_the_pending_lip_sync_request() {
        let old = AudioResourceHandle { client: 1, id: 1 };
        let new = AudioResourceHandle { client: 1, id: 2 };
        let mut window = Window {
            resource: Some(old), want: Some((old, 16, 100)),
            samples: vec![123], first: 16, eof: true, ..Default::default()
        };
        window.observe_resource(Some(new));
        assert!(window.want.is_none());
        assert!(window.samples.is_empty());
        assert_eq!(window.first, 0);
        assert!(!window.eof);
    }
}

pub(crate) fn install(rt: &mut Rt) {
    let Some(class) = rt.global_member("WaveSoundBuffer").object_handle() else { return };
    rt.register_object_native(class, "getSample", |rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>| {
        let h = this_of(rt, t, "WaveSoundBuffer.getSample")?;
        let n = match a.first() {
            None | Some(Variant::Void) => DEFAULT_COUNT,
            Some(v) => v.to_integer()?,
        };
        let samples = window(rt, h, n, 0)?;
        let positive: Vec<i64> = samples.iter().filter(|s| **s >= 0).map(|s| i64::from(*s)).collect();
        Ok(Variant::Integer(if positive.is_empty() { 0 } else { positive.iter().sum::<i64>() / positive.len() as i64 }))
    });
    rt.register_object_native_property(
        class,
        "sampleValue",
        |rt, t| {
            let h = this_of(rt, t, "WaveSoundBuffer.sampleValue")?;
            let (count, ahead) = state(rt, h);
            let samples = window(rt, h, count, ahead)?;
            let full = 32768.0f64;
            Ok(Variant::Real(samples.iter().map(|s| (f64::from(*s) / full).powi(2)).fold(0.0, f64::max)))
        },
        |_, _, _| Err(TjsError::runtime("WaveSoundBuffer.sampleValue is read-only")),
    );
    for (name, ahead) in [("sampleCount", false), ("sampleAhead", true)] {
        rt.register_object_native_property(
            class,
            name,
            move |rt, t| {
                let h = this_of(rt, t, name)?;
                let (count, a) = state(rt, h);
                Ok(Variant::Integer(if ahead { a } else { count }))
            },
            move |rt, t, v| {
                let h = this_of(rt, t, name)?;
                let v = v.to_integer()?;
                if !(0..=MAX_COUNT).contains(&v) {
                    return Err(TjsError::runtime(format!("WaveSoundBuffer.{name} out of range")));
                }
                state(rt, h);
                let w = rt.host_mut().get_sample.objects.get_mut(&h.0).unwrap();
                if ahead { w.ahead = v } else { w.count = v }
                Ok(())
            },
        );
    }
    for (name, ahead) in [("setDefaultCounts", false), ("setDefaultAheads", true)] {
        rt.register_object_native(class, name, move |rt: &mut Rt, _, a: Vec<Variant>| {
            let v = a.first().ok_or_else(TjsError::bad_param_count)?.to_integer()?;
            let s = &mut rt.host_mut().get_sample;
            let (count, a) = s.defaults.get_or_insert((DEFAULT_COUNT, DEFAULT_AHEAD));
            if ahead { *a = v } else { *count = v }
            Ok(Variant::Void)
        });
    }
}

/// The object's add-on state, created from the statics on first use.
fn state(rt: &mut Rt, h: ObjectHandle) -> (i64, i64) {
    let (count, ahead) = rt.host().get_sample.defaults.unwrap_or((DEFAULT_COUNT, DEFAULT_AHEAD));
    if !rt.host().get_sample.objects.contains_key(&h.0) {
        rt.set_object_member(h, "__wave_vis_buffer", Variant::Integer(1));
    }
    let w = rt.host_mut().get_sample.objects.entry(h.0).or_insert_with(|| Window { count, ahead, ..Window::default() });
    (w.count, w.ahead)
}

/// Latest completed mono window for `h`; schedules the next read.
fn window(rt: &mut Rt, h: ObjectHandle, count: i64, ahead: i64) -> Result<Vec<i16>> {
    if !(0..=MAX_COUNT).contains(&count) || !(0..=1 << 30).contains(&ahead) {
        return Err(TjsError::runtime("getSample window out of range"));
    }
    state(rt, h);
    let source = crate::sound::sample_source(rt, h)?;
    let w = rt.host_mut().get_sample.objects.get_mut(&h.0).unwrap();
    let resource = source.map(|(handle, _)| handle);
    w.observe_resource(resource);
    let Some((resource, start)) = source else {
        return Ok(Vec::new());
    };
    let start = start.saturating_add(ahead as u64);
    let end = w.first.saturating_add(w.samples.len() as u64);
    if start >= w.first && (w.eof || start.saturating_add(count as u64) <= end) {
        w.want = None;
        let offset = start.min(end) - w.first;
        return Ok(w.samples[offset as usize..].iter().copied().take(count as usize).collect());
    }
    w.want = Some((resource, start, count as u32));
    Ok(w.samples.iter().copied().take(count as usize).collect())
}

/// Start and complete the scheduled reads; never blocks the VM.
pub(crate) fn poll(rt: &mut Rt, h: &mut dyn HostServices, client: ClientId) -> anyhow::Result<()> {
    let host = rt.host_mut();
    for (id, w) in &mut host.get_sample.objects {
        // Fast-forward can replace the voice after a lip-sync read requested
        // samples but before this poll starts or publishes that read.
        let active = host.sounds.buffers.get(id).and_then(|buffer| buffer.playing_resource());
        w.observe_resource(active);
        if w.job.is_none()
            && let Some((resource, start, frames)) = w.want.take()
            && frames > 0
        {
            // Lip-sync windows are short and overlap as the play cursor moves.
            let frames = frames.max(4096);
            w.job = Some((frames, h.begin_audio_visual_samples(client, resource, AudioSampleRequest { start_frame: start, max_bytes: frames * 4 })?));
        }
        let Some((requested, job)) = w.job.as_mut() else { continue };
        if let AudioSampleState::Ready(receipt) = job.poll()? {
            w.eof = receipt.frames < *requested;
            w.first = receipt.start_frame;
            w.job = None;
            let channels = usize::from(receipt.format.channels);
            let wide = receipt.format.bits_per_sample == 16;
            let width = if wide { 2 } else { 1 };
            w.samples = receipt
                .bytes
                .chunks_exact(channels * width)
                .map(|frame| {
                    let sum: i32 = frame
                        .chunks_exact(width)
                        .map(|s| if wide { i32::from(i16::from_le_bytes([s[0], s[1]])) } else { (i32::from(s[0]) - 128) << 8 })
                        .sum();
                    (sum / channels as i32) as i16
                })
                .collect();
        }
    }
    Ok(())
}
