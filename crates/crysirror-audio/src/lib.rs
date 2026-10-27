// SPDX-License-Identifier: MPL-2.0
//! Engine-owned audio scheduling and buses. The platform device is replaceable.
//! Canonical asset/voice identity remains in SceneState; SDK objects live here.
#![forbid(unsafe_code)]
use anyhow::{Result, ensure};
use crysirror_core::state::{AudioVoice, ObjectId, SceneState};
use std::{collections::BTreeMap, sync::Arc};

pub mod buffered;
pub mod channels;
mod channel_async;
pub mod decoder;
pub mod doppler;
pub mod motion;
pub mod legacy_pcm;
pub mod mixer;
mod envelope;
mod mix_groups;
pub mod native;
pub mod pcm;
pub mod reverb;
pub mod resources;
pub mod voice_compression;
pub mod render_job;
pub mod scene;

pub mod pcm_output;
#[cfg(feature = "cpal-output")]
mod cpal_output;
#[cfg(feature = "rodio-output")]
pub mod rodio_device;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum AudioBus {
    #[default]
    Sfx,
    Music,
    Voice,
    Ui,
    Ambient,
}

pub struct VoiceStart {
    pub voice: AudioVoice,
    pub bytes: Arc<[u8]>,
    pub gain_milli: u16,
    pub paused: bool,
}
pub trait AudioDevice {
    fn start(&mut self, id: ObjectId, request: VoiceStart) -> Result<()>;
    fn stop(&mut self, id: ObjectId);
    fn set_gain(&mut self, id: ObjectId, gain_milli: u16);
    fn pause(&mut self, id: ObjectId);
    fn resume(&mut self, id: ObjectId);
}
struct Playing {
    epoch: u64,
    gain: u16,
    bus: AudioBus,
}

/// Bus levels/routing are host playback policy in this version, not serialized
/// VM state. No native pan/fade/streaming behavior is inferred from these buses.
pub struct AudioSystem<D: AudioDevice> {
    device: D,
    playing: BTreeMap<ObjectId, Playing>,
    restored: u64,
    paused: bool,
    master: u16,
    buses: BTreeMap<AudioBus, u16>,
    routes: BTreeMap<ObjectId, AudioBus>,
}
impl<D: AudioDevice> AudioSystem<D> {
    pub fn new(device: D) -> Self {
        Self {
            device,
            playing: BTreeMap::new(),
            restored: 0,
            paused: false,
            master: 1000,
            buses: BTreeMap::new(),
            routes: BTreeMap::new(),
        }
    }
    pub fn device(&self) -> &D {
        &self.device
    }
    pub fn voice_bus(&self, id: ObjectId) -> Option<AudioBus> {
        self.playing.get(&id).map(|v| v.bus)
    }
    pub fn set_master_gain(&mut self, gain: u16) -> Result<()> {
        ensure!(gain <= 1000, "master gain exceeds 1000");
        self.master = gain;
        Ok(())
    }
    pub fn set_bus_gain(&mut self, bus: AudioBus, gain: u16) -> Result<()> {
        ensure!(gain <= 1000, "bus gain exceeds 1000");
        self.buses.insert(bus, gain);
        Ok(())
    }
    pub fn set_route(&mut self, id: ObjectId, bus: AudioBus) -> Result<()> {
        ensure!(
            self.playing.contains_key(&id),
            "route requires an active client-scoped voice"
        );
        self.routes.insert(id, bus);
        Ok(())
    }
    fn gain(&self, voice: &AudioVoice, bus: AudioBus) -> u16 {
        ((u64::from(voice.gain_milli)
            * u64::from(self.master)
            * u64::from(*self.buses.get(&bus).unwrap_or(&1000)))
            / 1_000_000) as u16
    }
    /// Explicit synchronous compatibility path: load/device callbacks run on
    /// this caller. Interactive hosts use SceneAudio with worker preparation.
    /// Device operations follow committed logical state. Device/asset failures
    /// are reported; this does not claim to roll back already-issued audio.
    pub fn sync(
        &mut self,
        scene: &SceneState,
        restored: u64,
        mut load: impl FnMut(ObjectId, &AudioVoice) -> Result<Arc<[u8]>>,
    ) -> Result<()> {
        if restored != self.restored {
            for id in self.playing.keys() {
                self.device.stop(*id);
            }
            self.playing.clear();
            self.restored = restored;
        }
        let stale: Vec<_> = self
            .playing
            .iter()
            .filter(|(id, p)| {
                !scene.voices().contains_key(id) || scene.voice_epoch(**id) != p.epoch
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.device.stop(id);
            self.playing.remove(&id);
        }
        self.routes.retain(|id, _| scene.voices().contains_key(id));
        for (id, voice) in scene.voices() {
            let bus = self.routes.get(id).copied().unwrap_or_default();
            let gain = self.gain(voice, bus);
            if let Some(old) = self.playing.get_mut(id) {
                if old.gain != gain {
                    self.device.set_gain(*id, gain);
                    old.gain = gain;
                }
                old.bus = bus;
                continue;
            }
            let bytes = load(*id, voice)?;
            self.device.start(
                *id,
                VoiceStart {
                    voice: voice.clone(),
                    bytes,
                    gain_milli: gain,
                    paused: self.paused,
                },
            )?;
            self.playing.insert(
                *id,
                Playing {
                    epoch: scene.voice_epoch(*id),
                    gain,
                    bus,
                },
            );
        }
        Ok(())
    }
    pub fn pause(&mut self) {
        self.paused = true;
        for id in self.playing.keys() {
            self.device.pause(*id);
        }
    }
    pub fn resume(&mut self) {
        self.paused = false;
        for id in self.playing.keys() {
            self.device.resume(*id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{
        path::{Mount, VfsPath},
        state::{ClientId, Command, Limits},
    };
    #[derive(Default)]
    struct Device {
        events: Vec<String>,
        fail: bool,
    }
    impl AudioDevice for Device {
        fn start(&mut self, id: ObjectId, r: VoiceStart) -> Result<()> {
            ensure!(!self.fail, "device failure");
            self.events.push(format!(
                "start:{}:{}:{}:{}:{}",
                id.client.0, id.local, r.gain_milli, r.voice.looping, r.paused
            ));
            Ok(())
        }
        fn stop(&mut self, id: ObjectId) {
            self.events
                .push(format!("stop:{}:{}", id.client.0, id.local));
        }
        fn set_gain(&mut self, id: ObjectId, g: u16) {
            self.events
                .push(format!("gain:{}:{}:{g}", id.client.0, id.local));
        }
        fn pause(&mut self, id: ObjectId) {
            self.events
                .push(format!("pause:{}:{}", id.client.0, id.local));
        }
        fn resume(&mut self, id: ObjectId) {
            self.events
                .push(format!("resume:{}:{}", id.client.0, id.local));
        }
    }
    fn play() -> Command {
        Command::PlayAudio {
            channel: 0,
            voice: AudioVoice {
                asset: VfsPath::new(Mount::Asset, "tone.wav").unwrap(),
                looping: true,
                gain_milli: 800,
            },
        }
    }
    fn id(client: u32) -> ObjectId {
        ObjectId {
            client: ClientId(client),
            local: 0,
        }
    }
    fn load(_: ObjectId, _: &AudioVoice) -> Result<Arc<[u8]>> {
        Ok(Arc::from([1u8, 2].as_slice()))
    }
    #[test]
    fn retrigger_restore_and_client_ids_preserve_voice_semantics() {
        let mut scene = SceneState::default();
        let mut system = AudioSystem::new(Device::default());
        for client in [1, 2] {
            scene
                .apply_batch(ClientId(client), &[play()], Limits::default())
                .unwrap();
        }
        system.sync(&scene, 0, load).unwrap();
        system.sync(&scene, 0, load).unwrap();
        assert_eq!(system.device().events.len(), 2);
        scene
            .apply_batch(ClientId(1), &[play()], Limits::default())
            .unwrap();
        system.sync(&scene, 0, load).unwrap();
        assert_eq!(
            &system.device().events[2..],
            ["stop:1:0", "start:1:0:800:true:false"]
        );
        system.sync(&scene, 1, load).unwrap();
        assert_eq!(system.device().events.len(), 8);
        scene
            .apply_batch(
                ClientId(1),
                &[Command::StopAudio { channel: 0 }],
                Limits::default(),
            )
            .unwrap();
        system.sync(&scene, 1, load).unwrap();
        assert_eq!(system.device().events.last().unwrap(), "stop:1:0");
    }
    #[test]
    fn bus_changes_gain_without_restarting_or_affecting_other_clients() {
        let mut scene = SceneState::default();
        for c in [1, 2] {
            scene
                .apply_batch(ClientId(c), &[play()], Limits::default())
                .unwrap();
        }
        let mut system = AudioSystem::new(Device::default());
        system.sync(&scene, 0, load).unwrap();
        system.set_route(id(1), AudioBus::Music).unwrap();
        system.set_bus_gain(AudioBus::Music, 500).unwrap();
        system.sync(&scene, 0, load).unwrap();
        assert_eq!(system.device().events.last().unwrap(), "gain:1:0:400");
        assert_eq!(system.voice_bus(id(1)), Some(AudioBus::Music));
        assert_eq!(system.voice_bus(id(2)), Some(AudioBus::Sfx));
        assert_eq!(system.device().events.len(), 3);
        system.set_master_gain(750).unwrap();
        system.sync(&scene, 0, load).unwrap();
        assert_eq!(
            &system.device().events[3..],
            ["gain:1:0:300", "gain:2:0:600"]
        );
        assert!(system.set_route(id(3), AudioBus::Sfx).is_err());
        assert!(system.set_master_gain(1001).is_err());
        assert!(system.set_bus_gain(AudioBus::Sfx, 1001).is_err());
    }
    #[test]
    fn paused_start_and_device_errors_are_explicit() {
        let mut scene = SceneState::default();
        scene
            .apply_batch(ClientId(1), &[play()], Limits::default())
            .unwrap();
        let mut system = AudioSystem::new(Device::default());
        system.pause();
        system.sync(&scene, 0, load).unwrap();
        assert_eq!(system.device().events[0], "start:1:0:800:true:true");
        system.resume();
        assert_eq!(system.device().events.last().unwrap(), "resume:1:0");
        let mut failed = AudioSystem::new(Device {
            fail: true,
            ..Device::default()
        });
        assert!(failed.sync(&scene, 0, load).is_err());
        assert!(failed.playing.is_empty());
    }
}
