// SPDX-License-Identifier: MPL-2.0
//! Native channel requests mapped onto the host's existing playback service.
use crate::{engine::state, files::ScopedFiles};
use crysirror_core::{
    Error, Result,
    service::{AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, HostServices, PcmBus},
    state::ClientId,
};
use crysirror_python::{PyResult, Value, Vm, builtins::Kw, methods, ops};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Command {
    Play {
        channel: u32,
        name: String,
        path: String,
        parameters: AudioParameters,
    },
    Stop {
        channel: u32,
        fade_ms: f32,
    },
}

struct Playback {
    name: String,
    ticket: Option<u64>,
    path: String,
    looping: bool,
}

#[derive(Default)]
pub struct Transport {
    active: BTreeMap<u32, Playback>,
}

impl Transport {
    pub fn rewind(&self, loops: BTreeMap<u32, Command>) -> Vec<Command> {
        let mut commands = Vec::new();
        for (&channel, playback) in &self.active {
            if !playback.looping || !loops.contains_key(&channel) {
                commands.push(Command::Stop {
                    channel,
                    fade_ms: 0.0,
                });
            }
        }
        for (channel, command) in loops {
            if let Command::Play { path, .. } = &command {
                if self
                    .active
                    .get(&channel)
                    .is_some_and(|a| a.looping && &a.path == path)
                {
                    continue;
                }
            }
            commands.push(command);
        }
        commands
    }

    pub fn sync(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &ScopedFiles,
        commands: Vec<Command>,
    ) -> Result<BTreeMap<u32, String>> {
        for command in commands {
            match command {
                Command::Play {
                    channel,
                    name,
                    path,
                    parameters,
                } => {
                    let asset = files
                        .asset_path(&path)
                        .ok_or_else(|| Error::Io(format!("Ren'Py audio asset missing: {path}")))?;
                    let looping = parameters.looping;
                    let ticket = host.begin_audio(
                        client,
                        AudioRequest::PlayAsset {
                            channel,
                            source: AudioAsset {
                                path: asset.clone(),
                                frame_limit: None,
                            },
                            intro: None,
                            parameters,
                        },
                    )?;
                    if let Some(previous) = self.active.insert(
                        channel,
                        Playback {
                            name,
                            ticket: Some(ticket),
                            path,
                            looping,
                        },
                    ) {
                        if let Some(previous) = previous.ticket {
                            host.release_audio(client, previous)?;
                        }
                    }
                }
                Command::Stop { channel, fade_ms } => {
                    if let Some(playback) = self.active.remove(&channel) {
                        if let Some(ticket) = playback.ticket {
                            host.release_audio(client, ticket)?;
                        }
                    }
                    host.submit_audio(client, AudioRequest::Stop { channel, fade_ms })?;
                }
            }
        }
        let mut current = BTreeMap::new();
        let mut completed = Vec::new();
        for (&channel, playback) in &mut self.active {
            if let Some(ticket) = playback.ticket {
                let preparation = host.audio_preparation(client, ticket)?;
                match preparation.phase {
                    AudioPreparePhase::Pending => {
                        current.insert(channel, playback.name.clone());
                        continue;
                    }
                    AudioPreparePhase::Ready => {
                        host.release_audio(client, ticket)?;
                        playback.ticket = None;
                    }
                    AudioPreparePhase::Failed => {
                        host.release_audio(client, ticket)?;
                        playback.ticket = None;
                        return Err(Error::Io(preparation.error.unwrap_or_else(|| {
                            format!("Ren'Py audio failed: {}", playback.name)
                        })));
                    }
                }
            }
            if host.audio_playing(client, channel)? {
                current.insert(channel, playback.name.clone());
            } else {
                completed.push(channel);
            }
        }
        for channel in completed {
            self.active.remove(&channel);
        }
        Ok(current)
    }
}

pub fn in_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(
        state(vm)
            .contexts
            .borrow()
            .last()
            .is_some_and(|c| c.init_phase),
    ))
}

pub fn current(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [channel] = args else {
        return Err(vm.type_error("audio_current expects a channel"));
    };
    let channel = channel_id(vm, channel)?;
    let st = state(vm);
    let names = st.audio_playing.borrow();
    Ok(names
        .get(&channel)
        .map_or(Value::None, |name| vm.types.new_str(name)))
}

pub fn play(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [
        channel,
        name,
        path,
        mixer,
        looping,
        fadeout,
        fadein,
        if_changed,
    ] = args
    else {
        return Err(vm.type_error("invalid audio_play arguments"));
    };
    let channel = channel_id(vm, channel)?;
    let name = ops::str(vm, name)?;
    let path = ops::str(vm, path)?;
    if path.starts_with('<') {
        return Err(vm.exc_msg("NotImplementedError", "Ren'Py audio clip ranges"));
    }
    let st = state(vm);
    if ops::truthy(vm, if_changed)? && st.audio_playing.borrow().contains_key(&channel) {
        return Err(vm.exc_msg("NotImplementedError", "Ren'Py if_changed with active audio"));
    }
    let mixer = mixer.as_str().unwrap_or("sfx");
    let bus = match mixer {
        "music" => PcmBus::Music,
        "voice" => PcmBus::Voice,
        _ => PcmBus::Sfx,
    };
    let parameters = AudioParameters {
        bus,
        looping: ops::truthy(vm, looping)?,
        replace_fade_ms: milliseconds(vm, fadeout)?,
        fade_ms: milliseconds(vm, fadein)?,
        ..Default::default()
    };
    push(
        vm,
        &st,
        Command::Play {
            channel,
            name,
            path,
            parameters,
        },
    )
}

pub fn stop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [channel, fade] = args else {
        return Err(vm.type_error("invalid audio_stop arguments"));
    };
    let channel = channel_id(vm, channel)?;
    let fade_ms = milliseconds(vm, fade)?;
    let st = state(vm);
    push(vm, &st, Command::Stop { channel, fade_ms })
}

fn push(vm: &Vm, state: &crate::engine::State, command: Command) -> PyResult<Value> {
    let mut pending = state.audio_commands.borrow_mut();
    if pending.len() >= 512 {
        return Err(vm.value_error("too many pending audio commands"));
    }
    match &command {
        Command::Play {
            channel,
            parameters,
            ..
        } if parameters.looping => {
            state
                .audio_loops
                .borrow_mut()
                .insert(*channel, command.clone());
        }
        Command::Play { channel, .. } | Command::Stop { channel, .. } => {
            state.audio_loops.borrow_mut().remove(channel);
        }
    }
    pending.push(command);
    Ok(Value::None)
}

fn channel_id(vm: &mut Vm, value: &Value) -> PyResult<u32> {
    u32::try_from(ops::index_of(vm, value)?).map_err(|_| vm.value_error("invalid audio channel"))
}

fn milliseconds(vm: &mut Vm, value: &Value) -> PyResult<f32> {
    let seconds = methods::to_float(vm, value)?;
    if !seconds.is_finite() || !(0.0..=600.0).contains(&seconds) {
        return Err(vm.value_error("invalid audio fade duration"));
    }
    Ok((seconds * 1000.) as f32)
}
