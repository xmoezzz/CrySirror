// SPDX-License-Identifier: MPL-2.0
//! Serialized AnimatorData from the standalone Unity 5.6 dance scenes.
use super::*;
use crate::unity_scene::{ObjectKey, UnityData};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_core::service::{
    AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, PcmBus,
};
use crysirror_unity::class_id as c;
use glam::{Quat, Vec3};

const CHANNEL: u32 = 24;
const MAX_KEYS: usize = 32_768;

#[derive(Clone)]
struct Key<T> {
    frame: u32,
    ease: i32,
    curve: Vec<[f32; 4]>,
    value: T,
}

fn phase(ease: i32, curve: &[[f32; 4]], t: f32) -> f32 {
    match ease {
        0 => t.powi(2),
        1 => 1. - (1. - t).powi(2),
        2 if t < 0.5 => 2. * t.powi(2),
        2 => 1. - 2. * (1. - t).powi(2),
        32 if !curve.is_empty() => {
            let right = curve.partition_point(|key| key[0] <= t);
            if right == 0 {
                return curve[0][1];
            }
            let [start, a, _, outgoing] = curve[right - 1];
            let Some(&[end, b, incoming, _]) = curve.get(right) else {
                return a;
            };
            if !outgoing.is_finite() || !incoming.is_finite() {
                return a;
            }
            let duration = end - start;
            let x = (t - start) / duration;
            let x2 = x * x;
            let x3 = x2 * x;
            (2. * x3 - 3. * x2 + 1.) * a
                + (x3 - 2. * x2 + x) * outgoing * duration
                + (-2. * x3 + 3. * x2) * b
                + (x3 - x2) * incoming * duration
        }
        33 => 0., // AMTween.stayStart
        34 => 1., // AMTween.stayEnd
        _ => t,
    }
}

fn sample<T: Copy>(keys: &[Key<T>], frame: f32, interpolate: impl Fn(T, T, f32) -> T) -> Option<T> {
    let right = keys.partition_point(|key| key.frame as f32 <= frame);
    if right == 0 {
        return keys.first().map(|key| key.value);
    }
    let left = &keys[right - 1];
    let Some(right) = keys.get(right) else {
        return Some(left.value);
    };
    let t = (frame - left.frame as f32) / (right.frame - left.frame) as f32;
    Some(interpolate(
        left.value,
        right.value,
        phase(left.ease, &left.curve, t),
    ))
}

struct Path {
    end: u32,
    waypoints: Vec<Vec3>,
}

fn camera_position(paths: &[Key<Path>], frame: f32) -> Option<Vec3> {
    let i = paths
        .partition_point(|key| key.frame as f32 <= frame)
        .saturating_sub(1);
    let path = paths.get(i)?;
    let points = &path.value.waypoints;
    if points.len() == 1 || frame <= path.frame as f32 {
        return points.first().copied();
    }
    let t = ((frame - path.frame as f32) / path.value.end.saturating_sub(path.frame).max(1) as f32)
        .min(1.);
    let t = phase(path.ease, &path.curve, t).clamp(0., 1.);
    if points.len() == 2 {
        return Some(points[0].lerp(points[1], t));
    }
    let last = points.len() - 1;
    let pos = t * last as f32;
    let j = (pos as usize).min(last - 1);
    let x = pos - j as f32;
    let b = points[j];
    let c = points[j + 1];
    let closed = points[0] == points[last];
    let a = if j > 0 {
        points[j - 1]
    } else if closed {
        points[last - 1]
    } else {
        2. * b - c
    };
    let d = if j + 2 <= last {
        points[j + 2]
    } else if closed {
        points[1]
    } else {
        2. * c - b
    };
    Some(
        0.5 * ((-a + 3. * b - 3. * c + d) * x.powi(3)
            + (2. * a - 5. * b + 4. * c - d) * x.powi(2)
            + (-a + c) * x
            + 2. * b),
    )
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[derive(Clone)]
enum Event {
    Face(usize, String, bool),
    Eye(usize, String),
    Visible(usize, bool),
    Fade(f32, bool),
}

struct Timeline {
    fps: u32,
    frames: u32,
    audio: String,
    audio_frame: u32,
    clips: Vec<(String, u32)>,
    lips: Vec<String>,
    translation: Vec<Key<Path>>,
    rotation: Vec<Key<Quat>>,
    fov: Vec<Key<f32>>,
    far: Vec<Key<f32>>,
    light: Vec<Key<f32>>,
    light_color: Vec<Key<Vec3>>,
    cues: Vec<(u32, Event)>,
    other: std::collections::BTreeSet<String>,
}

fn body(data: &[u8]) -> Result<Reader<'_>> {
    let mut r = Reader::new(data);
    r.seek(28)?;
    r.string()?;
    Ok(r)
}

fn references(r: &mut Reader<'_>) -> Result<Vec<i64>> {
    let n = r.count(12)?;
    ensure!(n <= MAX_KEYS, "dance reference count");
    (0..n)
        .map(|_| {
            let ptr = r.pptr64()?;
            ensure!(ptr.file_id == 0, "unexpected external dance track");
            Ok(ptr.path_id)
        })
        .collect()
}

fn skip_references(r: &mut Reader<'_>) -> Result<()> {
    let count = r.count(12)?;
    r.take(count * 12)?;
    Ok(())
}
fn strings(r: &mut Reader<'_>) -> Result<Vec<String>> {
    let n = r.count(4)?;
    ensure!(n <= 128, "dance string list length");
    (0..n).map(|_| r.string()).collect()
}

fn class(
    unity: &mut UnityData,
    op: &mut Opener,
    file: &str,
    bytes: &[u8],
) -> Result<Option<String>> {
    let mut r = Reader::new(bytes);
    r.seek(16)?;
    let Some(script) = unity.resolve(op, file, r.pptr64()?)? else {
        return Ok(None);
    };
    let (_, bytes) = unity.bytes(op, &script)?;
    Ok(Some(Reader::new(&bytes).string()?))
}

fn keys<T>(
    file: &crysirror_unity::SerializedFile,
    refs: &[i64],
    value: impl Fn(&mut Reader<'_>) -> Result<T>,
) -> Result<Vec<Key<T>>> {
    let mut result = Vec::with_capacity(refs.len());
    for &id in refs {
        let data = file.bytes_of(id)?;
        let mut r = body(&data)?;
        let frame = r.u32()?;
        let ease = r.i32()?;
        let n = r.count(4)?;
        ensure!(n % 4 == 0, "invalid dance easing curve length");
        let curve = (0..n / 4)
            .map(|_| Ok(r.f32s()?))
            .collect::<Result<Vec<_>>>()?;
        result.push(Key {
            frame,
            ease,
            curve,
            value: value(&mut r)?,
        });
    }
    result.sort_by_key(|key| key.frame);
    Ok(result)
}

fn event_arg(file: &crysirror_unity::SerializedFile, id: i64) -> Result<(bool, i32, f32, String)> {
    let data = file.bytes_of(id)?;
    let mut r = body(&data)?;
    let boolean = r.i32()? != 0;
    r.i32()?; // AMEventParameter.ValueType.
    let integer = r.i32()?;
    let float = r.f32()?;
    r.take(68)?;
    Ok((boolean, integer, float, r.string()?))
}

impl Timeline {
    fn load(unity: &mut UnityData, op: &mut Opener, path: &str) -> Result<Self> {
        let file = unity.file(op, path)?;
        let mut source = None;
        for item in file
            .objects
            .iter()
            .filter(|o| o.class_id == c::MONO_BEHAVIOUR)
        {
            let bytes = file.bytes(item)?;
            if class(unity, op, path, &bytes)?.as_deref() == Some("DanceMain") {
                source = Some(bytes);
                break;
            }
        }
        let source = source.context("dance scene is missing DanceMain")?;
        let mut r = body(&source)?;
        r.take(16)?; // dummy-body flag and Camera pointer.
        skip_references(&mut r)?; // debug objects.
        let animator = r.pptr64()?;
        ensure!(animator.file_id == 0, "external dance animator");
        skip_references(&mut r)?; // dummy body references.
        let motions = strings(&mut r)?;
        skip_references(&mut r)?; // event objects can refer to shared assets.
        let presets = strings(&mut r)?;
        skip_references(&mut r)?; // prop objects.
        let lips = strings(&mut r)?;
        let audio_source = r.string()?;
        let audio = r.string()?;
        let take_name = r.string()?;
        let flags = r.take(4)?;
        ensure!(
            flags[2] == 0,
            "external DanceMain binary timelines require a separate profile"
        );
        ensure!(
            !motions.is_empty()
                && motions.len() == lips.len()
                && motions.len() == presets.len()
                && motions.len() <= 18,
            "invalid dance cast metadata"
        );
        let animator_data = file.bytes_of(animator.path_id)?;
        let take_refs = references(&mut body(&animator_data)?)?;
        let mut take = None;
        for id in take_refs {
            let bytes = file.bytes_of(id)?;
            if body(&bytes)?.string()? == take_name {
                take = Some(bytes);
                break;
            }
        }
        let take = take.context("DanceMain take was not serialized")?;
        let mut r = body(&take)?;
        r.string()?;
        let fps = r.u32()?;
        let frames = r.u32()?;
        ensure!(
            (1..=240).contains(&fps) && (1..=1_000_000).contains(&frames),
            "invalid dance take clock"
        );
        r.take(24)?; // editor range, playback speed and track/frame/group selection.
        let n = r.count(4)?;
        r.take(n * 4)?; // track ids correspond to the object list.
        let tracks = references(&mut r)?;
        let mut timeline = Self {
            fps,
            frames,
            audio,
            audio_frame: 0,
            clips: motions
                .into_iter()
                .map(|name| (format!("{name}.anm"), 0))
                .collect(),
            lips,
            translation: Vec::new(),
            rotation: Vec::new(),
            fov: Vec::new(),
            far: Vec::new(),
            light: Vec::new(),
            light_color: Vec::new(),
            cues: Vec::new(),
            other: Default::default(),
        };
        let mut has_audio = false;
        for id in tracks {
            let bytes = file.bytes_of(id)?;
            let kind = class(unity, op, path, &bytes)?.context("dance track has no class")?;
            let mut r = body(&bytes)?;
            r.i32()?;
            let name = r.string()?;
            let key_refs = references(&mut r)?;
            let actions = references(&mut r)?;
            r.take(16)?; // foldout and parent take.
            if key_refs.is_empty() {
                continue;
            }
            match kind.as_str() {
                "AMAudioTrack" => {
                    r.pptr64()?;
                    if r.string()? == audio_source {
                        ensure!(
                            !has_audio && key_refs.len() == 1,
                            "unexpected dance master audio layout"
                        );
                        has_audio = true;
                        timeline.audio_frame = keys(&file, &key_refs, |_| Ok(()))?[0].frame;
                    }
                }
                "AMAnimationTrack" => {
                    if let Some(slot) = name
                        .strip_prefix('f')
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|slot| slot.checked_sub(1))
                        .and_then(|slot| timeline.clips.get_mut(slot))
                    {
                        ensure!(key_refs.len() == 1, "unexpected character dance clip count");
                        slot.1 = keys(&file, &key_refs, |_| Ok(()))?[0].frame;
                    } else {
                        timeline.other.insert(format!("{kind}:{name}"));
                    }
                }
                "AMTranslationTrack" | "AMRotationTrack" => {
                    r.pptr64()?;
                    let target = r.string()?;
                    if target != "CameraMain" {
                        timeline.other.insert(format!("{kind}:{target}"));
                        continue;
                    }
                    if kind == "AMTranslationTrack" {
                        timeline.translation = keys(&file, &actions, |r| {
                            let end = r.u32()?;
                            r.pptr64()?;
                            let count = r.count(12)?;
                            ensure!((1..=MAX_KEYS).contains(&count), "invalid dance path");
                            let mut waypoints = Vec::with_capacity(count);
                            for _ in 0..count {
                                waypoints.push(Vec3::from_array(r.f32s()?));
                            }
                            Ok(Path { end, waypoints })
                        })?;
                    } else {
                        timeline.rotation = keys(&file, &key_refs, |r| {
                            Ok(Quat::from_array(r.f32s()?).normalize())
                        })?;
                    }
                }
                "AMPropertyTrack" => {
                    r.i32()?;
                    r.pptr64()?;
                    let target = r.string()?;
                    r.pptr64()?;
                    let component = r.string()?;
                    let property = r.string()?;
                    if target == "MainDirectionalLight"
                        && component == "Light"
                        && property == "color"
                    {
                        timeline.light_color = keys(&file, &key_refs, |r| {
                            r.take(28)?;
                            let [red, green, blue, _] = r.f32s::<4>()?;
                            Ok(Vec3::new(red, green, blue))
                        })?;
                        continue;
                    }
                    let dest = match (target.as_str(), component.as_str(), property.as_str()) {
                        ("CameraMain", "Camera", "fov" | "fieldOfView") => &mut timeline.fov,
                        ("CameraMain", "Camera", "farClipPlane") => &mut timeline.far,
                        ("MainDirectionalLight", "Light", "intensity") => &mut timeline.light,
                        _ => {
                            timeline.other.insert(format!("{kind}:{target}:{name}"));
                            continue;
                        }
                    };
                    *dest = keys(&file, &key_refs, |r| {
                        Ok(f64::from_le_bytes(r.take(8)?.try_into()?) as f32)
                    })?;
                }
                "AMEventTrack" => {
                    for key in keys(&file, &key_refs, |r| {
                        r.take(28)?;
                        let args = references(r)?;
                        Ok((r.string()?, args))
                    })? {
                        let (method, args) = key.value;
                        let event = match (method.as_str(), args.as_slice()) {
                            ("SetFace" | "SetFaceBlend" | "EyeToCam", [slot, text]) => {
                                let slot = usize::try_from(event_arg(&file, *slot)?.1)?;
                                let text = event_arg(&file, *text)?.3;
                                if method == "EyeToCam" {
                                    Event::Eye(slot, text)
                                } else {
                                    Event::Face(slot, text, method == "SetFaceBlend")
                                }
                            }
                            ("SetMaidVisible", [slot, visible]) => Event::Visible(
                                usize::try_from(event_arg(&file, *slot)?.1)?,
                                event_arg(&file, *visible)?.0,
                            ),
                            ("FadeIn" | "FadeOut", [seconds]) => {
                                Event::Fade(event_arg(&file, *seconds)?.2, method == "FadeOut")
                            }
                            _ => {
                                timeline.other.insert(format!("{kind}:{method}"));
                                continue;
                            }
                        };
                        timeline.cues.push((key.frame, event));
                    }
                }
                _ => {
                    timeline.other.insert(format!("{kind}:{name}"));
                }
            }
        }
        timeline.cues.sort_by_key(|(frame, _)| *frame);
        ensure!(
            has_audio && !timeline.translation.is_empty() && !timeline.rotation.is_empty(),
            "dance take is missing required master-audio or camera tracks"
        );
        Ok(timeline)
    }

    fn millis(&self, frame: u32) -> u64 {
        u64::from(frame.saturating_sub(1)) * 1000 / u64::from(self.fps)
    }
}

pub(in crate::com3d2) struct Show {
    timeline: Timeline,
    scene: String,
    start: u64,
    elapsed: u64,
    audio_ticket: Option<u64>,
    audio_playable: bool,
    audio_ready: bool,
    eof: Option<(u64, u64)>,
    cue: usize,
    started_clips: Vec<bool>,
    cast: Vec<usize>,
    music: VfsPath,
    opacity: (u64, f32, f32, u64),
}

impl Show {
    pub(in crate::com3d2) fn diagnostics(&self) -> serde_json::Value {
        json!({"scene": self.scene, "elapsed_ms": self.elapsed,
            "audio_ready": self.audio_ready, "unmapped_tracks": self.timeline.other})
    }
}

fn read_lips(unity: &mut UnityData, op: &mut Opener, name: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    let path = format!("SceneDance/{name}");
    let key: ObjectKey = unity
        .resource_of(op, &path, c::TEXT_ASSET)?
        .with_context(|| format!("dance lip pattern {path}"))?;
    let (_, data) = unity.bytes(op, &key)?;
    let (_, data) = crysirror_unity::classes::text_asset(&data)?;
    let data = data
        .into_iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    Ok(base64::engine::general_purpose::STANDARD.decode(data)?)
}

impl Game {
    pub(in crate::com3d2) fn open_dance_performance(
        &mut self,
        scene: &str,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<()> {
        let level = self.assets.scene_file(scene)?;
        let mut timeline = Timeline::load(&mut self.assets.unity, op, &level)
            .with_context(|| format!("DanceMain {scene}"))?;
        let cast = {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            let songs = s.retail.table("dance_setting.nei")?;
            let song = (1..songs.rows).find(|&row| songs.string(5, row) == scene);
            if let Some(row) = song
                && songs.exists(8, row)
            {
                timeline.audio = crate::sound::ogg(&songs.string(8, row));
            }
            s.active_maids
                .iter()
                .take(timeline.clips.len())
                .copied()
                .enumerate()
                .map(|(slot, id)| {
                    id.with_context(|| format!("DanceMain cast slot {slot} is empty"))
                })
                .collect::<Result<Vec<_>>>()?
        };
        let path = self
            .files
            .path(&timeline.audio)
            .with_context(|| format!("dance audio {}", timeline.audio))?;
        let music = VfsPath::new(Mount::Asset, path)?;
        let stage = club::model(&mut self.assets.unity, op, &level, "LiveGimic")?;
        self.enter(Scene::DancePerformance, op, host, client, now)?;
        self.sound.stop(host, client, Kind::Bgm, 0.)?;
        self.sound.stop(host, client, Kind::Voice, 0.)?;
        self.sound
            .set_dance_volume(host, client, self.sound.dance_volume)?;
        self.replace_leisure_model("LiveGimic", stage, host, client)?;
        self.actors.reset_positions();
        self.camera_control = false;
        self.main_light = MainLight::default();
        self.adv_cover = None;
        self.world_cover = None;
        self.message_open = false;
        for (slot, &id) in cast.iter().enumerate() {
            self.actors
                .activate_stock(&self.script, id, slot, Some(true))?;
            self.actors.apply(
                crate::kag::AdvUi::AllProcProp { man: false, slot },
                &self.script,
                &self.files,
                &mut self.assets,
                op,
                now,
            )?;
            let actor = self.actors.actor(false, slot).context("dance performer")?;
            actor.anims.clear();
            actor.performance_time = Some(0);
            actor.face.kuchipaku = Some((
                0.,
                read_lips(&mut self.assets.unity, op, &timeline.lips[slot])?,
            ));
        }
        let audio_playable = host.audio_codec_supported(client, "vorbis")?;
        let clips = timeline.clips.len();
        self.performance = Some(Show {
            timeline,
            scene: scene.to_owned(),
            start: now,
            elapsed: 0,
            audio_ticket: None,
            audio_playable,
            audio_ready: false,
            eof: None,
            cue: 0,
            started_clips: vec![false; clips],
            cast,
            music,
            opacity: (0, 1., 1., 0),
        });
        self.update_dance_performance(op, host, client, now)
    }

    pub(in crate::com3d2) fn close_dance_performance(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<()> {
        if let Some(show) = self.performance.take() {
            if let Some(ticket) = show.audio_ticket {
                host.release_audio(client, ticket)?;
            }
            host.submit_audio(
                client,
                AudioRequest::Stop {
                    channel: CHANNEL,
                    fade_ms: 0.,
                },
            )?;
            for slot in 0..show.cast.len() {
                if let Some(actor) = self.actors.actor(false, slot) {
                    actor.anims.clear();
                    actor.performance_time = None;
                    actor.performance_audio = None;
                    actor.face.kuchipaku = None;
                    actor.face.lip_on = false;
                    actor.visible = false;
                }
            }
            self.actors.reset_positions();
            self.main_light = MainLight::default();
            self.camera_control = true;
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let state = &mut vm.runtime.host_mut().state;
            state.active_maids.fill(None);
            if let Some(slot) = state.active_maids.first_mut() {
                *slot = show.cast.first().copied();
            }
        }
        Ok(())
    }

    pub(in crate::com3d2) fn update_dance_performance(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<()> {
        use crate::kag::AdvUi;
        let show = self
            .performance
            .as_mut()
            .context("missing DanceMain state")?;
        let clock = now.saturating_sub(show.start);
        let audio_at = show.timeline.millis(show.timeline.audio_frame);
        if show.audio_playable
            && clock >= audio_at
            && !show.audio_ready
            && show.audio_ticket.is_none()
        {
            show.audio_ticket = Some(host.begin_audio(
                client,
                AudioRequest::PlayAsset {
                    channel: CHANNEL,
                    source: AudioAsset {
                        path: show.music.clone(),
                        frame_limit: None,
                    },
                    intro: None,
                    parameters: AudioParameters {
                        gain: self.sound.all_volume / 100.,
                        mix_group: Some("com3d2-dance".into()),
                        retain_on_finish: true,
                        bus: PcmBus::Music,
                        ..Default::default()
                    },
                },
            )?);
        }
        if let Some(ticket) = show.audio_ticket {
            let status = host.audio_preparation(client, ticket)?;
            match status.phase {
                AudioPreparePhase::Failed => bail!(
                    "DanceMain audio {}: {}",
                    show.music.relative(),
                    status.error.unwrap_or_default()
                ),
                AudioPreparePhase::Ready => {
                    host.release_audio(client, ticket)?;
                    show.audio_ticket = None;
                    show.audio_ready = true;
                }
                AudioPreparePhase::Pending => {}
            }
        }
        let audio = if show.audio_ready {
            host.audio_info(client, CHANNEL)?
        } else {
            None
        };
        let audio_sample = audio
            .filter(|a| a.source_rate != 0)
            .map(|a| (a, a.position_frames * 1000 / u64::from(a.source_rate)));
        show.elapsed = match audio_sample {
            Some((a, pos)) if a.source_playing => audio_at + pos,
            Some((_, pos)) => {
                let (t, clock) = *show.eof.get_or_insert((audio_at + pos, now));
                t + now.saturating_sub(clock)
            }
            None if show.audio_playable => clock.min(audio_at),
            None => clock,
        };
        let elapsed = show.elapsed;
        let frame = elapsed as f32 * show.timeline.fps as f32 / 1000. + 1.;
        let audio_time = audio_sample.map(|(_, pos)| pos as f32 / 1000.);
        for (slot, (clip, frame)) in show.timeline.clips.iter().enumerate() {
            let start = show.timeline.millis(*frame);
            if elapsed >= start && !show.started_clips[slot] {
                self.actors.apply(
                    AdvUi::Motion {
                        man: false,
                        slot,
                        file: clip.clone(),
                        looping: false,
                        blend_s: 0.,
                        queue: false,
                    },
                    &self.script,
                    &self.files,
                    &mut self.assets,
                    op,
                    start,
                )?;
                show.started_clips[slot] = true;
            }
            if let Some(actor) = self.actors.actor(false, slot) {
                actor.performance_time = Some(elapsed);
                actor.performance_audio = audio_time;
            }
        }
        while let Some((at, event)) = show
            .timeline
            .cues
            .get(show.cue)
            .filter(|(at, _)| *at as f32 <= frame)
        {
            let effect = match event.clone() {
                Event::Face(slot, name, false) => Some(AdvUi::Face {
                    man: false,
                    slot,
                    name,
                }),
                Event::Face(slot, name, true) => Some(AdvUi::FaceBlend {
                    man: false,
                    slot,
                    name,
                }),
                Event::Eye(slot, name) => Some(AdvUi::Eye {
                    man: false,
                    slot,
                    look: crate::kag::EyeLook::Camera(name),
                    blend_s: 0.5,
                }),
                Event::Visible(slot, visible) => Some(AdvUi::CharaVisible {
                    man: false,
                    slot,
                    visible,
                }),
                Event::Fade(seconds, out) => {
                    let at = show.timeline.millis(*at);
                    let opacity = fade(show.opacity, at);
                    show.opacity = (
                        at,
                        opacity,
                        f32::from(out),
                        (seconds.max(0.) * 1000.) as u64,
                    );
                    None
                }
            };
            if let Some(effect) = effect {
                self.actors
                    .apply(effect, &self.script, &self.files, &mut self.assets, op, now)?;
            }
            show.cue += 1;
        }
        let eye =
            camera_position(&show.timeline.translation, frame).context("dance camera position")?;
        let rotation =
            sample(&show.timeline.rotation, frame, Quat::slerp).context("dance camera rotation")?;
        let direction = rotation * Vec3::Z;
        let target = eye + direction;
        let up = rotation * Vec3::Y;
        let mirror = |v: Vec3| [-v.x, v.y, v.z];
        self.main_light.intensity = sample(&show.timeline.light, frame, lerp).unwrap_or(0.95);
        let mut environment = self.main_light.environment(&self.settings);
        if let Some(color) = sample(&show.timeline.light_color, frame, Vec3::lerp) {
            environment["sun"]["color"] = json!(color.to_array());
        }
        host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":[
            {"op":"set_camera","camera":{"eye":mirror(eye),"target":mirror(target),"up":mirror(up),
                "fov_y_degrees":sample(&show.timeline.fov, frame, lerp).unwrap_or(35.).clamp(1., 179.),
                "near":0.1,"far":sample(&show.timeline.far, frame, lerp).unwrap_or(1000.).max(1.)}},
            {"op":"set_environment","environment":environment},
        ]}))?)?;
        apply_fade_widget(&mut self.widgets, fade(show.opacity, elapsed));
        if elapsed >= u64::from(show.timeline.frames) * 1000 / u64::from(show.timeline.fps) {
            let scene = show.scene.clone();
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            vm.eval(&format!(
                "tf['dance_scene_name']={}",
                serde_json::to_string(&scene)?
            ))?;
            drop(vm);
            return self.resume_at("label", op, host, client, now);
        }
        Ok(())
    }
}

fn fade((at, from, to, duration): (u64, f32, f32, u64), now: u64) -> f32 {
    if duration == 0 {
        to
    } else {
        lerp(
            from,
            to,
            (now.saturating_sub(at) as f32 / duration as f32).min(1.),
        )
    }
}

fn apply_fade_widget(widgets: &mut Vec<Widget>, opacity: f32) {
    widgets.retain(|w| w.path != "dance-cover");
    if opacity <= 0. {
        return;
    }
    widgets.push(Widget {
        path: "dance-cover".into(),
        rect: [0., 0., 1920., 1080.],
        depth: 4090,
        uv: [0., 0., 1., 1.],
        asset: String::new(),
        slices: None,
        tint: [0, 0, 0, (255. * opacity).round() as u8],
        label: None,
        component: 0,
        toggle: None,
        collider: None,
        button: None,
    });
}

#[test]
#[ignore = "installed COM3D2 Unity dance takes, sound and motion archives"]
fn installed_dance_performance_consumes_audio_clock_and_returns() -> Result<()> {
    let mut m = installed_menu::Menu::with_motion(true)?;
    for (level, cast) in [("level4", 3), ("level20", 1), ("level44", 3)] {
        let take = Timeline::load(&mut m.game.assets.unity, &mut m.op, level)?;
        assert_eq!(take.fps, 60);
        assert_eq!(take.clips.len(), cast);
        assert!(take.frames > 6000 && take.translation.len() > 50);
        assert!(
            take.cues
                .iter()
                .any(|(_, cue)| matches!(cue, Event::Face(_, name, _) if !name.is_empty()))
        );
        assert!(
            take.cues
                .iter()
                .any(|(_, cue)| matches!(cue, Event::Fade(seconds, _) if *seconds > 0.))
        );
        assert!(take.fov.iter().all(|key| (1. ..180.).contains(&key.value)));
    }
    {
        let mut vm = m.game.script.tjs.lock().unwrap();
        let s = &mut vm.runtime.host_mut().state;
        s.active_maids[0] = s.stock.first().copied();
        vm.eval("tf['dance_scene_name']='SceneDance_ETYL_Release'")?;
    }
    m.open_script("nightmain.ks", "*ダンススタート", Scene::DancePerformance)?;
    assert!(m.game.club_meshes > 0);
    for _ in 0..96 {
        m.rt.tick(16, InputFrame::default())?;
        m.game
            .update(&mut m.rt.host, m.client, &mut m.op, InputFrame::default())?;
        if m.game.performance.as_ref().unwrap().elapsed > 200 {
            break;
        }
    }
    let sound =
        m.rt.host
            .audio_info(m.client, CHANNEL)?
            .context("dance audio channel")?;
    let show = m.game.performance.as_mut().context("dance active")?;
    assert!(show.elapsed > 200);
    assert_eq!(
        show.elapsed,
        sound.position_frames * 1000 / u64::from(sound.source_rate)
    );
    assert!(show.started_clips[0]);
    let actor = m.game.actors.actor(false, 0).context("dance actor")?;
    assert_eq!(actor.performance_time, Some(show.elapsed));
    assert!(actor.face.kuchipaku.is_some());
    show.timeline.frames = 1; // Exercise the native completion callback without playing the entire song.
    m.tick(16)?;
    assert_eq!(m.game.scene, Scene::Script);
    assert_eq!(
        m.game.script.position().split_whitespace().next(),
        Some("nightmain.ks:143")
    );
    assert!(m.game.performance.is_none());
    assert!(!m.rt.host.audio_playing(m.client, CHANNEL)?);
    Ok(())
}
