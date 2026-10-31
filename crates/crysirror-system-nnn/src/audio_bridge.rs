// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Scoped native audio dependency routing. Native lookup policy runs on an
//! admitted worker; engine workers own decoding/mixing and the owner publishes.
use crate::{
    config::Names,
    machine::{Machine, NativeCall},
    native_audio::{AudioOperation, NativeAudioPlan, NativeAudioPolicy, NativeSource},
};
use anyhow::{bail, ensure, Result};
use crysirror_core::{
    service::{AudioPreparePhase, HostServices},
    state::ClientId,
};
use crysirror_tasks::{Cancellation, Task, TaskPool};
use std::sync::Arc;

/// Resolves reviewed native precedence using scoped engine archive/VFS APIs.
/// Never return a playlist/container as if it were decoded audio. Missing and
/// corrupt sources remain distinct; cancellation belongs to the current task.
pub type AudioResolver = Arc<
    dyn Fn(&NativeSource, &Cancellation) -> Result<crate::resource_bridge::ResolvedAudio>
        + Send
        + Sync,
>;
enum Pending {
    Resolve {
        call: NativeCall,
        plan: NativeAudioPlan,
        task: Task<crate::resource_bridge::ResolvedAudio>,
    },
    Prepare {
        call: NativeCall,
        plan: NativeAudioPlan,
        ticket: u64,
    },
}
pub struct AudioBridge {
    pub policy: NativeAudioPolicy,
    pub api_calls: u64,
    client: ClientId,
    music: Names,
    sound: Names,
    resolver: AudioResolver,
    pool: TaskPool,
    pending: Option<Pending>,
}
impl AudioBridge {
    pub fn client(&self) -> ClientId {
        self.client
    }
    pub fn new(
        client: ClientId,
        policy: NativeAudioPolicy,
        music: Names,
        sound: Names,
        resolver: AudioResolver,
    ) -> Result<Self> {
        ensure!(client.0 != 0, "NNN audio client identity");
        Ok(Self {
            policy,
            client,
            music,
            sound,
            resolver,
            pool: TaskPool::new("nnn-native-audio-lookup", 1, 1)?,
            pending: None,
            api_calls: 0,
        })
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn try_commit(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<bool> {
        machine.validate_receipt(call)?;
        if self.pending.is_none() {
            let playing = host.audio_playing(self.client, self.policy.bindings.music)?;
            let plan = self.policy.plan(call, &self.music, &self.sound, playing)?;
            if let Some(source) = plan.source.clone() {
                let resolver = self.resolver.clone();
                let task = self.pool.spawn(move |cancel| {
                    cancel.check()?;
                    let path = resolver(&source, &cancel)?;
                    cancel.check()?;
                    Ok(path)
                })?;
                self.pending = Some(Pending::Resolve {
                    call: call.clone(),
                    plan,
                    task,
                });
            } else {
                if let Some(request) = plan.control_request()? {
                    host.submit_audio(self.client, request)?;
                    self.api_calls += 1;
                }
                machine.acknowledge(call)?;
                self.policy = plan.next;
                return Ok(true);
            }
        }
        let pending = self.pending.take().unwrap();
        match pending {
            Pending::Resolve {
                call: receipt,
                plan,
                mut task,
            } => {
                ensure!(receipt == *call, "NNN audio lookup receipt changed");
                if let Some(path) = task.try_take()? {
                    let request = match path {
                        crate::resource_bridge::ResolvedAudio::Asset(path) => {
                            plan.asset_request(path)?
                        }
                        crate::resource_bridge::ResolvedAudio::Encoded(source) => {
                            plan.encoded_request(source)?
                        }
                    };
                    let ticket = host.begin_audio(self.client, request)?;
                    self.api_calls += 1;
                    self.pending = Some(Pending::Prepare {
                        call: receipt,
                        plan,
                        ticket,
                    });
                } else {
                    self.pending = Some(Pending::Resolve {
                        call: receipt,
                        plan,
                        task,
                    });
                }
                Ok(false)
            }
            Pending::Prepare {
                call: receipt,
                plan,
                ticket,
            } => {
                ensure!(receipt == *call, "NNN audio preparation receipt changed");
                let state = host.audio_preparation(self.client, ticket)?;
                let AudioOperation::Play { channel, .. } = plan.operation else {
                    bail!("NNN preparation without playback")
                };
                ensure!(
                    state.channel == channel,
                    "NNN shared audio channel receipt mismatch"
                );
                match state.phase {
                    AudioPreparePhase::Pending => {
                        self.pending = Some(Pending::Prepare {
                            call: receipt,
                            plan,
                            ticket,
                        });
                        Ok(false)
                    }
                    AudioPreparePhase::Failed => bail!(
                        "NNN shared audio preparation failed: {}",
                        state.error.as_deref().unwrap_or("no diagnostic")
                    ),
                    AudioPreparePhase::Ready => {
                        host.release_audio(self.client, ticket)?;
                        machine.acknowledge(call)?;
                        self.policy = plan.next;
                        Ok(true)
                    }
                }
            }
        }
    }
}
