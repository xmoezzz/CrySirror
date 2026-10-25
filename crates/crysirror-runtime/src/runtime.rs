// SPDX-License-Identifier: MPL-2.0
use crate::{
    assets::Assets,
    model::{Action, ClientSpec, Project},
    script::ScriptVm,
    vfs::DiskVfs,
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    Error,
    path::VfsPath,
    save::{SaveEnvelope, SaveIdentity, SaveLimits},
    service::{FileSystem, HostServices, InputFrame},
    state::{ClientId, Command, Limits, SceneState},
    vm::{VmAdapter, VmYield},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};
fn core<T>(r: crysirror_core::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow::anyhow!("{e}"))
}

/// A factory must construct a dormant VM: no filesystem/audio/window side effects.
/// This permits transactional restore by staging all replacement VMs first.
pub trait VmFactory: Send + Sync {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>>;
}
struct SiglusFactory;
impl VmFactory for SiglusFactory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "Siglus executes native bytecode; data VM program must be empty"
        );
        let config = serde_json::from_value(spec.config.clone()).context("native Siglus config")?;
        Ok(Box::new(crysirror_siglus_vm::adapter::SiglusVm::new(
            client, config,
        )?))
    }
}
struct ScriptFactory;
impl VmFactory for ScriptFactory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.config.is_null(),
            "built-in data script does not accept native config"
        );
        Ok(Box::new(ScriptVm::new(client, spec.program.clone())?))
    }
}
#[derive(Clone)]
pub struct AdapterRegistry {
    factories: BTreeMap<String, Arc<dyn VmFactory>>,
}
impl Default for AdapterRegistry {
    fn default() -> Self {
        let mut r = Self {
            factories: BTreeMap::new(),
        };
        r.factories
            .insert("crysirror-script-v1".into(), Arc::new(ScriptFactory));
        r.factories.insert("siglus".into(), Arc::new(SiglusFactory));
        r
    }
}
impl AdapterRegistry {
    pub fn register(&mut self, name: &str, factory: Arc<dyn VmFactory>) -> Result<()> {
        ensure!(
            crate::model::identity(name) && !self.factories.contains_key(name),
            "invalid/duplicate adapter factory"
        );
        self.factories.insert(name.into(), factory);
        Ok(())
    }
    fn create(&self, id: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        let f=self.factories.get(&spec.adapter).with_context(||format!("native adapter '{}' is not linked; RFVP/Siglus are not emulated by the demo script",spec.adapter))?;
        let vm = f.create(id, spec)?;
        ensure!(
            vm.engine_id() == spec.adapter,
            "factory returned the wrong engine"
        );
        Ok(vm)
    }
    pub fn names(&self) -> Vec<&str> {
        self.factories.keys().map(String::as_str).collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceEvent {
    pub frame: u64,
    pub time_ms: u64,
    pub client: u32,
    pub commands: Vec<String>,
}
pub struct SharedHost {
    pub scene: SceneState,
    pub engine: crysirror_scene::EngineSystems,
    pub fs: DiskVfs,
    clock_ms: u64,
    clients: BTreeSet<ClientId>,
    pub trace: VecDeque<TraceEvent>,
    generated: Vec<(ClientId, String, Vec<u8>)>,
    surfaces: Vec<(ClientId, u32, u32, u32, Vec<u8>)>,
    pcm: BTreeMap<ClientId, Vec<[f32; 2]>>,
    pub channel_audio: crysirror_audio::channels::ChannelAudio,
    pub media: crate::media::MediaManager,
    pub text: crysirror_text::TextSystem,
    decoders: crysirror_image::DecoderRegistry,
    image_dimensions: BTreeMap<(ClientId, String), ([u32; 2], bool)>,
}
impl SharedHost {
    /// Drain all client PCM blocks for one host frame, mixing without clipping.
    /// This is derived audio data and never part of Runtime snapshots.
    pub fn take_pcm_output(&mut self) -> Vec<[f32; 2]> {
        let blocks = std::mem::take(&mut self.pcm);
        let native = self.channel_audio.take_output();
        let count = blocks
            .values()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(native.len());
        let mut mixed = vec![[0.; 2]; count];
        for (out, frame) in mixed.iter_mut().zip(native) {
            *out = frame;
        }
        for block in blocks.values() {
            for (out, frame) in mixed.iter_mut().zip(block) {
                out[0] += frame[0];
                out[1] += frame[1];
            }
        }
        mixed
    }
}
impl FileSystem for SharedHost {
    fn read(&mut self, c: ClientId, p: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        self.fs.read(c, p)
    }
    fn write_atomic(&mut self, c: ClientId, p: &VfsPath, b: &[u8]) -> crysirror_core::Result<()> {
        self.fs.write_atomic(c, p, b)
    }
}
impl HostServices for SharedHost {
    fn audio_playing(&self, client: ClientId, channel: u32) -> crysirror_core::Result<bool> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        Ok(self.channel_audio.is_playing(client.0, channel))
    }
    fn query_engine(
        &mut self,
        client: ClientId,
        query: crysirror_core::service::EngineQuery,
    ) -> crysirror_core::Result<Vec<u8>> {
        use crysirror_core::service::EngineQuery as Q;
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        let result = match query {
            Q::Layer(id) => serde_json::to_value(self.engine.layers.get(client, id)),
            Q::Canvas(id) => serde_json::to_value(self.engine.canvases.surface(client, id)),
            Q::UiWidget(id) => serde_json::to_value(self.engine.ui.get(client.0, id)),
            Q::UiEvents => {
                let mut selected = Vec::new();
                self.engine.ui.events.retain(|e| {
                    if e.widget.owner == client.0 {
                        selected.push(e.clone());
                        false
                    } else {
                        true
                    }
                });
                serde_json::to_value(selected)
            }
        }
        .map_err(|e| Error::Io(e.to_string()))?;
        serde_json::to_vec(&result).map_err(|e| Error::Io(e.to_string()))
    }

    fn image_size(&mut self, client: ClientId, asset: &str) -> crysirror_core::Result<[u32; 2]> {
        let key = (client, asset.to_owned());
        if let Some((size, _)) = self.image_dimensions.get(&key) {
            return Ok(*size);
        }
        let path = VfsPath::new(crysirror_core::path::Mount::Asset, asset)?;
        let storage = self.fs.storage(client)?;
        let bytes = storage.read(&path)?;
        let source =
            |name: &str| storage.read(&VfsPath::new(crysirror_core::path::Mount::Asset, name)?);
        let image = self
            .decoders
            .decode_with(asset, &bytes, Some(&source), None, None)
            .map_err(|e| Error::Io(e.to_string()))?;
        let size = [image.width, image.height];
        if self.image_dimensions.len() >= 4096 {
            return Err(Error::Limit("image metadata cache"));
        }
        self.image_dimensions.insert(
            key,
            (
                size,
                image.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
            ),
        );
        Ok(size)
    }

    fn image_is_opaque(&mut self, client: ClientId, asset: &str) -> crysirror_core::Result<bool> {
        self.image_size(client, asset)?;
        Ok(self.image_dimensions[&(client, asset.to_owned())].1)
    }
    fn font_metrics(
        &self,
        font: &str,
        size: f32,
        character: char,
    ) -> crysirror_core::Result<crysirror_core::service::FontMetrics> {
        self.text
            .metrics(font, size, character)
            .map_err(|e| Error::Io(e.to_string()))
    }
    fn media_request(
        &mut self,
        client: ClientId,
        request: crysirror_core::service::MediaRequest,
    ) -> crysirror_core::Result<VfsPath> {
        use crysirror_core::{path::Mount, service::MediaRequest};
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        let id = match &request {
            MediaRequest::Play { id, .. } | MediaRequest::Stop { id } => *id,
        };
        if id >= 1024 {
            return Err(Error::Limit("media identity"));
        }
        let slot = 0x7100_0000 + id;
        match request {
            MediaRequest::Play {
                asset,
                looping,
                audio,
                gain,
                ..
            } => {
                let path = VfsPath::new(Mount::Asset, &asset)?;
                let storage = self.fs.storage(client)?;
                self.media
                    .play(
                        client,
                        id,
                        storage,
                        &path,
                        crate::media::MediaOptions {
                            looping,
                            audio,
                            gain,
                            audio_channel: 0x7000_0000 + id,
                        },
                        self.clock_ms,
                    )
                    .map_err(|e| Error::Io(e.to_string()))?;
                self.surfaces.push((client, slot, 1, 1, vec![0, 0, 0, 255]));
            }
            MediaRequest::Stop { .. } => self
                .media
                .stop(client, id)
                .map_err(|e| Error::Io(e.to_string()))?,
        }
        for audio in self.media.take_audio() {
            self.channel_audio
                .submit(audio.client.0, audio.request)
                .map_err(|e| Error::Io(e.to_string()))?;
        }
        VfsPath::new(Mount::Asset, &format!("__surface/{slot}"))
    }
    fn media_info(
        &self,
        client: ClientId,
        id: u32,
    ) -> crysirror_core::Result<crysirror_core::service::MediaInfo> {
        use crate::media::MediaState as S;
        use crysirror_core::service::{MediaInfo, MediaPhase as P};
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        let state = self.media.status(client, id).ok_or(Error::NotFound)?;
        Ok(MediaInfo {
            phase: match state.state {
                S::Loading => P::Loading,
                S::Playing => P::Playing,
                S::Ended => P::Ended,
                S::Stopped => P::Stopped,
                S::Failed => P::Failed,
            },
            position_ms: state.position_ms,
            published_frames: state.published_frames,
        })
    }

    fn submit_audio(
        &mut self,
        client: ClientId,
        request: crysirror_core::service::AudioRequest,
    ) -> crysirror_core::Result<()> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        self.channel_audio
            .submit(client.0, request)
            .map_err(|e| Error::Io(format!("shared audio: {e:#}")))
    }
    fn submit_pcm(
        &mut self,
        client: ClientId,
        sample_rate: u32,
        frames: &[[f32; 2]],
    ) -> crysirror_core::Result<()> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        if sample_rate != 48_000 {
            return Err(Error::Unsupported("PCM output sample rate"));
        }
        if !frames
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= 256.)
        {
            return Err(Error::Invalid("PCM output samples"));
        }
        let before = self.pcm.get(&client).map_or(0, Vec::len);
        if before.checked_add(frames.len()).is_none_or(|n| n > 96_000) {
            return Err(Error::Limit("PCM output queue; drain between frames"));
        }
        self.pcm
            .entry(client)
            .or_default()
            .extend_from_slice(frames);
        Ok(())
    }
    fn submit(&mut self, client: ClientId, commands: &[Command]) -> crysirror_core::Result<()> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        self.scene
            .apply_batch(client, commands, Limits::default())?;
        if self.trace.len() == 256 {
            self.trace.pop_front();
        }
        self.trace.push_back(TraceEvent {
            frame: self.scene.frame(),
            time_ms: self.clock_ms,
            client: client.0,
            commands: commands
                .iter()
                .take(64)
                .map(|c| format!("{c:?}").chars().take(512).collect())
                .collect(),
        });
        Ok(())
    }
    fn now_ms(&self) -> u64 {
        self.clock_ms
    }
    fn submit_engine(
        &mut self,
        client: ClientId,
        schema: u32,
        data: &[u8],
    ) -> crysirror_core::Result<()> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        if schema != 1 {
            return Err(Error::Unsupported("engine batch schema"));
        }
        if data.len() > 4 * 1024 * 1024 {
            return Err(Error::Limit("engine batch"));
        }
        let batch: crysirror_scene::ServiceBatch =
            serde_json::from_slice(data).map_err(|e| Error::Io(e.to_string()))?;
        self.engine
            .submit(client, &batch)
            .map_err(|e| Error::Io(e.to_string()))
    }
    fn upload_rgba(
        &mut self,
        client: ClientId,
        id: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> crysirror_core::Result<VfsPath> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        let expected =
            crysirror_image::rgba_len(width, height).map_err(|e| Error::Io(e.to_string()))?;
        if rgba.len() != expected {
            return Err(Error::Invalid("RGBA length"));
        }
        let surface = crysirror_image::ImageSurface {
            width,
            height,
            rgba: rgba.to_vec(),
        };
        surface.validate().map_err(|e| Error::Io(e.to_string()))?;
        let path = format!(
            "__crysirror/{id}-{width}-{height}-{}.png",
            blake3::hash(rgba).to_hex()
        );
        let bytes = crysirror_image::encode_png(&surface).map_err(|e| Error::Io(e.to_string()))?;
        if self
            .generated
            .iter()
            .map(|(_, _, b)| b.len())
            .sum::<usize>()
            + bytes.len()
            > 64 * 1024 * 1024
        {
            return Err(Error::Limit("image upload batch"));
        }
        self.generated.push((client, path.clone(), bytes));
        VfsPath::new(crysirror_core::path::Mount::Asset, &path)
    }
    fn storage(
        &mut self,
        client: ClientId,
    ) -> crysirror_core::Result<Arc<dyn crysirror_core::service::ClientStorage>> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        self.fs.storage(client)
    }
    fn present_surface(
        &mut self,
        client: ClientId,
        slot: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> crysirror_core::Result<VfsPath> {
        if !self.clients.contains(&client) {
            return Err(Error::NotFound);
        }
        if !(1..=8192).contains(&width)
            || !(1..=8192).contains(&height)
            || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4
        {
            return Err(Error::Invalid("surface dimensions/length"));
        }
        // Last write in a tick wins; the asset store swaps it in after the tick.
        self.surfaces.retain(|s| !(s.0 == client && s.1 == slot));
        self.surfaces
            .push((client, slot, width, height, rgba.to_vec()));
        VfsPath::new(
            crysirror_core::path::Mount::Asset,
            &format!("{}{slot}", crate::assets::SURFACE_PREFIX),
        )
    }
}
// Restore grants immutable uploads only. It cannot read/write files, submit
// commands, advance clocks or touch the live scene while staging a VM.
struct RestoreResources<'a>(&'a mut SharedHost);
impl crysirror_core::service::FileSystem for RestoreResources<'_> {
    fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("restore resource file read"))
    }
    fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
}
impl HostServices for RestoreResources<'_> {
    fn now_ms(&self) -> u64 {
        self.0.clock_ms
    }
    fn submit(
        &mut self,
        _: ClientId,
        _: &[crysirror_core::state::Command],
    ) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("restore scene mutation"))
    }
    fn upload_rgba(
        &mut self,
        c: ClientId,
        id: u32,
        w: u32,
        h: u32,
        b: &[u8],
    ) -> crysirror_core::Result<VfsPath> {
        self.0.upload_rgba(c, id, w, h, b)
    }
}
struct Session {
    name: String,
    id: ClientId,
    vm: Box<dyn VmAdapter>,
    yielded: VmYield,
}
pub struct Runtime {
    /// Persistent CPU simulation derived from saved World body/joint data.
    pub physics: crate::physics::Physics,
    pub project: Project,
    pub host: SharedHost,
    pub assets: Assets,
    pub restored: u64,
    sessions: Vec<Session>,
    registry: AdapterRegistry,
    fingerprint: String,
    /// Decoders for world videos (visual state, rebuilt from the saved spec).
    videos: std::collections::HashMap<(u32, String), VideoPlayback>,
    /// Serial of the transition whose generated mask is in its surface slot.
    transition_mask: Option<u64>,
    /// Loaded puppet rigs by (client, rig asset) (ADR 0029).
    puppet_rigs: std::collections::HashMap<(u32, String), Arc<crysirror_puppet::PuppetRig>>,
}
/// Surface slot holding the generated Siglus pattern mask of a transition.
pub const TRANSITION_MASK_SLOT: u32 = 999;
struct VideoPlayback {
    asset: String,
    looping: bool,
    player: crysirror_video::Player,
    /// World playback time last applied to the player.
    synced_ms: u64,
    published: u64,
}
/// Decoder-side state of a world video.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoStatus {
    pub width: u32,
    pub height: u32,
    pub ended: bool,
    pub frames_presented: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VmSave {
    name: String,
    engine: String,
    schema: u32,
    bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientScene {
    client: u32,
    commands: Vec<Action>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServicesSave {
    clock_ms: u64,
    frame: u64,
    clients: Vec<ClientScene>,
    #[serde(default)]
    engine: Option<crysirror_scene::EngineSystems>,
}

pub fn asset_root(project_path: &Path, project: &Project) -> Result<PathBuf> {
    let root = project_path
        .canonicalize()?
        .parent()
        .context("project parent")?
        .to_path_buf();
    let assets = root
        .join(&project.assets)
        .canonicalize()
        .context("asset directory missing")?;
    ensure!(
        assets.starts_with(&root) && assets.is_dir(),
        "asset directory escapes project root"
    );
    Ok(assets)
}
impl Runtime {
    pub fn load(path: &Path, writable: &Path) -> Result<Self> {
        Self::with_registry(
            Project::load(path)?,
            path,
            writable,
            AdapterRegistry::default(),
        )
    }
    pub fn with_registry(
        project: Project,
        path: &Path,
        writable: &Path,
        registry: AdapterRegistry,
    ) -> Result<Self> {
        project.validate()?;
        let mut sessions = Vec::new();
        let mut engine = crysirror_scene::EngineSystems::default();
        // Reject missing factories before creating writable state directories.
        for (i, spec) in project.clients.iter().enumerate() {
            let id = ClientId((i + 1) as u32);
            sessions.push(Session {
                name: spec.name.clone(),
                id,
                vm: registry.create(id, spec)?,
                yielded: VmYield::BudgetExhausted,
            });
            if let Some(systems) = &spec.systems {
                engine.install(id, systems)?;
            }
        }
        let root = asset_root(path, &project)?;
        let mut assets = Assets::new(&root)?;
        let mut fs = core(DiskVfs::new(&root, writable))?;
        for (session, spec) in sessions.iter().zip(&project.clients) {
            core(fs.register_with_assets(
                session.id,
                &spec.adapter,
                &spec.game,
                &spec.asset_prefix,
            ))?;
            assets.register(session.id, &spec.asset_prefix)?;
            assets.refresh_storage(session.id, core(fs.storage(session.id))?)?;
        }
        let mut text = crysirror_text::TextSystem::default();
        for (alias, path) in &project.fonts {
            let bytes = assets.read(path)?.to_vec();
            text.add_font(alias, bytes.clone())?;
            assets.text.add_font(alias, bytes)?;
        }
        let clients = sessions.iter().map(|s| s.id).collect();
        let fingerprint = project.fingerprint()?;
        Ok(Self {
            physics: Default::default(),
            project,
            host: SharedHost {
                scene: SceneState::default(),
                engine,
                fs,
                clock_ms: 0,
                clients,
                trace: VecDeque::new(),
                generated: Vec::new(),
                surfaces: Vec::new(),
                pcm: BTreeMap::new(),
                channel_audio: Default::default(),
                media: Default::default(),
                text,
                decoders: assets.decoders.clone(),
                image_dimensions: BTreeMap::new(),
            },
            assets,
            restored: 0,
            sessions,
            registry,
            fingerprint,
            videos: Default::default(),
            transition_mask: None,
            puppet_rigs: Default::default(),
        })
    }
    /// Decode every world video up to its saved time and publish the frame
    /// to its client's surface slot (ADR 0020).
    fn sync_videos(&mut self) -> Result<()> {
        let specs: Vec<(u32, String, crysirror_scene::world::VideoSpec)> = self
            .host
            .engine
            .world
            .videos()
            .map(|(c, n, v)| (c, n.to_owned(), v.clone()))
            .collect();
        self.videos
            .retain(|k, _| specs.iter().any(|(c, n, _)| *c == k.0 && *n == k.1));
        for (client, name, spec) in specs {
            let key = (client, name.clone());
            let stale = self.videos.get(&key).map_or(true, |p| {
                p.asset != spec.asset || p.looping != spec.looping || spec.time_ms < p.synced_ms
            });
            if stale {
                let bytes = self.assets.read_for(ClientId(client), &spec.asset)?;
                let stream = crysirror_video::open(bytes.to_vec())
                    .with_context(|| format!("video {name}: {}", spec.asset))?;
                let player = crysirror_video::Player::new(stream, spec.looping)?;
                self.videos.insert(
                    key.clone(),
                    VideoPlayback {
                        asset: spec.asset.clone(),
                        looping: spec.looping,
                        player,
                        synced_ms: 0,
                        published: 0,
                    },
                );
            }
            let p = self.videos.get_mut(&key).expect("video inserted above");
            p.player.advance((spec.time_ms - p.synced_ms) as f64)?;
            p.synced_ms = spec.time_ms;
            if p.player.revision != p.published {
                if let Some(f) = p.player.frame() {
                    self.assets.put_surface(
                        ClientId(client),
                        spec.slot,
                        f.width,
                        f.height,
                        f.rgba.clone(),
                    )?;
                }
                p.published = p.player.revision;
            }
        }
        Ok(())
    }
    pub fn video_status(&self, client: ClientId, name: &str) -> Option<VideoStatus> {
        let p = self.videos.get(&(client.0, name.to_owned()))?;
        let info = p.player.info();
        Some(VideoStatus {
            width: info.width,
            height: info.height,
            ended: p.player.ended(),
            frames_presented: p.player.revision,
        })
    }
    pub fn tick(&mut self, delta_ms: u64, input: InputFrame) -> Result<()> {
        // PCM is frame output, like the current render product. A headless
        // caller may leave it unread; never replay it during a later frame.
        self.host.pcm.clear();
        let clock = self
            .host
            .clock_ms
            .checked_add(delta_ms)
            .context("clock overflow")?;
        let consumed = {
            let physics = &mut self.physics;
            let SharedHost { engine, fs, .. } = &mut self.host;
            match engine.tick_with(delta_ms, input, self.project.canvas, |world, dt| {
                physics.step(world, fs, dt)
            }) {
                Ok(consumed) => consumed,
                Err(error) => {
                    physics.invalidate();
                    return Err(error);
                }
            }
        };
        self.host.clock_ms = clock;
        let input = if consumed {
            InputFrame {
                buttons_down: 0,
                buttons_pressed: 0,
                ..input
            }
        } else {
            input
        };
        self.host.decoders = self.assets.decoders.clone();
        for s in &mut self.sessions {
            if s.yielded == VmYield::Halted {
                continue;
            }
            s.yielded = core(s.vm.step(&mut self.host, input, 1024))
                .with_context(|| format!("VM {}", s.name))?;
        }
        for (client, path, bytes) in std::mem::take(&mut self.host.generated) {
            self.assets.register_generated(client, &path, bytes)?;
        }
        for (client, slot, w, h, rgba) in std::mem::take(&mut self.host.surfaces) {
            self.assets.put_surface(client, slot, w, h, rgba)?;
        }
        let media = self.host.media.tick(self.host.clock_ms)?;
        for update in media.frames {
            self.assets.put_surface(
                update.client,
                0x7100_0000 + update.id,
                update.frame.width,
                update.frame.height,
                update.frame.rgba,
            )?;
        }
        for update in media.audio {
            self.host
                .channel_audio
                .submit(update.client.0, update.request)?;
        }
        self.sync_videos()?;
        self.tick_puppets(delta_ms)?;
        self.host.channel_audio.render(delta_ms)?;
        core(self.host.scene.advance_frame())?;
        Ok(())
    }
    /// Loads a puppet rig from the client's assets (cached).
    pub fn puppet_rig(
        &mut self,
        client: ClientId,
        asset: &str,
    ) -> Result<Arc<crysirror_puppet::PuppetRig>> {
        let key = (client.0, asset.to_owned());
        if let Some(r) = self.puppet_rigs.get(&key) {
            return Ok(r.clone());
        }
        let bytes = self.assets.read_for(client, asset)?;
        let rig = Arc::new(
            crysirror_puppet::PuppetRig::from_json(&bytes)
                .with_context(|| format!("puppet rig {asset}"))?,
        );
        self.puppet_rigs.insert(key, rig.clone());
        Ok(rig)
    }
    /// Applies pending puppet commands, advances every puppet layer and
    /// publishes its meshes (ADR 0029). Texture paths become client asset
    /// paths relative to the rig file.
    fn tick_puppets(&mut self, delta_ms: u64) -> Result<()> {
        for (client, id, asset) in self.host.engine.layers.puppet_layers() {
            let rig = self.puppet_rig(ClientId(client), &asset)?;
            let slot = self
                .host
                .engine
                .layers
                .puppet_slot(client, id)
                .context("puppet slot")?
                .clone();
            let mut p = match slot.state {
                Some(s) => crysirror_puppet::Puppet::with_state(rig.clone(), s)?,
                None => crysirror_puppet::Puppet::new(rig.clone())?,
            };
            for c in &slot.pending {
                use crysirror_scene::layer::PuppetCommand as C;
                match c {
                    C::SetParameter {
                        parameter,
                        value,
                        duration_ms,
                    } => p.set_parameter(parameter, *value, *duration_ms),
                    C::ReleaseParameter { parameter } => p.release_parameter(parameter),
                    C::Play {
                        motion,
                        looping,
                        weight,
                    } => p.play(motion, *looping, *weight),
                    C::Stop { motion } => p.stop(motion),
                }
                .with_context(|| format!("puppet layer {id} ({asset})"))?;
            }
            p.advance(delta_ms as f32)?;
            let mut frame = p.frame();
            let dir = asset.rsplit_once('/').map_or("", |(d, _)| d);
            for t in &mut frame.textures {
                if !dir.is_empty() {
                    *t = format!("{dir}/{t}");
                }
            }
            self.host
                .engine
                .layers
                .set_puppet(client, id, p.state().clone(), Arc::new(frame))?;
        }
        Ok(())
    }
    /// Resolve a project client name to its scoped id.
    pub fn client_id(&self, name: &str) -> Result<ClientId> {
        self.sessions
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.id)
            .with_context(|| format!("unknown client '{name}'"))
    }
    /// Overlay a read-only provider in one client's asset namespace (ADR 0006).
    /// Native VMs read through HostServices, so they observe the mount without
    /// any change to their own path/archive semantics.
    pub fn mount_provider(
        &mut self,
        client: ClientId,
        mount_point: &str,
        provider: Arc<dyn crysirror_core::vfs::VfsProvider>,
    ) -> Result<()> {
        core(self.host.fs.mount_provider(client, mount_point, provider))?;
        self.assets
            .refresh_storage(client, core(self.host.fs.storage(client))?)?;
        self.physics.invalidate_client(client.0);
        Ok(())
    }
    pub fn all_halted(&self) -> bool {
        self.sessions.iter().all(|s| s.yielded == VmYield::Halted)
    }
    pub fn now_ms(&self) -> u64 {
        self.host.clock_ms
    }
    pub fn inspect(&self) -> serde_json::Value {
        serde_json::json!({"name":self.project.name,"frame":self.host.scene.frame(),"time_ms":self.host.clock_ms,
            "visuals":self.host.scene.visuals().len(),"voices":self.host.scene.voices().len(),
            "channel_audio":self.host.channel_audio.mixer.stats(),"media":self.host.media.inspect(),
            "engine":self.host.engine,
            "adapters":self.registry.names(),"sessions":self.sessions.iter().map(|s|serde_json::json!({
                "name":s.name,"client":s.id.0,"engine":s.vm.engine_id(),"yield":format!("{:?}",s.yielded),"native":serde_json::from_slice::<serde_json::Value>(&s.vm.diagnostics()).ok()
            })).collect::<Vec<_>>()})
    }
    fn identity(&self) -> SaveIdentity {
        SaveIdentity {
            engine: "crysirror".into(),
            game: self.project.name.clone(),
            content_revision: self.fingerprint.clone(),
            vm_schema: 1,
            services_schema: if self
                .project
                .clients
                .iter()
                .any(|c| c.systems.is_some() || c.adapter == "siglus")
            {
                2
            } else {
                1
            },
        }
    }
    pub fn snapshot(&self) -> Result<Vec<u8>> {
        ensure!(
            !self.host.media.has_resources(),
            "engine media snapshot requires resource reconstruction support (including retained EOF frames)"
        );
        let mut clients = Vec::new();
        let mut machines = Vec::new();
        for s in &self.sessions {
            let mut commands: Vec<Action> = self
                .host
                .scene
                .visuals()
                .iter()
                .filter(|(id, _)| id.client == s.id)
                .map(|(id, node)| Action::from_visual(id.local, node))
                .collect();
            for (id, v) in self
                .host
                .scene
                .voices()
                .iter()
                .filter(|(id, _)| id.client == s.id)
            {
                commands.push(Action::Play {
                    channel: id.local,
                    asset: v.asset.relative().into(),
                    looping: v.looping,
                    gain_milli: v.gain_milli,
                });
            }
            clients.push(ClientScene {
                client: s.id.0,
                commands,
            });
            machines.push(VmSave {
                name: s.name.clone(),
                engine: s.vm.engine_id().into(),
                schema: s.vm.snapshot_schema(),
                bytes: core(s.vm.snapshot())?,
            });
        }
        let services = serde_json::to_vec(&ServicesSave {
            clock_ms: self.host.clock_ms,
            frame: self.host.scene.frame(),
            clients,
            engine: Some(self.host.engine.clone()),
        })?;
        let vm = serde_json::to_vec(&machines)?;
        core(
            SaveEnvelope {
                identity: self.identity(),
                services,
                vm,
            }
            .encode(SaveLimits::default()),
        )
    }
    /// All VMs and service commands are reconstructed before any live state swap.
    /// Audio restarts at sample zero. GPU/audio handles are deliberately not saved.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        let envelope = core(SaveEnvelope::decode(
            bytes,
            &self.identity(),
            SaveLimits::default(),
        ))?;
        let mut services: ServicesSave = serde_json::from_slice(&envelope.services)?;
        let machines: Vec<VmSave> = serde_json::from_slice(&envelope.vm)?;
        ensure!(
            self.project
                .clients
                .iter()
                .all(|c| c.systems.is_none() && c.adapter != "siglus")
                || services.engine.is_some(),
            "save missing engine systems state"
        );
        let mut engine = services.engine.take().unwrap_or_default();
        engine.validate()?;
        engine.validate_clients(&self.sessions.iter().map(|s| s.id.0).collect())?;
        engine.world.render_scene(self.project.scene3d.as_ref())?;
        engine.frame(self.project.canvas)?;
        ensure!(
            services.clients.len() == self.sessions.len() && machines.len() == self.sessions.len(),
            "save session count"
        );
        let mut staged_scene = SceneState::default();
        let mut staged_sessions = Vec::new();
        for (((old, spec), saved), visuals) in self
            .sessions
            .iter()
            .zip(&self.project.clients)
            .zip(&machines)
            .zip(&services.clients)
        {
            ensure!(
                saved.name == old.name
                    && saved.engine == old.vm.engine_id()
                    && saved.schema == old.vm.snapshot_schema()
                    && visuals.client == old.id.0,
                "save session identity"
            );
            let mut seen_visuals = BTreeSet::new();
            let mut seen_voices = BTreeSet::new();
            let mut commands = Vec::new();
            for action in &visuals.commands {
                match action {
                    Action::Sprite { id, .. } | Action::Text { id, .. } => {
                        ensure!(seen_visuals.insert(*id), "duplicate saved visual")
                    }
                    Action::Play { channel, .. } => {
                        ensure!(seen_voices.insert(*channel), "duplicate saved voice")
                    }
                    _ => bail!("save contains a non-state operation"),
                }
                commands.push(action.command()?.context("save command")?);
            }
            // Saved state may exceed one command batch (visuals + voices).
            for chunk in commands.chunks(1024) {
                core(staged_scene.apply_batch(old.id, chunk, Limits::default()))?;
            }
            let mut vm = self.registry.create(old.id, spec)?;
            core(vm.restore(&saved.bytes))?;
            staged_sessions.push(Session {
                name: old.name.clone(),
                id: old.id,
                vm,
                yielded: VmYield::BudgetExhausted,
            });
        }
        let next_restored = self.restored.checked_add(1).context("restore counter")?;
        let previous_uploads = std::mem::take(&mut self.host.generated);
        let rebuild = (|| -> Result<()> {
            let mut host = RestoreResources(&mut self.host);
            for s in &mut staged_sessions {
                core(s.vm.restore_resources(&mut host))?;
            }
            Ok(())
        })();
        let uploads = std::mem::replace(&mut self.host.generated, previous_uploads);
        rebuild?;
        // Adding immutable content to the cache does not change live VM/scene state.
        for (client, path, bytes) in uploads {
            self.assets.register_generated(client, &path, bytes)?;
        }

        staged_scene.restore_frame(services.frame);
        self.sessions = staged_sessions;
        self.host.scene = staged_scene;
        self.host.clock_ms = services.clock_ms;
        engine.ui.reset_input_after_restore();
        self.host.engine = engine;
        self.physics.invalidate();
        self.host.trace.clear();
        self.restored = next_restored;
        Ok(())
    }
    pub fn export_original_save(&mut self, client: ClientId) -> Result<Vec<u8>> {
        let s = self
            .sessions
            .iter_mut()
            .find(|s| s.id == client)
            .context("native client not registered")?;
        core(s.vm.export_original_save())
    }
    pub fn import_original_save(&mut self, client: ClientId, bytes: &[u8]) -> Result<()> {
        let i = self
            .sessions
            .iter()
            .position(|s| s.id == client)
            .context("native client not registered")?;
        if self.sessions[i].vm.original_save_import_is_transactional() {
            core(self.sessions[i].vm.import_original_save(bytes))?;
            self.sessions[i].yielded = VmYield::BudgetExhausted;
            return Ok(());
        }
        let mut vm = self.registry.create(client, &self.project.clients[i])?;
        core(vm.restore(&core(self.sessions[i].vm.snapshot())?))?;
        core(vm.import_original_save(bytes))?;
        self.sessions[i].vm = vm;
        self.sessions[i].yielded = VmYield::BudgetExhausted;
        Ok(())
    }
    /// This frame's screen transition (ADR 0022). Pattern wipes get the
    /// original siglus_rs generated mask at the canvas size, built once per
    /// transition into the owner's surface slot `TRANSITION_MASK_SLOT`.
    pub fn render_transition(&mut self) -> Result<Option<crysirror_scene::layer::TransitionFrame>> {
        let Some(mut t) = self.host.engine.layers.transition_frame() else {
            return Ok(None);
        };
        if t.generated_mask {
            if self.transition_mask != Some(t.serial) {
                let [w, h] = self.project.canvas;
                let gray = siglus_scene_vm::runtime::wipe_mask::generate(
                    t.code,
                    &t.options,
                    w,
                    h,
                    t.options[15] as u32,
                )
                .with_context(|| {
                    format!(
                        "Siglus wipe type {} has no generated mask; pass a mask asset",
                        t.code
                    )
                })?;
                let rgba = gray.pixels.iter().flat_map(|&v| [v, v, v, 255]).collect();
                self.assets.put_surface(
                    ClientId(t.owner),
                    TRANSITION_MASK_SLOT,
                    gray.width,
                    gray.height,
                    rgba,
                )?;
                self.transition_mask = Some(t.serial);
            }
            t.mask = Some(format!(
                "{}{TRANSITION_MASK_SLOT}",
                crate::assets::SURFACE_PREFIX
            ));
        }
        Ok(Some(t))
    }
    /// PBR extraction (ADR 0010); `None` keeps the legacy raster path.
    pub fn render_pbr(&self) -> Result<Option<crysirror_scene::view::PbrScene>> {
        self.host
            .engine
            .world
            .pbr_scene(self.project.scene3d.as_ref())
    }
    pub fn render_scene3d(&self) -> Result<Option<crate::model::Scene3d>> {
        self.host
            .engine
            .world
            .render_scene(self.project.scene3d.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime() -> (tempfile::TempDir, Runtime) {
        let tmp = tempfile::tempdir().unwrap();
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/project.json");
        let r = Runtime::load(&p, tmp.path()).unwrap();
        (tmp, r)
    }
    #[test]
    fn two_clients_share_services_but_not_ids() {
        let (_t, mut r) = runtime();
        r.tick(0, InputFrame::default()).unwrap();
        assert!(
            r.host
                .scene
                .visuals()
                .keys()
                .any(|id| id.client == ClientId(1) && id.local == 0)
        );
        assert!(
            r.host
                .scene
                .visuals()
                .keys()
                .any(|id| id.client == ClientId(2) && id.local == 0)
        );
    }
    #[test]
    fn roundtrip_rewinds_runtime() {
        let (_t, mut r) = runtime();
        r.tick(0, InputFrame::default()).unwrap();
        let before = r.host.scene.clone();
        let save = r.snapshot().unwrap();
        r.tick(
            5000,
            InputFrame {
                buttons_pressed: 1,
                ..InputFrame::default()
            },
        )
        .unwrap();
        r.restore(&save).unwrap();
        assert_eq!(r.host.scene.visuals(), before.visuals());
        assert_eq!(r.now_ms(), 0);
    }
    #[test]
    fn corrupt_save_does_not_mutate_live_state() {
        let (_t, mut r) = runtime();
        r.tick(0, InputFrame::default()).unwrap();
        let before = r.snapshot().unwrap();
        let mut bad = before.clone();
        bad[30] ^= 1;
        assert!(r.restore(&bad).is_err());
        assert_eq!(r.snapshot().unwrap(), before);
    }
    #[test]
    fn invalid_vm_payload_is_transactional() {
        let (_t, mut r) = runtime();
        r.tick(0, InputFrame::default()).unwrap();
        let before = r.snapshot().unwrap();
        let mut e = core(SaveEnvelope::decode(
            &before,
            &r.identity(),
            SaveLimits::default(),
        ))
        .unwrap();
        let mut v: Vec<VmSave> = serde_json::from_slice(&e.vm).unwrap();
        v[1].bytes = b"not a VM snapshot".to_vec();
        e.vm = serde_json::to_vec(&v).unwrap();
        let bad = core(e.encode(SaveLimits::default())).unwrap();
        assert!(r.restore(&bad).is_err());
        assert_eq!(r.snapshot().unwrap(), before);
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;
    fn systems() -> (tempfile::TempDir, Runtime) {
        let t = tempfile::tempdir().unwrap();
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/systems.json");
        let rt = Runtime::load(&path, t.path()).unwrap();
        (t, rt)
    }
    #[test]
    fn systems_keep_running_after_data_vm_halt_and_restore_exactly() {
        let (_t, mut r) = systems();
        r.tick(0, InputFrame::default()).unwrap();
        assert!(r.all_halted());
        let initial = r.render_scene3d().unwrap().unwrap();
        r.tick(1000, InputFrame::default()).unwrap();
        assert_ne!(
            serde_json::to_value(initial).unwrap(),
            serde_json::to_value(r.render_scene3d().unwrap()).unwrap()
        );
        let save = r.snapshot().unwrap();
        let before = serde_json::to_value(&r.host.engine).unwrap();
        r.tick(1000, InputFrame::default()).unwrap();
        r.restore(&save).unwrap();
        assert_eq!(serde_json::to_value(&r.host.engine).unwrap(), before);
        assert_eq!(r.now_ms(), 1000);
        let mut other = systems().1;
        other.restore(&save).unwrap();
        r.tick(31, InputFrame::default()).unwrap();
        other.tick(31, InputFrame::default()).unwrap();
        assert_eq!(
            serde_json::to_value(&r.host.engine).unwrap(),
            serde_json::to_value(&other.host.engine).unwrap()
        );
    }
    #[test]
    fn focused_ui_consumes_enter_and_button_pauses_only_its_world() {
        let (_t, mut r) = systems();
        r.project.clients[0].program = vec![
            Action::WaitInput,
            Action::Sprite {
                id: 99,
                asset: "panel.png".into(),
                rect: [0, 0, 5, 5],
                z: 0,
                tint: [255; 4],
            },
            Action::Halt,
        ];
        r.sessions[0].vm =
            Box::new(ScriptVm::new(ClientId(1), r.project.clients[0].program.clone()).unwrap());
        r.tick(0, InputFrame::default()).unwrap();
        r.tick(
            16,
            InputFrame {
                buttons_pressed: 1,
                ..InputFrame::default()
            },
        )
        .unwrap();
        assert!(
            r.host.scene.visuals().is_empty(),
            "UI click must not resume background VM"
        );
        let h = r.host.engine.world.handle(ClientId(1), 1).unwrap();
        let t = r.host.engine.world.entity(h).unwrap().transform.clone();
        r.tick(160, InputFrame::default()).unwrap();
        assert_eq!(
            r.host.engine.world.entity(h).unwrap().transform.translation,
            t.translation
        );
        assert_eq!(
            r.host
                .engine
                .world
                .entity(h)
                .unwrap()
                .transform
                .rotation_degrees,
            t.rotation_degrees
        );
        r.tick(
            0,
            InputFrame {
                buttons_pressed: 64,
                ..InputFrame::default()
            },
        )
        .unwrap();
        r.tick(
            16,
            InputFrame {
                buttons_pressed: 1,
                ..InputFrame::default()
            },
        )
        .unwrap();
        assert_ne!(
            r.host
                .engine
                .world
                .entity(h)
                .unwrap()
                .transform
                .rotation_degrees,
            t.rotation_degrees
        );
    }
    #[test]
    fn invalid_engine_save_is_transactional() {
        let (_t, mut r) = systems();
        r.tick(16, InputFrame::default()).unwrap();
        let before = r.snapshot().unwrap();
        let mut envelope = core(SaveEnvelope::decode(
            &before,
            &r.identity(),
            SaveLimits::default(),
        ))
        .unwrap();
        let mut services: serde_json::Value = serde_json::from_slice(&envelope.services).unwrap();
        let clients = services["engine"]["world"]["clients"]
            .as_object_mut()
            .unwrap();
        let foreign = clients.remove("1").unwrap();
        clients.insert("900".into(), foreign);
        envelope.services = serde_json::to_vec(&services).unwrap();
        let bad = core(envelope.encode(SaveLimits::default())).unwrap();
        assert!(r.restore(&bad).is_err());
        assert_eq!(r.snapshot().unwrap(), before);
    }
}
#[cfg(test)]
mod required_engine_save_test {
    use super::*;
    #[test]
    fn modern_save_must_not_silently_drop_engine_state() {
        let temp = tempfile::tempdir().unwrap();
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/systems.json");
        let mut r = Runtime::load(&path, temp.path()).unwrap();
        let before = r.snapshot().unwrap();
        let mut envelope = core(SaveEnvelope::decode(
            &before,
            &r.identity(),
            SaveLimits::default(),
        ))
        .unwrap();
        let mut data: serde_json::Value = serde_json::from_slice(&envelope.services).unwrap();
        data.as_object_mut().unwrap().remove("engine");
        envelope.services = serde_json::to_vec(&data).unwrap();
        let bad = core(envelope.encode(SaveLimits::default())).unwrap();
        assert!(
            r.restore(&bad)
                .unwrap_err()
                .to_string()
                .contains("missing engine")
        );
        assert_eq!(r.snapshot().unwrap(), before);
    }
}

#[cfg(test)]
mod siglus_native_tests {
    use super::*;
    #[test]
    fn original_siglus_bytecode_calls_shared_layers_and_roundtrips_native_wait() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("assets")).unwrap();
        std::fs::write(
            tmp.path().join("assets/scene.scn"),
            crysirror_siglus_vm::fixture::demo_scene(),
        )
        .unwrap();
        let data = serde_json::json!({"schema":2,"name":"native-siglus-fixture","content_revision":"1","canvas":[320,180],"assets":"assets","clients":[{"name":"siglus","adapter":"siglus","game":"fixture","config":{"scene":"scene.scn","format":"scn","string_codec":"plain"}}]});
        let path = tmp.path().join("project.json");
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        let mut rt = Runtime::load(&path, &tmp.path().join("state")).unwrap();
        assert!(rt.registry.names().contains(&"siglus"));
        rt.tick(0, InputFrame::default()).unwrap();
        let info = rt.inspect();
        assert_eq!(
            info["sessions"][0]["native"]["int_stack"],
            serde_json::json!([42])
        );
        let frame = rt.host.engine.frame([320, 180]).unwrap();
        assert_eq!(frame.len(), 1);
        assert_eq!(frame[0].quad[0][0], 28.);
        let save = rt.snapshot().unwrap();
        rt.tick(50, InputFrame::default()).unwrap();
        assert!(!rt.all_halted());
        rt.tick(50, InputFrame::default()).unwrap();
        assert!(rt.all_halted());
        assert_eq!(
            rt.host.engine.frame([320, 180]).unwrap()[0].quad[0][0],
            180.
        );
        rt.restore(&save).unwrap();
        assert_eq!(rt.now_ms(), 0);
        rt.tick(99, InputFrame::default()).unwrap();
        assert!(!rt.all_halted());
        assert_eq!(rt.host.engine.frame([320, 180]).unwrap()[0].quad[0][0], 28.);
        rt.tick(1, InputFrame::default()).unwrap();
        assert!(rt.all_halted());
        assert_eq!(
            rt.host.engine.frame([320, 180]).unwrap()[0].quad[0][0],
            180.
        );
    }
}
#[cfg(test)]
mod provider_tests {
    use super::*;
    #[test]
    fn siglus_reads_its_scene_through_a_mounted_provider() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("assets")).unwrap();
        let data = serde_json::json!({"schema":2,"name":"provider-siglus","content_revision":"1","canvas":[320,180],"assets":"assets","clients":[{"name":"siglus","adapter":"siglus","game":"fixture","config":{"scene":"pack/scene.scn","format":"scn","string_codec":"plain"}}]});
        let path = tmp.path().join("project.json");
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        let mut rt = Runtime::load(&path, &tmp.path().join("state")).unwrap();
        let id = rt.client_id("siglus").unwrap();
        let mut m = crysirror_core::vfs::MemoryProvider::new();
        m.insert("scene.scn", crysirror_siglus_vm::fixture::demo_scene())
            .unwrap();
        rt.mount_provider(id, "pack", Arc::new(m)).unwrap();
        assert!(!tmp.path().join("assets/pack").exists());
        rt.tick(0, InputFrame::default()).unwrap();
        assert_eq!(
            rt.inspect()["sessions"][0]["native"]["int_stack"],
            serde_json::json!([42])
        );
        assert_eq!(rt.host.engine.frame([320, 180]).unwrap().len(), 1);
        assert!(rt.client_id("nobody").is_err());
    }
}
#[cfg(test)]
mod siglus_pack_tests {
    use super::*;
    use crysirror_siglus_vm::{
        fixture,
        runtime::constants::{cd, fm},
    };
    fn runtime(scene: Vec<u8>, format: &str) -> (tempfile::TempDir, Runtime) {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("assets")).unwrap();
        std::fs::write(t.path().join("assets/scene.bin"), scene).unwrap();
        std::fs::write(t.path().join("assets/native.g00"), fixture::pixel_g00()).unwrap();
        let p = serde_json::json!({"schema":2,"name":"siglus-native","content_revision":"1","canvas":[320,180],"assets":"assets","clients":[{"name":"siglus","adapter":"siglus","game":"native","config":{"scene":"scene.bin","format":format,"string_codec":"plain"}}]});
        let path = t.path().join("project.json");
        std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
        let rt = Runtime::load(&path, &t.path().join("state")).unwrap();
        (t, rt)
    }
    #[test]
    fn native_pck_loads_original_scene_metadata_and_g00_through_scoped_host() {
        let (_t, mut r) = runtime(fixture::pack_scene(&fixture::image_scene()), "pck");
        r.tick(0, InputFrame::default()).unwrap();
        let frame = r.host.engine.frame([320, 180]).unwrap();
        assert_eq!(frame.len(), 1);
        if let crysirror_scene::layer::Paint::Image { asset, .. } = &frame[0].paint {
            let bytes = r.assets.read_for(ClientId(1), asset).unwrap();
            assert!(bytes.starts_with(b"\x89PNG"));
        } else {
            panic!("native G00 must reach common image service")
        };
        let save = r.snapshot().unwrap();
        r.tick(100, InputFrame::default()).unwrap();
        assert!(r.all_halted());
        r.restore(&save).unwrap();
        r.tick(1, InputFrame::default()).unwrap();
        assert!(!r.all_halted());
    }
    #[test]
    fn fresh_native_restore_rebuilds_images_without_advancing_pc_or_wait() {
        let (tmp, mut r) = runtime(fixture::pack_scene(&fixture::image_scene()), "pck");
        r.tick(0, InputFrame::default()).unwrap();
        let expected = r.inspect()["sessions"][0]["native"].clone();
        let save = r.snapshot().unwrap();
        let mut fresh = Runtime::load(
            &tmp.path().join("project.json"),
            &tmp.path().join("fresh-state"),
        )
        .unwrap();
        fresh.restore(&save).unwrap();
        assert_eq!(fresh.inspect()["sessions"][0]["native"], expected);
        let frame = fresh.host.engine.frame(fresh.project.canvas).unwrap();
        let crysirror_scene::layer::Paint::Image { asset, .. } = &frame[0].paint else {
            panic!("native image expected")
        };
        let png = fresh
            .assets
            .read_for(ClientId(frame[0].owner), asset)
            .unwrap();
        let image = crysirror_image::decode_standard(&png).unwrap();
        assert_eq!(image.rgba, vec![20, 200, 10, 255]);
    }
    #[test]
    fn native_gosub_return_unicode_and_branch_execute_original_instruction_paths() {
        let mut b = fixture::SceneBuilder::default();
        b.code.push(cd::GOSUB);
        b.i32(0);
        b.i32(0);
        b.string("原始 VM 🦀");
        b.int(0);
        b.code.push(cd::GOTO_FALSE);
        b.i32(1);
        b.int(-999);
        b.labels.push(b.code.len() as i32);
        b.int(42);
        b.code.push(cd::RETURN);
        b.i32(1);
        b.i32(fm::INT);
        b.labels.push(b.code.len() as i32);
        b.code.push(cd::EOF);
        let (_t, mut r) = runtime(b.finish(), "scn");
        r.tick(0, InputFrame::default()).unwrap();
        assert!(r.all_halted());
        let d = r.inspect();
        assert_eq!(
            d["sessions"][0]["native"]["int_stack"],
            serde_json::json!([42])
        );
        assert_eq!(
            d["sessions"][0]["native"]["str_stack"],
            serde_json::json!(["原始 VM 🦀"])
        );
    }
    #[test]
    fn unknown_native_opcode_never_reports_success() {
        let mut b = fixture::SceneBuilder::default();
        b.code.push(0xfe);
        let (_t, mut r) = runtime(b.finish(), "scn");
        assert!(r.tick(0, InputFrame::default()).is_err());
        assert!(r.host.engine.frame([320, 180]).unwrap().is_empty());
    }
    #[test]
    fn bad_native_header_rejected_without_panic() {
        let (_t, mut r) = runtime(vec![0; 4], "pck");
        assert!(r.tick(0, InputFrame::default()).is_err());
    }
    #[test]
    fn independent_siglus_clients_share_layers_without_aliasing_ids() {
        let (t, mut r) = runtime(fixture::demo_scene(), "scn");
        let mut p = r.project.clone();
        let mut c = p.clients[0].clone();
        c.name = "other".into();
        p.clients.push(c);
        let path = t.path().join("project.json");
        std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
        r = Runtime::load(&path, &t.path().join("state2")).unwrap();
        r.tick(0, InputFrame::default()).unwrap();
        let f = r.host.engine.frame([320, 180]).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].id, f[1].id);
        assert_ne!(f[0].owner, f[1].owner);
    }
}
