// SPDX-License-Identifier: MPL-2.0
//! V1 effect commands can address the loose voice bank instead of the WAV bank.
use anyhow::{Result, bail, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{
        AudioParameters, AudioPcm16Gain, AudioRequest, HostServices, PcmBus, ResidentAudioJob,
        ResidentAudioState,
    },
    state::ClientId,
};
use crysirror_resource::{DecodeContext, DecodedFile, FileFormat, exhibit_pcm::ExhibitPcm};
use crysirror_tasks::{Task, TaskPool};

pub(super) const CHANNEL: u32 = 2;
pub(super) struct Voice {
    owner: ClientId,
    source: Source,
    pub id: i32,
    pub wait: bool,
    stereo: [u32; 2],
}
enum Source {
    Decoding(Task<Option<DecodedFile>>),
    Preparing(ResidentAudioJob),
}
impl Voice {
    /// Prepare the authored ordinary voice using the loose PCM path. Do not
    /// enter this from the effect-voice command: its gain and message marks
    /// belong to a different native caller. Dialogue owns playback/advance.
    pub fn begin_character(
        h: &mut dyn HostServices,
        worker: &TaskPool,
        owner: ClientId,
        characters: &crate::character::Catalog,
        registers: &crate::registers::Registers,
        character: u8,
        ordinal: i32,
    ) -> Result<Option<Self>> {
        if ordinal < 0 {
            return Ok(None);
        }
        let Some(gain) = characters.unpositioned_voice_gain(character, registers)? else {
            return Ok(None);
        };
        Ok(Some(Self::begin(h, worker, owner, ordinal, false, gain)?))
    }
    pub fn begin(
        h: &mut dyn HostServices,
        worker: &TaskPool,
        c: ClientId,
        id: i32,
        wait: bool,
        stereo: [u32; 2],
    ) -> Result<Self> {
        let storage = h.storage(c)?;
        let path = VfsPath::new(Mount::Asset, &format!("res/s/v/{:02}/{id:05}.v", id / 1000))?;
        let task = worker.spawn(move |cancel| {
            cancel.check()?;
            if storage.metadata(Mount::Asset, path.relative())?.is_none() {
                return Ok(None);
            }
            let input = storage.open_read(&path)?;
            let ctx = DecodeContext::new(c).with_cancellation(cancel);
            Ok(Some(ExhibitPcm.decode(path.relative(), input, &ctx)?))
        })?;
        Ok(Self {
            owner: c,
            source: Source::Decoding(task),
            id,
            wait,
            stereo,
        })
    }
    pub fn poll(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        ensure!(c == self.owner, "foreign ExHibit voice owner");
        if let Source::Decoding(task) = &mut self.source {
            let Some(file) = task.try_take()? else {
                return Ok(false);
            };
            // playVoice checks for existence before touching the current voice.
            let Some(file) = file else { return Ok(true) };
            self.source = Source::Preparing(h.begin_resident_audio(
                c,
                file.reader,
                "exhibit-voice.wav",
                None,
            )?);
        }
        if let Source::Preparing(job) = &mut self.source {
            ensure!(job.client() == c, "foreign ExHibit voice preparation");
            match job.poll()? {
                ResidentAudioState::Pending => return Ok(false),
                ResidentAudioState::Failed(reason) => {
                    bail!("ExHibit voice preparation failed: {reason:?}")
                }
                ResidentAudioState::Ready(pcm) => {
                    h.submit_audio(
                        c,
                        AudioRequest::PlayPcm {
                            channel: CHANNEL,
                            source: pcm,
                            parameters: AudioParameters {
                                bus: PcmBus::Voice,
                                pcm16_gain: Some(AudioPcm16Gain {
                                    stereo: self.stereo,
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        },
                    )?;
                }
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{
        Error, Result as CoreResult,
        service::{
            AssetReader, ClientStorage, FileSystem, MemoryAssetReader, PcmAudio,
            ResidentAudioProcess, StorageMetadata,
        },
        state::Command,
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct Assets(std::collections::BTreeMap<String, Vec<u8>>);
    impl ClientStorage for Assets {
        fn read(&self, path: &VfsPath) -> CoreResult<Vec<u8>> {
            if path.mount() != Mount::Asset {
                return Err(Error::ReadOnly);
            }
            self.0.get(path.relative()).cloned().ok_or(Error::NotFound)
        }
        fn open_read(&self, path: &VfsPath) -> CoreResult<Arc<dyn AssetReader>> {
            Ok(Arc::new(MemoryAssetReader(self.read(path)?.into())))
        }
        fn metadata(&self, mount: Mount, path: &str) -> CoreResult<Option<StorageMetadata>> {
            if mount != Mount::Asset {
                return Err(Error::ReadOnly);
            }
            Ok(self.0.get(path).map(|bytes| StorageMetadata {
                is_directory: false,
                size: Some(bytes.len() as u64),
                created_ms: None,
                modified_ms: None,
                accessed_ms: None,
            }))
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> CoreResult<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> CoreResult<Vec<String>> {
            Err(Error::Unsupported("unused fixture list"))
        }
    }

    struct FixtureHost {
        owner: ClientId,
        assets: Arc<Assets>,
        prepared: usize,
        requests: Vec<AudioRequest>,
    }
    impl FileSystem for FixtureHost {
        fn read(&mut self, client: ClientId, path: &VfsPath) -> CoreResult<Vec<u8>> {
            if client != self.owner {
                return Err(Error::NotFound);
            }
            self.assets.read(path)
        }
        fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for FixtureHost {
        fn submit(&mut self, _: ClientId, _: &[Command]) -> CoreResult<()> {
            Ok(())
        }
        fn now_ms(&self) -> u64 {
            0
        }
        fn storage(&mut self, client: ClientId) -> CoreResult<Arc<dyn ClientStorage>> {
            if client != self.owner {
                return Err(Error::NotFound);
            }
            Ok(self.assets.clone())
        }
        fn begin_resident_audio(
            &mut self,
            client: ClientId,
            reader: Arc<dyn AssetReader>,
            name: &str,
            _: Option<u64>,
        ) -> CoreResult<ResidentAudioJob> {
            if client != self.owner {
                return Err(Error::NotFound);
            }
            assert_eq!(name, "exhibit-voice.wav");
            let bytes = crysirror_resource::read_all(reader.as_ref(), 16384).unwrap();
            assert!(bytes.starts_with(b"RIFF"));
            let data = bytes.windows(4).position(|tag| tag == b"data").unwrap() + 8;
            assert_eq!(&bytes[data..data + 4], &[9, 0, 9, 0]);
            self.prepared += 1;
            Ok(Box::new(Prepared(client)))
        }
        fn submit_audio(&mut self, client: ClientId, request: AudioRequest) -> CoreResult<()> {
            if client != self.owner {
                return Err(Error::NotFound);
            }
            self.requests.push(request);
            Ok(())
        }
    }

    #[test]
    fn synthetic_definition_uses_native_dispatch_then_owner_scoped_voice() -> Result<()> {
        use crysirror_runtime::{model::ClientSpec, runtime::VmFactory};
        let client = ClientId(7);
        let mut rld = b"\0DLR".to_vec();
        for word in [1u32, 16, 4] {
            rld.extend(word.to_le_bytes());
        }
        for raw in [
            b"5,24,1,stand,Example,,,,,,,,,\0".as_slice(),
            b"6,0,0,stand,Example,,,,,,,,,\0".as_slice(),
        ] {
            rld.extend([0x30, 0, 0, 1]);
            rld.extend(raw);
        }
        rld.extend([0x31, 0, 0, 1]);
        rld.extend(crate::character_body::tests::payload(5, 11));
        rld.push(0);
        rld.extend([0x8c, 0, 0, 0]); // ordinary v1 End
        let mut host = FixtureHost {
            owner: client,
            assets: Arc::new(Assets(
                [
                    ("rld/defChara.rld".into(), rld),
                    (
                        "res/s/v/00/00001.v".into(),
                        vec![0, 0, 0, 0, 1, 0, 1, 0, 4, 9, 0],
                    ),
                ]
                .into(),
            )),
            prepared: 0,
            requests: Vec::new(),
        };
        let spec: ClientSpec = serde_json::from_value(serde_json::json!({
            "name":"test", "adapter":"exhibit", "game":"generated",
            "config":{"entry":"defChara", "abi":1, "canvas":[800,600],
                "limits":{"regs":2048,"sys":4096,"cg":32,"scene":8,"sound":16,"strings":128,"characters":34}}
        }))?;
        let mut vm = crate::Factory.create(client, &spec)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if matches!(
                vm.step(&mut host, Default::default(), 10)?,
                crysirror_core::vm::VmYield::Halted
            ) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let diagnostics: serde_json::Value = serde_json::from_slice(&vm.diagnostics())?;
        assert_eq!(diagnostics["authored_characters"], 2);
        assert_eq!(diagnostics["authored_body_variants"], 1);
        assert!(host.requests.is_empty()); // definitions do not play audio

        let pool = TaskPool::new("voice-fixture", 1, 1)?;
        let mut catalog = crate::character::Catalog::new(37);
        catalog.define(crate::character::Definition::parse(
            b"5,24,1,stand,Example,,,,,,,,,",
        )?)?;
        catalog.define(crate::character::Definition::parse(
            b"6,0,0,stand,Example,,,,,,,,,",
        )?)?;
        let mut regs = crate::native_state_tests::fresh(1)?;
        assert!(
            Voice::begin_character(&mut host, &pool, client, &catalog, &regs, 5, -1)?.is_none()
        );
        assert!(Voice::begin_character(&mut host, &pool, client, &catalog, &regs, 6, 1)?.is_none());
        regs.set_system(53, 1)?;
        assert!(Voice::begin_character(&mut host, &pool, client, &catalog, &regs, 5, 1)?.is_none());
        regs.set_system(53, 0)?;
        regs.set_system(54, 192)?;
        regs.set_system(38, 27)?;
        let mut voice =
            Voice::begin_character(&mut host, &pool, client, &catalog, &regs, 5, 1)?.unwrap();
        assert!(voice.poll(&mut host, ClientId(8)).is_err());
        assert!(host.requests.is_empty());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !voice.poll(&mut host, client)? {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(host.prepared, 1);
        let [
            AudioRequest::PlayPcm {
                channel,
                parameters,
                ..
            },
        ] = host.requests.as_slice()
        else {
            bail!("expected one prepared voice");
        };
        assert_eq!(*channel, CHANNEL);
        assert_eq!(parameters.pcm16_gain.unwrap().stereo, [192, 192]);
        let mut absent =
            Voice::begin_character(&mut host, &pool, client, &catalog, &regs, 5, 2)?.unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !absent.poll(&mut host, client)? {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!((host.prepared, host.requests.len()), (1, 1));
        Ok(())
    }

    struct Prepared(ClientId);
    impl ResidentAudioProcess for Prepared {
        fn client(&self) -> ClientId {
            self.0
        }
        fn poll(&mut self) -> crysirror_core::Result<ResidentAudioState> {
            Ok(ResidentAudioState::Ready(PcmAudio::new(
                44100,
                vec![[0.25, -0.25]],
            )?))
        }
    }

    #[test]
    fn native_voice_is_published_on_its_bus_with_scoped_pcm_and_stereo_gain() -> Result<()> {
        let client = ClientId(7);
        let mut host = crate::sound::tests::Host::default();
        let mut voice = Voice {
            owner: client,
            source: Source::Preparing(Box::new(Prepared(client))),
            id: 42,
            wait: false,
            stereo: [64, 128],
        };
        assert!(voice.poll(&mut host, ClientId(8)).is_err());
        assert!(host.requests.is_empty());
        assert!(voice.poll(&mut host, client)?);
        let [
            AudioRequest::PlayPcm {
                channel,
                source,
                parameters,
            },
        ] = host.requests.as_slice()
        else {
            bail!("expected a resident voice request");
        };
        assert_eq!(*channel, CHANNEL);
        assert!(matches!(parameters.bus, PcmBus::Voice));
        assert_eq!(parameters.pcm16_gain.unwrap().stereo, [64, 128]);
        assert_eq!(source.frames(), &[[0.25, -0.25]]);
        Ok(())
    }
}
