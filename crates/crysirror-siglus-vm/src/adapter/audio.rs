// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native command commit gate. Worker tickets never enter VM/save bytes.
use super::*;
use crate::bgm_bridge::{CHANNEL, Operation, Play};
use crysirror_core::service::{
    AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, PcmBus,
};
use std::collections::VecDeque;

pub(super) struct Pending {
    after: SceneVm<'static>,
    operations: VecDeque<Operation>,
    ticket: Option<(u64, Play)>,
}
fn number(value: Option<&str>, default: i64) -> Result<i64> {
    let Some(v) = value else { return Ok(default) };
    let v = v.trim();
    if let Some(h) = v.strip_prefix("-0x") {
        Ok(-i64::from_str_radix(h, 16)?)
    } else if let Some(h) = v.strip_prefix("0x") {
        Ok(i64::from_str_radix(h, 16)?)
    } else {
        Ok(v.parse()?)
    }
}
impl SiglusVm {
    pub(super) fn sync_audio(&mut self, host: &dyn HostServices) -> Result<()> {
        if let Some(vm) = self.state.vm.as_mut() {
            if vm.ctx.bgm.current.is_some() || vm.ctx.bgm.check_state() != 0 {
                let info = host
                    .audio_info(ClientId(self.state.client), CHANNEL)
                    .map_err(err)?;
                vm.ctx.bgm.sync(info);
            }
        }
        Ok(())
    }
    /// Return false while admitted resource work owns the original command.
    pub(super) fn begin_audio_batch(
        &mut self,
        host: &mut dyn HostServices,
        before: SceneVm<'static>,
    ) -> Result<bool> {
        let operations = std::mem::take(&mut self.state.vm.as_mut().unwrap().ctx.bgm.pending);
        if operations.is_empty() {
            return Ok(true);
        }
        let after = self.state.vm.replace(before).unwrap();
        self.pending_audio = Some(Pending {
            after,
            operations: operations.into(),
            ticket: None,
        });
        self.poll_audio(host)
    }
    pub(super) fn poll_audio(&mut self, host: &mut dyn HostServices) -> Result<bool> {
        let Some(mut pending) = self.pending_audio.take() else {
            return Ok(true);
        };
        let client = ClientId(self.state.client);
        loop {
            if let Some((ticket, play)) = pending.ticket.as_ref() {
                let status = match host.audio_preparation(client, *ticket) {
                    Ok(status) => status,
                    Err(error) => {
                        self.pending_audio = Some(pending);
                        return Err(anyhow::anyhow!("{error}"));
                    }
                };
                match status.phase {
                    AudioPreparePhase::Pending => {
                        self.pending_audio = Some(pending);
                        return Ok(false);
                    }
                    AudioPreparePhase::Failed => {
                        let failure = status
                            .error
                            .unwrap_or_else(|| "Siglus BGM preparation failed".into());
                        host.release_audio(client, *ticket).map_err(err)?;
                        anyhow::bail!("{failure}");
                    }
                    AudioPreparePhase::Ready => {
                        host.release_audio(client, *ticket).map_err(err)?;
                        if play.delay > 0 {
                            host.submit_audio(
                                client,
                                AudioRequest::Resume {
                                    channel: CHANNEL,
                                    delay_ms: play.delay as f32,
                                    fade_ms: play.fade_in.max(0) as f32,
                                },
                            )
                            .map_err(err)?;
                        }
                        let info = host.audio_info(client, CHANNEL).map_err(err)?;
                        if let Some(info) = info {
                            pending
                                .after
                                .ctx
                                .bgm
                                .set_initial_position(info.position_frames);
                        }
                        pending.after.ctx.bgm.sync(info);
                        pending.ticket = None;
                    }
                }
            }
            let Some(operation) = pending.operations.pop_front() else {
                self.state.vm = Some(pending.after);
                return Ok(true);
            };
            match operation {
                Operation::Control(request) => {
                    host.submit_audio(client, request).map_err(err)?;
                    pending
                        .after
                        .ctx
                        .bgm
                        .sync(host.audio_info(client, CHANNEL).map_err(err)?);
                }
                Operation::Play(play) => {
                    let request = self.bgm_request(&mut pending.after, &play)?;
                    let ticket = host.begin_audio(client, request).map_err(err)?;
                    pending.ticket = Some((ticket, play));
                }
            }
        }
    }
    fn bgm_request(&self, vm: &mut SceneVm<'static>, play: &Play) -> Result<AudioRequest> {
        let mut file = play.name.clone();
        let mut start = 0;
        let mut end = -1;
        let mut repeat = 0;
        if let Some(cfg) = vm.ctx.tables.gameexe.as_ref() {
            ensure!(
                cfg.indexed_count("BGM") <= 4096 && cfg.get_usize("BGM.CNT").unwrap_or(0) <= 4096,
                "Siglus BGM table exceeds 4096 entries"
            );
            for index in 0..cfg.indexed_count("BGM") {
                if !cfg
                    .get_indexed_item_unquoted("BGM", index, 0)
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case(&play.name))
                {
                    continue;
                }
                file = cfg
                    .get_indexed_item_unquoted("BGM", index, 1)
                    .context("Siglus BGM file name missing")?
                    .trim()
                    .into();
                ensure!(!file.is_empty(), "empty Siglus BGM file name");
                start = number(cfg.get_indexed_item_unquoted("BGM", index, 2), 0)?;
                end = number(cfg.get_indexed_item_unquoted("BGM", index, 3), -1)?;
                repeat = number(cfg.get_indexed_item_unquoted("BGM", index, 4), 0)?;
                let count = cfg
                    .get_usize("BGM.CNT")
                    .unwrap_or_else(|| cfg.indexed_count("BGM"));
                if index < count {
                    vm.ctx.globals.bgm_table_flags.resize(
                        count.max(vm.ctx.globals.bgm_table_flags.len()),
                        vm.ctx.globals.bgm_table_all_flag,
                    );
                    vm.ctx.globals.bgm_table_flags[index] = true;
                    vm.ctx
                        .globals
                        .bgm_table_listened
                        .insert(play.name.clone(), true);
                }
                break;
            }
        }
        let asset =
            self.state.config.resources.get(&file).with_context(|| {
                format!("Siglus BGM requires config.resources alias for {file:?}")
            })?;
        Ok(AudioRequest::PlayAsset {
            channel: CHANNEL,
            source: AudioAsset {
                path: VfsPath::new(Mount::Asset, asset).map_err(err)?,
                frame_limit: (end >= 0).then_some(end.max(0) as u64),
            },
            intro: None,
            parameters: AudioParameters {
                bus: PcmBus::Music,
                gain: f32::from(vm.ctx.bgm.volume_raw()) / 255.,
                start_frame: Some(if play.start == -1 { start } else { play.start }.max(0) as u64),
                clamp_range: true,
                loop_start: repeat.max(0) as u64,
                looping: play.looping,
                paused: play.paused || play.delay > 0,
                fade_ms: if play.paused || play.delay > 0 {
                    0.
                } else {
                    play.fade_in.max(0) as f32
                },
                replace_fade_ms: play.fade_out.max(0) as f32,
                ..Default::default()
            },
        })
    }
}
