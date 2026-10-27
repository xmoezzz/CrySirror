// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native event semantic commit/continuation gate over the shared cue service.
//! This module contains no timer, random-selection or interval algorithm.
use super::*;
use crate::runtime::globals::PcmEventState;
use crysirror_core::service::{AudioSequenceCursor, AudioSequenceHandle};
use std::collections::BTreeMap;
pub(super) type Key = (u32, usize);
#[derive(Serialize, Deserialize)]
#[serde(remote = "AudioSequenceCursor")]
struct Cursor {
    active: bool,
    elapsed_ms: u64,
    next_ms: i64,
    last_index: Option<usize>,
    rng_state: u32,
    emissions: u64,
    frame_emissions: u32,
    budget_exhausted: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    declaration: PcmEventState,
    #[serde(with = "Cursor")]
    cursor: AudioSequenceCursor,
}
/// Serializable execution intent, not a worker ticket. A pending emission is
/// saved at its pre-selection cursor and replayed without ticking the frame twice.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Frame {
    keys: Vec<Key>,
    index: usize,
    advanced: bool,
    delta: u64,
}
struct Await {
    key: Key,
    channel: u32,
    handle: AudioSequenceHandle,
    serial: u64,
}
#[derive(Default)]
pub(super) struct Events {
    bindings: BTreeMap<Key, AudioSequenceHandle>,
    retired: Vec<AudioSequenceHandle>,
    awaiting: Option<Await>,
}
impl Events {
    pub(super) fn invalidate(&mut self) {
        self.retired
            .extend(std::mem::take(&mut self.bindings).into_values());
        self.awaiting = None;
    }
}
impl SiglusVm {
    pub(super) fn release_event(&mut self, host: &mut dyn HostServices, k: Key) -> Result<()> {
        if let Some(h) = self.events.bindings.remove(&k) {
            host.release_audio_sequence(ClientId(self.state.client), h)
                .map_err(err)?;
        }
        self.state.event_cursors.remove(&k);
        Ok(())
    }

    pub(super) fn validate_events(&self) -> Result<()> {
        Self::validate_event_state(&self.state)
    }
    pub(super) fn validate_event_state(state: &Machine) -> Result<()> {
        ensure!(
            state.event_cursors.len() <= crate::pcmevent_bridge::LIMIT,
            "saved native event cursor count"
        );
        if let Some(f) = &state.event_frame {
            ensure!(
                f.keys.len() <= crate::pcmevent_bridge::LIMIT
                    && f.index < f.keys.len()
                    && f.delta <= 60000,
                "invalid saved native event frame"
            );
            ensure!(
                f.keys.windows(2).all(|w| w[0] < w[1]),
                "invalid saved native event order"
            );
        }
        for (k, r) in &state.event_cursors {
            ensure!(
                k.1 < crate::pcmevent_bridge::LIMIT,
                "saved event cursor index"
            );
            ensure!(
                r.cursor.active
                    && r.declaration.cur_time >= 0
                    && r.declaration.cur_time as u64 == r.cursor.elapsed_ms
                    && r.declaration.next_time == r.cursor.next_ms
                    && r.declaration.cur_line_no == r.cursor.last_index.map_or(-1, |i| i as i32),
                "saved native event cursor/declaration mismatch"
            );
            r.cursor
                .validate_against(&crate::pcmevent_bridge::spec(&r.declaration)?)
                .map_err(err)?;
        }
        if let Some(vm) = &state.vm {
            let n: usize = vm.ctx.globals.pcm_event_lists.values().map(Vec::len).sum();
            ensure!(
                n <= crate::pcmevent_bridge::LIMIT,
                "saved native event list exceeds 128"
            );
            for k in state
                .event_cursors
                .keys()
                .chain(state.event_frame.iter().flat_map(|f| f.keys.iter()))
            {
                ensure!(
                    k.0 == vm.ctx.ids.form_global_pcm_event && k.1 < crate::pcmevent_bridge::LIMIT,
                    "saved native event cursor/frame namespace"
                );
            }
            for (form, list) in &vm.ctx.globals.pcm_event_lists {
                ensure!(
                    *form == vm.ctx.ids.form_global_pcm_event,
                    "unsupported saved native event namespace"
                );
                for st in list {
                    if st.is_active() {
                        crate::pcmevent_bridge::spec(st)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn declaration(&self, k: Key) -> Option<&PcmEventState> {
        self.state
            .vm
            .as_ref()?
            .ctx
            .globals
            .pcm_event_lists
            .get(&k.0)?
            .get(k.1)
    }
    fn remember_event(&mut self, host: &dyn HostServices, k: Key, selected: bool) -> Result<bool> {
        let client = ClientId(self.state.client);
        let h = self.events.bindings[&k];
        let cursor = host.capture_audio_sequence(client, h).map_err(err)?;
        let vm = self.state.vm.as_mut().unwrap();
        let st = &mut vm.ctx.globals.pcm_event_lists.get_mut(&k.0).unwrap()[k.1];
        if selected {
            vm.ctx.globals.rng_state = cursor.rng_state;
        }
        if cursor.active {
            if selected && st.event_type == crate::runtime::globals::PCM_EVENT_TYPE_RANDOM {
                st.last_line_no = st.cur_line_no;
            }
            st.cur_time = cursor.elapsed_ms as i64;
            st.next_time = cursor.next_ms;
            st.cur_line_no = cursor.last_index.map_or(-1, |i| i as i32);
            self.state.event_cursors.insert(
                k,
                Record {
                    declaration: st.clone(),
                    cursor,
                },
            );
            Ok(true)
        } else {
            // An empty/final ONESHOT event reinitializes the native declaration.
            st.reinit();
            self.state.event_cursors.remove(&k);
            Ok(false)
        }
    }
    pub(super) fn begin_event_frame(&mut self) -> Result<()> {
        self.validate_events()?;
        let delta = self
            .state
            .event_clock_ms
            .map_or(0, |old| self.state.now_ms.saturating_sub(old));
        self.state.event_clock_ms = Some(self.state.now_ms);
        let vm = self.state.vm.as_ref().unwrap();
        let mut keys = vm
            .ctx
            .globals
            .pcm_event_lists
            .iter()
            .flat_map(|(form, list)| {
                list.iter()
                    .enumerate()
                    .filter(|(_, s)| s.is_active())
                    .map(move |(i, _)| (*form, i))
            })
            .collect::<Vec<_>>();
        keys.sort_unstable();
        if !keys.is_empty() {
            self.state.event_frame = Some(Frame {
                keys,
                index: 0,
                advanced: false,
                delta,
            });
        }
        Ok(())
    }
    pub(super) fn drive_events(&mut self, host: &mut dyn HostServices) -> Result<bool> {
        let client = ClientId(self.state.client);
        // Direct adapter restore can leave already-canceled derived handles;
        // Runtime restore clears the entire bank. Cancel those old generations
        // best-effort, never interpreting them as portable scheduling state.
        for h in std::mem::take(&mut self.events.retired) {
            let _ = host.release_audio_sequence(client, h);
        }
        let obsolete = self
            .events
            .bindings
            .keys()
            .copied()
            .filter(|k| {
                let st = self.declaration(*k);
                !st.is_some_and(|st| {
                    st.is_active()
                        && self
                            .state
                            .event_cursors
                            .get(k)
                            .is_some_and(|r| &r.declaration == st)
                })
            })
            .collect::<Vec<_>>();
        for k in obsolete {
            let h = self.events.bindings.remove(&k).unwrap();
            host.release_audio_sequence(client, h).map_err(err)?;
            self.state.event_cursors.remove(&k);
        }
        let stale = self
            .state
            .event_cursors
            .iter()
            .filter_map(|(k, r)| {
                (!self
                    .declaration(*k)
                    .is_some_and(|st| st.is_active() && *st == r.declaration))
                .then_some(*k)
            })
            .collect::<Vec<_>>();
        for k in stale {
            self.state.event_cursors.remove(&k);
        }
        loop {
            if let Some(a) = self.events.awaiting.take() {
                let info = host
                    .audio_info(client, a.channel)
                    .map_err(err)?
                    .context("PCMEVENT published voice has no source metadata")?;
                ensure!(info.source_rate > 0, "PCMEVENT invalid source sample rate");
                let duration = (u128::from(
                    info.source_frames
                        .context("PCMEVENT needs measured source length")?,
                ) * 1000
                    / u128::from(info.source_rate))
                .min(u128::from(u64::MAX)) as u64;
                host.complete_audio_sequence(client, a.handle, a.serial, duration)
                    .map_err(err)?;
                self.remember_event(host, a.key, true)?;
            }
            let Some(f) = &self.state.event_frame else {
                // Dependency time was frozen; do not retroactively charge it
                // through the next VM frame's wall-clock delta.
                self.state.event_clock_ms = Some(self.state.now_ms);
                return Ok(true);
            };
            let k = f.keys[f.index];
            if !self.declaration(k).is_some_and(PcmEventState::is_active) {
                if let Some(h) = self.events.bindings.remove(&k) {
                    host.release_audio_sequence(client, h).map_err(err)?;
                }
                self.state.event_cursors.remove(&k);
                self.next_event();
                continue;
            }
            if !self.events.bindings.contains_key(&k) {
                let st = self.declaration(k).unwrap();
                let spec = crate::pcmevent_bridge::spec(st)?;
                let h = if let Some(r) = self
                    .state
                    .event_cursors
                    .get(&k)
                    .filter(|r| &r.declaration == st)
                {
                    host.restore_audio_sequence(client, spec, r.cursor.clone())
                        .map_err(err)?
                } else {
                    host.create_audio_sequence(
                        client,
                        spec,
                        self.state.vm.as_ref().unwrap().ctx.globals.rng_state,
                    )
                    .map_err(err)?
                };
                self.events.bindings.insert(k, h);
            }
            let h = self.events.bindings[&k];
            if !self.state.event_frame.as_ref().unwrap().advanced {
                let dt = self.state.event_frame.as_ref().unwrap().delta;
                host.advance_audio_sequence(client, h, dt, dt)
                    .map_err(err)?;
                self.state.event_frame.as_mut().unwrap().advanced = true;
            }
            // Capture BEFORE selection so snapshot rewind preserves RNG and
            // the already-advanced owner frame across uncommitted preparation.
            if !self.remember_event(host, k, false)? {
                continue;
            }
            let seed = self.state.vm.as_ref().unwrap().ctx.globals.rng_state;
            let Some(e) = host
                .poll_audio_sequence(client, h, Some(seed))
                .map_err(err)?
            else {
                self.remember_event(host, k, false)?;
                self.next_event();
                continue;
            };
            let before = self.state.vm.as_ref().unwrap().clone();
            let st = self.declaration(k).unwrap().clone();
            let channel = crate::pcmevent_bridge::play(
                &mut self.state.vm.as_mut().unwrap().ctx,
                &st,
                &e.key,
            )?;
            self.events.awaiting = Some(Await {
                key: k,
                channel,
                handle: h,
                serial: e.serial,
            });
            if !self.begin_audio_batch(host, before)? {
                return Ok(false);
            }
        }
    }
    fn next_event(&mut self) {
        let f = self.state.event_frame.as_mut().unwrap();
        f.index += 1;
        f.advanced = false;
        if f.index == f.keys.len() {
            self.state.event_frame = None;
        }
    }
    pub(super) fn event_diagnostics(&self) -> serde_json::Value {
        self.state.vm.as_ref().map(|vm|serde_json::Value::Array(vm.ctx.globals.pcm_event_lists.iter().flat_map(|(form,list)|list.iter().enumerate().filter(|(_,st)|st.is_active()).map(move |(i,st)|serde_json::json!({"form":form,"index":i,"type":st.event_type,"target":st.pcm_buf_no,"current_line":st.cur_line_no,"elapsed_ms":st.cur_time,"next_ms":st.next_time,"lines":st.lines.len(),"shared":self.events.bindings.get(&(*form,i)).map(|h|serde_json::json!({"client":h.client,"generation":h.generation}))}))).collect())).unwrap_or(serde_json::json!([]))
    }
}
