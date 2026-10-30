// SPDX-License-Identifier: MPL-2.0
//! Native MOV names/commands -> client-owned engine Media resources and layers.
use super::*;
use crate::movie::MovieCommand;
use crysirror_core::service::{MediaPhase, MediaRequest};
pub(crate) const MEDIA_ID: u32 = 1023;

impl SiglusVm {
    pub(super) fn movie_asset(&self, host: &mut dyn HostServices, name: &str) -> Result<String> {
        if let Some(alias) = self.state.config.resources.get(name) {
            return Ok(alias.clone());
        }
        let normalized = name.replace('\\', "/");
        VfsPath::new(Mount::Asset, &normalized).map_err(err)?;
        let append = &self.state.vm.as_ref().unwrap().ctx.globals.append_dir;
        ensure!(
            append.is_empty(),
            "Siglus MOV append lookup requires an explicit resource alias"
        );
        let storage = host.storage(ClientId(self.state.client))?;
        let find = |candidate: &str| -> Result<Option<String>> {
            Ok(
                crysirror_resource::capability_lookup::resolve_ascii_case_insensitive(
                    storage.as_ref(),
                    &VfsPath::new(Mount::Asset, candidate)?,
                )?
                .map(|p| p.relative().to_owned()),
            )
        };
        if normalized.contains('/') {
            if let Some(path) = find(&normalized)? {
                return Ok(path);
            }
        }
        let base = normalized.as_str();
        let ext = std::path::Path::new(base).extension();
        if let Some(ext) = ext {
            ensure!(
                matches!(
                    ext.to_str().unwrap_or("").to_ascii_lowercase().as_str(),
                    "wmv" | "mpg" | "avi" | "omv"
                ),
                "unsupported native MOV extension"
            );
            if let Some(path) = find(&format!("mov/{base}"))? {
                return Ok(path);
            }
        } else {
            // Original Siglus MOV_ORDER, not a codec-dependent fallback order.
            for ext in ["wmv", "mpg", "avi", "omv"] {
                if let Some(path) = find(&format!("mov/{base}.{ext}"))? {
                    return Ok(path);
                }
            }
        }
        anyhow::bail!("Siglus MOV resource not found: {name}")
    }
    pub(super) fn native_movie_gain(&self) -> f32 {
        let config = &self
            .state
            .vm
            .as_ref()
            .unwrap()
            .ctx
            .globals
            .syscom
            .original_config;
        let raw = if config.play_all_sound_check && config.play_sound_check[4] {
            config.all_sound_user_volume.clamp(0, 255) * config.sound_user_volume[4].clamp(0, 255)
                / 255
        } else {
            0
        };
        raw as f32 / 255.
    }
    pub(super) fn flush_movie(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let vm = self.state.vm.as_mut().unwrap();
        if vm.ctx.movie.live && !vm.ctx.globals.mov.playing {
            vm.ctx.globals.mov.stop();
            vm.ctx.movie.stop();
        }
        let pending = std::mem::take(&mut vm.ctx.movie.pending);
        for command in pending {
            match command {
                MovieCommand::Play(name) => {
                    let asset = self.movie_asset(host, &name)?;
                    // Native category 4 is MOV; preserve integer 0..255 master
                    // multiplication before converting to the engine gain.
                    let config = &self
                        .state
                        .vm
                        .as_ref()
                        .unwrap()
                        .ctx
                        .globals
                        .syscom
                        .original_config;
                    let raw = if config.play_all_sound_check && config.play_sound_check[4] {
                        config.all_sound_user_volume.clamp(0, 255)
                            * config.sound_user_volume[4].clamp(0, 255)
                            / 255
                    } else {
                        0
                    };
                    let path = host.media_request(
                        ClientId(self.state.client),
                        MediaRequest::Play {
                            id: MEDIA_ID,
                            asset,
                            looping: false,
                            audio: true,
                            gain: raw as f32 / 255.,
                        },
                    )?;
                    let movie = &mut self.state.vm.as_mut().unwrap().ctx.movie;
                    movie.surface = Some(path.relative().into());
                    movie.ready = false;
                }
                MovieCommand::Stop => {
                    host.media_request(
                        ClientId(self.state.client),
                        MediaRequest::Stop { id: MEDIA_ID },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn sync_movie(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let vm = self.state.vm.as_mut().unwrap();
        if !vm.ctx.movie.live || vm.ctx.movie.surface.is_none() {
            return Ok(());
        }
        let status = host.media_info(ClientId(self.state.client), MEDIA_ID)?;
        ensure!(
            status.phase != MediaPhase::Failed,
            "Siglus engine MOV resource failed"
        );
        vm.ctx.globals.mov.timer_ms = status.position_ms;
        if status.published_frames > 0 {
            ensure!(
                host.media_frame(ClientId(self.state.client), MEDIA_ID)?
                    .is_some(),
                "MOV published frames lack owner picture"
            );
            vm.ctx.movie.ready = true;
        }
        if matches!(status.phase, MediaPhase::Ended | MediaPhase::Stopped) {
            vm.ctx.globals.mov.stop();
            vm.ctx.movie.stop();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        constants::{cd, fm},
        forms::codes,
    };
    use crysirror_core::{
        service::{FileSystem, MediaFrame, MediaInfo},
        state::Command,
    };
    #[derive(Default)]
    struct Host {
        bytes: Vec<u8>,
        now: u64,
        engine: crysirror_scene::EngineSystems,
        requests: Vec<MediaRequest>,
        info: Option<MediaInfo>,
        picture: Option<MediaFrame>,
    }
    impl FileSystem for Host {
        fn read(&mut self, client: ClientId, path: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            if client == ClientId(7)
                && path.mount() == Mount::Asset
                && path.relative() == "movie.scn"
            {
                Ok(self.bytes.clone())
            } else {
                Err(Error::NotFound)
            }
        }
        fn write_atomic(
            &mut self,
            _: ClientId,
            _: &VfsPath,
            _: &[u8],
        ) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Host {
        fn now_ms(&self) -> u64 {
            self.now
        }
        fn submit(&mut self, _: ClientId, _: &[Command]) -> crysirror_core::Result<()> {
            Ok(())
        }
        fn submit_engine(
            &mut self,
            client: ClientId,
            schema: u32,
            data: &[u8],
        ) -> crysirror_core::Result<()> {
            assert_eq!(schema, 1);
            let batch = serde_json::from_slice(data).map_err(err)?;
            self.engine.submit(client, &batch).map_err(err)
        }
        fn media_request(
            &mut self,
            client: ClientId,
            request: MediaRequest,
        ) -> crysirror_core::Result<VfsPath> {
            if client != ClientId(7) {
                return Err(Error::NotFound);
            }
            match &request {
                MediaRequest::SetClock { id, .. } => {
                    assert!(*id < MEDIA_ID);
                    self.requests.push(request);
                    return VfsPath::new(Mount::Asset, "__surface/1895826431");
                }
                MediaRequest::Pause { .. } | MediaRequest::Resume { .. } => {
                    self.info.as_mut().unwrap().paused =
                        matches!(request, MediaRequest::Pause { .. });
                    self.requests.push(request);
                    return VfsPath::new(Mount::Asset, "__surface/1895826431");
                }
                MediaRequest::Remove { .. } => {
                    self.info = None;
                    self.picture = None;
                    self.requests.push(request);
                    return VfsPath::new(Mount::Asset, "__surface/1895826431");
                }
                _ => {}
            }
            self.info = Some(MediaInfo {
                phase: match &request {
                    MediaRequest::Play {
                        id, asset, audio, ..
                    } => {
                        assert!(*id <= MEDIA_ID);
                        assert_eq!(asset, "movies/opening.wmv");
                        assert!(*audio);
                        MediaPhase::Loading
                    }
                    MediaRequest::Stop { id } => {
                        assert_eq!(*id, MEDIA_ID);
                        MediaPhase::Stopped
                    }
                    other => panic!("unexpected media request {other:?}"),
                },
                position_ms: 0,
                published_frames: 0,
                paused: false,
                frame_ms: 0,
                size: None,
                duration_ms: None,
            });
            self.requests.push(request);
            VfsPath::new(Mount::Asset, "__surface/1895826431")
        }
        fn media_info(&self, client: ClientId, id: u32) -> crysirror_core::Result<MediaInfo> {
            if client != ClientId(7) || id > MEDIA_ID {
                return Err(Error::NotFound);
            }
            self.info.ok_or(Error::NotFound)
        }
        fn media_frame(
            &self,
            client: ClientId,
            id: u32,
        ) -> crysirror_core::Result<Option<MediaFrame>> {
            if client != ClientId(7) || id > MEDIA_ID {
                return Err(Error::NotFound);
            }
            Ok(self.picture.clone())
        }
    }
    fn fixture(op: i32) -> (SiglusVm, Host) {
        let mut s = crate::fixture::SceneBuilder::default();
        s.command_string(
            &[codes::ELM_GLOBAL_MOV, op],
            0,
            "opening",
            &[12, 24, 320, 180],
            fm::INT,
        );
        s.int(77);
        s.code.push(cd::EOF);
        let config = Config {
            scene: "movie.scn".into(),
            format: "scn".into(),
            scene_name: None,
            string_codec: "plain".into(),
            gameexe: None,
            key_file: None,
            resources: BTreeMap::from([("opening".into(), "movies/opening.wmv".into())]),
            message_window: None,
        };
        (
            SiglusVm::new(ClientId(7), config).unwrap(),
            Host {
                bytes: s.finish(),
                ..Default::default()
            },
        )
    }
    #[test]
    fn original_movie_bytecode_waits_on_engine_frames_and_eof() {
        let (mut vm, mut host) = fixture(codes::mov_op::PLAY_WAIT);
        assert_ne!(
            vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
            VmYield::Halted
        );
        assert_eq!(host.requests.len(), 1);
        assert!(
            vm.snapshot().is_err(),
            "live resource must not masquerade as a restorable snapshot"
        );
        host.now = 16;
        host.info = Some(MediaInfo {
            phase: MediaPhase::Playing,
            position_ms: 31,
            published_frames: 1,
            paused: false,
            frame_ms: 0,
            size: Some([1, 1]),
            duration_ms: None,
        });
        host.picture = Some(MediaFrame::new([1, 1], vec![255, 20, 0, 255], 1, 31).unwrap());
        vm.step(&mut host, InputFrame::default(), 1024).unwrap();
        let layer = host.engine.layers.get(ClientId(7), 0xA000_0000).unwrap();
        assert_eq!(layer.rect, [12., 24., 320., 180.]);
        assert!(
            matches!(&layer.paint,crysirror_scene::layer::Paint::Image {asset,..} if asset=="__surface/1895826431")
        );
        assert!(host.engine.layers.get(ClientId(8), 0xA000_0000).is_none());
        assert_eq!(
            vm.state.vm.as_ref().unwrap().ctx.globals.mov.timer_ms,
            31,
            "native frame ticking cannot advance the engine media clock"
        );
        host.now = 32;
        host.info.as_mut().unwrap().phase = MediaPhase::Ended;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
            VmYield::Halted
        );
        assert!(host.engine.layers.get(ClientId(7), 0xA000_0000).is_none());
        let diagnostics = vm.state.vm.as_ref().unwrap().engine_diagnostics();
        assert_eq!(
            diagnostics["int_stack"].as_array().unwrap().last(),
            Some(&serde_json::json!(77))
        );
        assert!(matches!(
            host.requests.last(),
            Some(MediaRequest::Stop { .. })
        ));
    }
    #[test]
    fn native_movie_cancel_is_a_down_up_edge_and_stops_the_engine() {
        let (mut vm, mut host) = fixture(codes::mov_op::PLAY_WAIT_KEY);
        vm.step(&mut host, InputFrame::default(), 1024).unwrap();
        host.now = 16;
        let bit = crysirror_core::service::buttons::ESCAPE;
        vm.step(
            &mut host,
            InputFrame {
                buttons_pressed: bit,
                buttons_down: bit,
                ..Default::default()
            },
            1024,
        )
        .unwrap();
        assert!(vm.state.vm.as_ref().unwrap().ctx.globals.mov.playing);
        host.now = 32;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
            VmYield::Halted
        );
        assert!(matches!(
            host.requests.last(),
            Some(MediaRequest::Stop { .. })
        ));
        let diagnostics = vm.state.vm.as_ref().unwrap().engine_diagnostics();
        assert!(
            vm.state
                .vm
                .as_ref()
                .unwrap()
                .ctx
                .globals
                .mov
                .file_name
                .is_none()
        );
        assert!(
            diagnostics["int_stack"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(2)),
            "native CANCEL return code is 2"
        );
    }
    #[test]
    fn native_object_movie_waits_for_engine_metadata_and_uses_engine_picture_clock_and_removal() {
        let (mut vm, mut host) = fixture(codes::mov_op::PLAY_WAIT);
        let mut scene = crate::fixture::SceneBuilder::default();
        let object = [
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
        ];
        let op = |code| object.into_iter().chain([code]).collect::<Vec<_>>();
        scene.command_string(
            &op(codes::ELM_OBJECT_CREATE_MOVIE),
            2,
            "opening",
            &[1, 12, 24],
            fm::VOID,
        );
        scene.command(&op(codes::ELM_OBJECT_GET_SIZE_X), 0, &[], fm::INT);
        scene.command(&op(codes::ELM_OBJECT_GET_SIZE_Y), 0, &[], fm::INT);
        scene.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[5000], fm::VOID);
        scene.code.push(cd::EOF);
        host.bytes = scene.finish();
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
            VmYield::BudgetExhausted
        );
        assert_eq!(
            host.requests
                .iter()
                .filter(|r| matches!(r, MediaRequest::Play { .. }))
                .count(),
            1
        );
        assert!(vm.snapshot().is_err());
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
            VmYield::BudgetExhausted
        );
        assert_eq!(
            host.requests
                .iter()
                .filter(|r| matches!(r, MediaRequest::Play { .. }))
                .count(),
            1,
            "pending metadata does not create another decoder"
        );
        host.info.as_mut().unwrap().size = Some([320, 180]);
        host.now = 16;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
            VmYield::WaitUntil(5016)
        );
        let diagnostics = vm.diagnostics();
        assert_eq!(diagnostics["int_stack"], serde_json::json!([320, 180]));
        assert!(!host.engine.frame([640,480]).unwrap().iter().any(|p| matches!(&p.paint, crysirror_scene::layer::Paint::Image { asset,.. } if asset == "__surface/1895826431")), "metadata is not a decoded picture");
        host.now = 32;
        host.info.as_mut().unwrap().phase = MediaPhase::Playing;
        host.info.as_mut().unwrap().position_ms = 31;
        host.info.as_mut().unwrap().published_frames = 1;
        host.picture = Some(MediaFrame::new([320, 180], vec![255; 320 * 180 * 4], 1, 31).unwrap());
        vm.step(&mut host, InputFrame::default(), 1024).unwrap();
        let paints = host.engine.frame([640, 480]).unwrap();
        let paint = paints.iter().find(|p| matches!(&p.paint, crysirror_scene::layer::Paint::Image {asset,..} if asset == "__surface/1895826431")).unwrap();
        assert_eq!(paint.extent, [320., 180.]);
        assert_eq!(paint.quad[0][..2], [12., 24.]);
        let ctx = &vm.state.vm.as_ref().unwrap().ctx;
        let object = &ctx
            .globals
            .stage_forms
            .values()
            .next()
            .unwrap()
            .object_lists[&1][0];
        assert_eq!(
            object.movie.timer_ms, 31,
            "native tick does not synthesize a media clock"
        );
        assert_eq!(object.movie.total_ms, None, "unknown duration is retained");
        host.requests.clear();
        vm.state
            .vm
            .as_mut()
            .unwrap()
            .ctx
            .globals
            .script
            .stage_time_stop_flag = true;
        host.now = 40;
        vm.step(&mut host, InputFrame::default(), 1024).unwrap();
        let pause = host
            .requests
            .iter()
            .position(|r| matches!(r, MediaRequest::Pause { id: 0 }))
            .unwrap();
        assert!(
            host.requests[..pause]
                .iter()
                .any(|r| matches!(r, MediaRequest::SetClock { id: 0, .. })),
            "source clock must be sampled before stage-pause admission"
        );
        assert!(
            host.requests
                .iter()
                .any(|r| matches!(r, MediaRequest::SetClock { id: 0, now_ms: 40 })),
            "an existing native TIMEWAIT still supplies its post-frame clock sample"
        );
        assert!(
            !vm.state
                .vm
                .as_ref()
                .unwrap()
                .ctx
                .globals
                .stage_forms
                .values()
                .next()
                .unwrap()
                .object_lists[&1][0]
                .movie
                .pause_flag
        );
        host.requests.clear();
        vm.state
            .vm
            .as_mut()
            .unwrap()
            .ctx
            .globals
            .script
            .stage_time_stop_flag = false;
        host.now = 44;
        vm.step(&mut host, InputFrame::default(), 1024).unwrap();
        let resume = host
            .requests
            .iter()
            .position(|r| matches!(r, MediaRequest::Resume { id: 0 }))
            .unwrap();
        assert!(
            host.requests[..resume]
                .iter()
                .any(|r| matches!(r, MediaRequest::SetClock { id: 0, .. }))
        );
        host.now = 48;
        host.info.as_mut().unwrap().phase = MediaPhase::Ended;
        vm.step(&mut host, InputFrame::default(), 1024).unwrap();
        assert!(matches!(
            host.requests.last(),
            Some(MediaRequest::Remove { id: 0 })
        ));
        assert!(!vm.object_movies.active());
        assert!(
            vm.snapshot().is_ok(),
            "autofree retired the external decoder before snapshot"
        );
        assert!(!host.engine.frame([640,480]).unwrap().iter().any(|p| matches!(&p.paint, crysirror_scene::layer::Paint::Image {asset,..} if asset == "__surface/1895826431")));
    }
    #[test]
    #[ignore = "writes a self-authored native fixture only with SIGLUS_MOV_FIXTURE_DIR"]
    fn write_authored_native_movie_fixture() {
        let root = PathBuf::from(
            std::env::var_os("SIGLUS_MOV_FIXTURE_DIR").expect("SIGLUS_MOV_FIXTURE_DIR"),
        );
        std::fs::create_dir_all(root.join("assets/movies")).unwrap();
        let (_, host) = fixture(codes::mov_op::PLAY_WAIT);
        std::fs::write(root.join("assets/movie.scn"), host.bytes).unwrap();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crysirror-video/tests/fixtures/wmv_wma.asf");
        std::fs::copy(source, root.join("assets/movies/opening.wmv")).unwrap();
        let project = serde_json::json!({"schema":2,"name":"Authored native Siglus Movie","content_revision":"1","canvas":[320,180],"assets":"assets",
            "clients":[{"name":"native","adapter":"siglus","game":"authored-movie","asset_prefix":"","config":{
                "scene":"movie.scn","format":"scn","string_codec":"plain","resources":{"opening":"movies/opening.wmv"}}}]});
        std::fs::write(
            root.join("project.json"),
            serde_json::to_vec_pretty(&project).unwrap(),
        )
        .unwrap();
    }
}
