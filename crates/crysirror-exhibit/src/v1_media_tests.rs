// SPDX-License-Identifier: MPL-2.0
use super::*;
use crysirror_core::{Result as CoreResult, service::*, state::Command};

#[derive(Default)]
struct Host {
    now: u64,
    ended: bool,
    playing: bool,
    ready: bool,
    files: Vec<String>,
    released: Vec<u64>,
    receipts: Vec<u64>,
    audio: Vec<AudioRequest>,
    movies: Vec<String>,
    layers: Vec<String>,
}
impl FileSystem for Host {
    fn read(&mut self, _: ClientId, _: &VfsPath) -> CoreResult<Vec<u8>> {
        Err(Error::NotFound)
    }
    fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
        Err(Error::ReadOnly)
    }
}
impl HostServices for Host {
    fn submit(&mut self, _: ClientId, _: &[Command]) -> CoreResult<()> {
        Ok(())
    }
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn submit_engine(&mut self, _: ClientId, _: u32, bytes: &[u8]) -> CoreResult<()> {
        let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        self.layers
            .push(value["layers"][0]["op"].as_str().unwrap().into());
        Ok(())
    }
    fn media_request(&mut self, _: ClientId, request: MediaRequest) -> CoreResult<VfsPath> {
        match request {
            MediaRequest::Play {
                id: 1,
                asset,
                looping: false,
                ..
            } => self.movies.push(asset),
            MediaRequest::Remove { id: 1 } => self.movies.push("removed".into()),
            _ => panic!("unexpected media request"),
        }
        VfsPath::new(Mount::Asset, "frame")
    }
    fn media_info(&self, _: ClientId, _: u32) -> CoreResult<MediaInfo> {
        Ok(MediaInfo {
            phase: if self.ended {
                MediaPhase::Ended
            } else {
                MediaPhase::Playing
            },
            position_ms: self.now,
            published_frames: 1,
            paused: false,
            frame_ms: 40,
            size: None,
            duration_ms: None,
        })
    }
    fn audio_playing(&self, _: ClientId, _: u32) -> CoreResult<bool> {
        Ok(self.playing)
    }
    fn begin_audio_resource(
        &mut self,
        _: ClientId,
        asset: AudioAsset,
        measure: bool,
    ) -> CoreResult<u64> {
        assert!(!measure);
        self.files.push(asset.path.relative().into());
        Ok(self.files.len() as u64)
    }
    fn audio_resource_preparation(
        &mut self,
        c: ClientId,
        ticket: u64,
    ) -> CoreResult<AudioResourcePreparation> {
        Ok(AudioResourcePreparation {
            phase: if self.ready {
                AudioPreparePhase::Ready
            } else {
                AudioPreparePhase::Pending
            },
            resource: self.ready.then_some(AudioResourceHandle {
                client: c.0,
                id: ticket,
            }),
            info: None,
            error: None,
        })
    }
    fn release_audio_resource(
        &mut self,
        _: ClientId,
        resource: AudioResourceHandle,
    ) -> CoreResult<()> {
        self.released.push(resource.id);
        Ok(())
    }
    fn release_audio_resource_preparation(&mut self, _: ClientId, ticket: u64) -> CoreResult<()> {
        self.receipts.push(ticket);
        Ok(())
    }
    fn submit_audio(&mut self, _: ClientId, request: AudioRequest) -> CoreResult<()> {
        self.audio.push(request);
        Ok(())
    }
    fn begin_audio(&mut self, c: ClientId, request: AudioRequest) -> CoreResult<u64> {
        self.submit_audio(c, request)?;
        Ok(999)
    }
    fn audio_preparation(&mut self, _: ClientId, _: u64) -> CoreResult<AudioPreparation> {
        Ok(AudioPreparation {
            channel: 15,
            phase: AudioPreparePhase::Ready,
            error: None,
        })
    }
    fn release_audio(&mut self, _: ClientId, _: u64) -> CoreResult<()> {
        Ok(())
    }
}
fn script(lines: &[&[u8]]) -> Result<Vm> {
    let mut records = Vec::new();
    for line in lines {
        records.extend([0x0b, 0, 0, 1]);
        records.extend(*line);
        records.push(0);
    }
    records.extend([0x18, 0, 0, 0]);
    let mut vm = crate::scenario_load_tests::vm(1)?;
    vm.script = Some(Script::new(
        crate::scenario_load_tests::scenario(1, &records, lines.len() as u32 + 1),
        1,
    )?);
    Ok(vm)
}
#[test]
fn v1_preparing_an_effect_replaces_only_its_lane_without_starting_audio() -> Result<()> {
    let mut vm = script(&[b"35,9,2,1,31", b"35,9,2,-5,-1", b"35,9,2,1,1080"])?;
    vm.registers.set_local(9, 78)?;
    let mut host = Host {
        playing: true,
        ..Default::default()
    };
    vm.step(&mut host, InputFrame::default(), 20)?;
    vm.step(&mut host, InputFrame::default(), 20)?;
    assert!(vm.preparation_pending());
    assert_eq!(host.files, ["res/s/e/031.wav"]);
    host.ready = true;
    for _ in 0..3 {
        vm.step(&mut host, InputFrame::default(), 20)?;
    }
    assert_eq!(
        host.files,
        ["res/s/e/031.wav", "res/s/e/0-1.wav", "res/s/e/080.wav"]
    );
    assert_eq!(host.receipts, [1, 2, 3]);
    assert_eq!(host.released, [1]);
    let channels: Vec<_> = host
        .audio
        .iter()
        .map(|r| match r {
            AudioRequest::Stop {
                channel,
                fade_ms: 0.0,
            } => *channel,
            _ => panic!("preparing must not play"),
        })
        .collect();
    assert_eq!(channels, [15, 14, 15]);
    assert_eq!(vm.registers.read(Value::Local(9))?, 78);
    Ok(())
}
#[test]
fn v1_movie_gallery_waits_then_advances_each_complete_group() -> Result<()> {
    let mut vm = script(&[b"42,9,2,3,-1.9.0.0..3.26.99.8.0x4.-7.9.8.5.9.0"])?;
    let mut h = Host::default();
    let step = |vm: &mut Vm, h: &mut Host| vm.step(h, InputFrame::default(), 3);
    step(&mut vm, &mut h)?;
    step(&mut vm, &mut h)?;
    assert_eq!(h.movies, ["res/g/ev/00/0003.mpg"]);
    assert_eq!(vm.registers.read(Value::Seen(registers::Bank::Cg, 3))?, 1);
    let AudioRequest::PlayAsset {
        channel: 15,
        source,
        parameters,
        ..
    } = &h.audio[0]
    else {
        panic!("movie effect")
    };
    assert_eq!(source.path.relative(), "res/s/e/026.wav");
    assert!(matches!(parameters.bus, PcmBus::Sfx));
    assert!(parameters.pcm16_gain.is_some());
    h.ended = true;
    h.now = 999;
    step(&mut vm, &mut h)?;
    assert_eq!(h.movies.len(), 1);
    h.ended = false;
    h.now = 1000;
    step(&mut vm, &mut h)?;
    assert_eq!(h.movies.len(), 1);
    h.ended = true;
    step(&mut vm, &mut h)?;
    assert_eq!(
        h.movies,
        ["res/g/ev/00/0003.mpg", "removed", "res/g/ev/00/0004.mpg"]
    );
    assert_eq!(vm.registers.read(Value::Seen(registers::Bank::Cg, 4))?, 1);
    assert!(matches!(
        &h.audio[1],
        AudioRequest::Stop {
            channel: 15,
            fade_ms: 0.0
        }
    ));
    h.now = 2000;
    step(&mut vm, &mut h)?;
    assert!(vm.halted);
    assert_eq!(h.movies.len(), 4);
    assert_eq!(h.layers, ["put", "remove", "put", "remove"]);
    assert_eq!(vm.registers.read(Value::Seen(registers::Bank::Cg, 5))?, 0);
    Ok(())
}
