// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! CPU-only M2 motion resources and sampling into engine-owned puppet meshes.
//!
//! Format evidence: Kirakira c451918023477620ff9055a2eb8a6714b947d9c1,
//! krkr-emote/{normalize,decode,model,motion}.rs. No upstream renderer is used.
//! Callers may load/sample immutable models on workers; only the owner publishes
//! the returned textures and PuppetFrame. No VM, filesystem or device is owned here.
use anyhow::{bail, ensure, Context, Result};
use crysirror_puppet::{PuppetFrame, MAX_PARTS, MAX_VERTICES};
use eluna::{
    EmoteDrawPass, EmoteModelSchema, EmoteStaticScene, PsbFile, PsbNormalizeOptions, PsbValue,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

const MAX_BYTES: usize = 256 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct MotionTexture {
    pub resource: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}
#[derive(Clone, Debug)]
pub struct MotionTiming {
    pub duration: f64,
    pub loop_start: Option<f64>,
}
/// Immutable parsed model shared by independent players and sampling workers.
#[derive(Clone)]
pub struct MotionModel {
    psb: Arc<PsbFile>,
    bytes: Arc<[u8]>,
    schema: EmoteModelSchema,
    textures: Vec<MotionTexture>,
}
impl MotionModel {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_BYTES,
            "motion exceeds 256 MiB input limit"
        );
        let (plain, mut psb) = PsbFile::parse_normalized(bytes, &PsbNormalizeOptions::default())?;
        ensure!(plain.len() <= MAX_BYTES, "normalized motion exceeds limit");
        let mut budget = 2_000_000usize;
        check_tree(&psb.root, 0, &mut budget)?;
        // The first profile does not dispatch authored per-layer action tags.
        // Reject such archives instead of silently losing script events.
        reject_action_tags(&psb.root)?;
        let textures = texture_resources(&psb, &plain)?;
        normalize(&mut psb.root)?;
        let schema = EmoteModelSchema::from_psb(&psb)?;
        ensure!(
            !textures.is_empty(),
            "motion has no decoded texture resources"
        );
        Ok(Self {
            psb: Arc::new(psb),
            bytes: plain.into(),
            schema,
            textures,
        })
    }
    pub fn psb(&self) -> &PsbFile {
        &self.psb
    }
    pub fn textures(&self) -> &[MotionTexture] {
        &self.textures
    }
    pub fn base_object(&self) -> &str {
        &self.schema.base_object
    }
    pub fn motions(&self, object: &str) -> Result<Vec<String>> {
        let table = self.motion_table(object)?;
        Ok(table.iter().map(|(name, _)| name.clone()).collect())
    }
    fn motion_table(&self, object: &str) -> Result<&[(String, PsbValue)]> {
        self.psb
            .root
            .field("object")
            .and_then(|v| v.field(object))
            .and_then(|v| v.field("motion"))
            .and_then(PsbValue::as_object)
            .with_context(|| format!("unknown motion object {object:?}"))
    }
    pub fn timing(&self, object: &str, animation: &str) -> Result<MotionTiming> {
        let motion = self
            .motion_table(object)?
            .iter()
            .find(|(k, _)| k == animation)
            .map(|(_, v)| v)
            .with_context(|| format!("unknown motion {object}/{animation}"))?;
        let mut schema = self.schema.clone();
        schema.base_object = object.to_owned();
        let duration = schema
            .motion_infos(&self.psb)?
            .into_iter()
            .find(|v| v.name == animation)
            .context("missing motion timing")?
            .duration_ticks;
        ensure!(
            duration.is_finite() && duration >= 0.,
            "invalid motion duration"
        );
        Ok(MotionTiming {
            duration: f64::from(duration),
            loop_start: motion
                .field_f32("loopTime")
                .filter(|v| v.is_finite() && *v >= 0. && *v < duration)
                .map(f64::from),
        })
    }
    /// Sample meshes in model coordinates; affine and viewport conversion happen
    /// here, leaving native adapters responsible only for argument mapping.
    /// Stateless: a serialized type-0 HOLD keyframe has no previous step to
    /// hold here; players use [`MotionModel::step`].
    pub fn frame(
        &self,
        object: &str,
        animation: &str,
        tick: f64,
        variables: &BTreeMap<String, f32>,
        affine: [f32; 6],
        size: [f32; 2],
        opacity: f32,
    ) -> Result<PuppetFrame> {
        self.step(object, animation, tick, variables, affine, size, opacity, None)
            .map(|(frame, _)| frame)
    }
    /// One native StepFrame: like [`MotionModel::frame`], continuing from the
    /// player's previous step so type-0 HOLD keyframes keep the decoded layer
    /// state (MMotionPlayer sub_1032FB00, eluna's previous-scene path). The
    /// returned scene is the next step's `previous`.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &self,
        object: &str,
        animation: &str,
        tick: f64,
        variables: &BTreeMap<String, f32>,
        affine: [f32; 6],
        size: [f32; 2],
        opacity: f32,
        previous: Option<&EmoteStaticScene>,
    ) -> Result<(PuppetFrame, EmoteStaticScene)> {
        self.timing(object, animation)?;
        ensure!(
            tick.is_finite()
                && tick >= 0.
                && affine.iter().all(|v| v.is_finite())
                && size.iter().all(|v| v.is_finite() && *v > 0.)
                && opacity.is_finite(),
            "invalid motion sample"
        );
        let mut schema = self.schema.clone();
        schema.base_object = object.to_owned();
        let scene = match previous {
            Some(previous) => schema.build_motion_scene_at_with_resources_variables_and_previous_scene(
                &self.psb,
                &self.bytes,
                animation,
                tick as f32,
                variables,
                previous,
            )?,
            None => schema.build_motion_scene_at_with_resources_and_variables(
                &self.psb,
                &self.bytes,
                animation,
                tick as f32,
                variables,
            )?,
        };
        ensure!(
            scene.sprites.len() <= MAX_PARTS,
            "motion part limit exceeded"
        );
        let slots: HashMap<u32, u32> = self
            .textures
            .iter()
            .enumerate()
            .map(|(i, t)| (t.resource, i as u32))
            .collect();
        let infos = scene
            .layer_states
            .iter()
            .map(|s| (&s.draw_frame_info.native_draw_key, &s.draw_frame_info))
            .collect();
        let mut parts = Vec::new();
        let mut masks = Vec::new();
        let mut keys = HashMap::new();
        if std::env::var_os("CRYSIRROR_MOTION_TRACE_PARTS").is_some() {
            for s in &scene.sprites {
                eprintln!(
                    "[motion-sprite] {object}/{animation} tick={tick:.1} label={:?} tex={} visible={} opacity={:.2} pass={:?} blend={} mask_key={:?}",
                    s.label, s.texture_resource_index, s.visible, s.opacity, s.draw_frame_info.pass, s.blend_mode,
                    s.draw_frame_info.native_draw_key
                );
            }
        }
        for sprite in scene.sprites.iter().filter(|s| s.visible && s.opacity > 0.) {
            ensure!(
                !sprite.feedback_history,
                "motion framebuffer feedback is unsupported"
            );
            ensure!(
                matches!(sprite.blend_mode & 0xFF0F, 0..=5),
                "unsupported motion blend {}",
                sprite.blend_mode
            );
            let slot = *slots
                .get(&sprite.texture_resource_index)
                .context("motion texture missing")?;
            if let Some(mesh) = sprite.mesh {
                ensure!(
                    (u64::from(mesh.division_x) + 1) * (u64::from(mesh.division_y) + 1)
                        <= MAX_VERTICES as u64,
                    "motion mesh exceeds vertex limit"
                );
            }
            let (inside, outside) = super::stencil_sources(&scene, &infos, sprite);
            ensure!(
                inside.is_empty() || outside.is_empty(),
                "mixed inside/outside motion mask chain unsupported"
            );
            ensure!(
                inside.len() <= 1 && outside.len() <= 1,
                "nested motion mask intersections require a shared mask-composition pass"
            );
            ensure!(
                !sprite.draw_frame_info.stencil_wipe_enabled,
                "motion stencil wipe requires a shared wipe-mask pass"
            );
            let invert = inside.is_empty() && !outside.is_empty();
            masks.push((if invert { outside } else { inside }, invert));
            keys.insert(sprite.draw_frame_info.native_draw_key.clone(), parts.len());
            let mut part = super::sprite_part(sprite, slot);
            if let Some(rect) = sprite.draw_frame_info.clip_rect {
                crysirror_puppet::clip_part_rect(&mut part, rect)?;
            }
            part.visible = !matches!(
                sprite.draw_frame_info.pass,
                EmoteDrawPass::MaskGeneration | EmoteDrawPass::StencilCompositeMask
            );
            if std::env::var_os("CRYSIRROR_MOTION_TRACE_PARTS").is_some() {
                let xs = part.positions.iter().map(|p| p[0]);
                let ys = part.positions.iter().map(|p| p[1]);
                eprintln!(
                    "[motion-part] {object}/{animation} tick={tick:.1} key={:?} left={} top={} w={} h={} raw=[{:.0},{:.0}..{:.0},{:.0}]",
                    sprite.draw_frame_info.native_draw_key, sprite.left(), sprite.top(), sprite.width, sprite.height,
                    xs.clone().fold(f32::INFINITY, f32::min), ys.clone().fold(f32::INFINITY, f32::min),
                    xs.fold(f32::NEG_INFINITY, f32::max), ys.fold(f32::NEG_INFINITY, f32::max),
                );
            }
            for point in &mut part.positions {
                let [x, y] = *point;
                *point = [
                    affine[0] * x + affine[2] * y + affine[4],
                    affine[1] * x + affine[3] * y + affine[5],
                ];
            }
            for color in &mut part.colors {
                color[3] *= opacity.clamp(0., 1.);
            }
            parts.push(part);
        }
        for (part, (references, invert)) in parts.iter_mut().zip(masks) {
            part.masks = references
                .iter()
                .map(|k| keys.get(k).copied().context("motion mask source missing"))
                .collect::<Result<_>>()?;
            part.invert_mask = invert;
        }
        ensure!(
            parts.iter().map(|p| p.positions.len()).sum::<usize>() <= MAX_VERTICES,
            "motion total vertex limit exceeded"
        );
        let frame = PuppetFrame {
            size,
            parts,
            textures: self
                .textures
                .iter()
                .map(|t| format!("motion-texture-{}", t.resource))
                .collect(),
        };
        Ok((frame, scene))
    }
}
/// An independent 60 Hz motion clock. Completion is an edge, delivered once.
#[derive(Clone, Debug)]
pub struct MotionClock {
    pub tick: f64,
    pub speed: f64,
    pub playing: bool,
    pub timing: MotionTiming,
}
impl MotionClock {
    pub fn new(timing: MotionTiming) -> Self {
        Self {
            tick: 0.,
            speed: 1.,
            playing: true,
            timing,
        }
    }
    pub fn advance_ms(&mut self, ms: f64) -> Result<bool> {
        ensure!(
            ms.is_finite() && ms >= 0. && self.speed.is_finite() && self.speed >= 0.,
            "invalid motion clock delta/speed"
        );
        if !self.playing {
            return Ok(false);
        }
        self.tick += ms * 0.06 * self.speed;
        if self.timing.duration == 0. {
            return Ok(false);
        }
        if self.tick < self.timing.duration {
            return Ok(false);
        }
        if let Some(start) = self.timing.loop_start {
            let span = self.timing.duration - start;
            ensure!(span > 0., "invalid loop interval");
            self.tick = start + (self.tick - self.timing.duration) % span;
            Ok(false)
        } else {
            self.tick = self.timing.duration;
            self.playing = false;
            Ok(true)
        }
    }
}
fn reject_action_tags(value: &PsbValue) -> Result<()> {
    match value {
        PsbValue::Object(fields) => {
            if let Some(action) = value.field_str("act") {
                let enabled = value
                    .field_i64("mask")
                    .is_none_or(|mask| mask & 0x40000 != 0);
                ensure!(
                    !enabled || action.is_empty(),
                    "motion action tags require shared event sampling ({action:?})"
                );
            }
            for (_, child) in fields {
                reject_action_tags(child)?;
            }
        }
        PsbValue::List(values) => {
            for child in values {
                reject_action_tags(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn check_tree(value: &PsbValue, depth: usize, budget: &mut usize) -> Result<()> {
    ensure!(
        depth <= 128 && *budget > 0,
        "motion PSB tree limit exceeded"
    );
    *budget -= 1;
    match value {
        PsbValue::Object(o) => {
            for (_, v) in o {
                check_tree(v, depth + 1, budget)?
            }
        }
        PsbValue::List(a) => {
            for v in a {
                check_tree(v, depth + 1, budget)?
            }
        }
        _ => {}
    }
    Ok(())
}
fn put(fields: &mut Vec<(String, PsbValue)>, key: &str, value: PsbValue) {
    if let Some((_, v)) = fields.iter_mut().find(|(k, _)| k == key) {
        *v = value
    } else {
        fields.push((key.into(), value))
    }
}
fn dimension(value: &PsbValue, key: &str) -> Result<u32> {
    let n = value
        .field_f32(key)
        .with_context(|| format!("missing texture {key}"))?;
    ensure!(
        n.is_finite() && n >= 1. && n <= 16384.,
        "invalid texture {key}: {n}"
    );
    Ok(n.round() as u32)
}
fn normalize(root: &mut PsbValue) -> Result<()> {
    let PsbValue::Object(fields) = root else {
        bail!("motion root is not object")
    };
    let sources = fields
        .iter_mut()
        .find(|(k, _)| k == "source")
        .context("motion source table missing")?;
    let PsbValue::Object(sources) = &mut sources.1 else {
        bail!("motion source table invalid")
    };
    let mut aliases = BTreeMap::new();
    let mut added = Vec::new();
    for (name, source) in sources.iter() {
        let Some(icons) = source.field("icon").and_then(PsbValue::as_object) else {
            continue;
        };
        for (icon_name, icon) in icons {
            let reference = format!("{name}/{icon_name}");
            if source.field("texture").is_some() {
                aliases.insert(reference, (name.clone(), icon_name.clone()));
                continue;
            }
            let Some(resource) = icon.field_u32("pixel") else {
                log::warn!("motion icon {reference} has no pixel resource: {icon:?}");
                continue;
            };
            let width = dimension(icon, "width")?;
            let height = dimension(icon, "height")?;
            let resolution = icon.field_f32("resolution").unwrap_or(1.);
            ensure!(
                resolution.is_finite() && resolution > 0.,
                "invalid motion icon resolution"
            );
            let mut texture = vec![
                ("pixel".into(), PsbValue::Resource(resource)),
                (
                    "width".into(),
                    PsbValue::Int((width as f32 * resolution).round() as i64),
                ),
                (
                    "height".into(),
                    PsbValue::Int((height as f32 * resolution).round() as i64),
                ),
            ];
            for key in ["compress", "type"] {
                if let Some(v) = icon.field(key).or_else(|| source.field(key)) {
                    texture.push((key.into(), v.clone()))
                }
            }
            let mut rect = vec![
                ("left".into(), PsbValue::Int(0)),
                ("top".into(), PsbValue::Int(0)),
            ];
            for key in [
                "width",
                "height",
                "resolution",
                "originX",
                "originY",
                "attr",
            ] {
                if let Some(v) = icon.field(key) {
                    rect.push((key.into(), v.clone()))
                }
            }
            added.push((
                reference.clone(),
                PsbValue::Object(vec![
                    ("texture".into(), PsbValue::Object(texture)),
                    (
                        "icon".into(),
                        PsbValue::Object(vec![(icon_name.clone(), PsbValue::Object(rect))]),
                    ),
                ]),
            ));
            aliases.insert(reference.clone(), (reference, icon_name.clone()));
        }
    }
    sources.extend(added);
    rewrite(root, &aliases)
}
fn rewrite(value: &mut PsbValue, aliases: &BTreeMap<String, (String, String)>) -> Result<()> {
    match value {
        PsbValue::Object(fields) => {
            fields.retain(|(k, v)| !(k == "parameterize" && matches!(v, PsbValue::Null)));
            let src = fields
                .iter()
                .find(|(k, _)| k == "src")
                .and_then(|(_, v)| v.as_str())
                .map(str::to_owned);
            if let Some(src) = src.and_then(|s| s.strip_prefix("src/").map(str::to_owned)) {
                let explicit_icon = fields
                    .iter()
                    .find(|(k, _)| k == "icon")
                    .and_then(|(_, v)| v.as_str());
                let reference = if src.contains('/') {
                    src
                } else {
                    format!("{src}/{}", explicit_icon.context("motion src lacks icon")?)
                };
                match aliases.get(&reference) {
                    Some((texture, icon)) => {
                        put(fields, "src", PsbValue::String(texture.clone()));
                        put(fields, "icon", PsbValue::String(icon.clone()));
                    }
                    None => {
                        // An icon without decodable pixels (logged with its
                        // structure under CRYSIRROR_KRKR_TRACE_MOTION): the
                        // frame draws nothing, as eluna does for an empty src.
                        log::warn!("motion icon {reference} has no texture; its frames are not drawn");
                        put(fields, "src", PsbValue::String(String::new()));
                    }
                }
            }
            for (_, v) in fields {
                rewrite(v, aliases)?
            }
        }
        PsbValue::List(values) => {
            for v in values {
                rewrite(v, aliases)?
            }
        }
        _ => {}
    }
    Ok(())
}
fn texture_resources(psb: &PsbFile, plain: &[u8]) -> Result<Vec<MotionTexture>> {
    let sources = psb
        .root
        .field("source")
        .and_then(PsbValue::as_object)
        .context("motion source missing")?;
    let mut textures = BTreeMap::new();
    let mut total = 0usize;
    // KiriKiri/Windows PSBs (`spec` "krkr"/"win") keep 32-bit pixels in the
    // engine's in-memory BGRA order, like their ARGB palettes; other specs
    // store R, G, B, A.
    let bgra = matches!(psb.root.field_str("spec"), Some("krkr" | "win"));
    for (_, source) in sources {
        let entries: Vec<&PsbValue> = if let Some(t) = source.field("texture") {
            vec![t]
        } else {
            source
                .field("icon")
                .and_then(PsbValue::as_object)
                .map(|v| v.iter().map(|(_, v)| v).collect())
                .unwrap_or_default()
        };
        for icon in entries {
            let Some(resource) = icon.field_u32("pixel") else {
                continue;
            };
            if textures.contains_key(&resource) {
                continue;
            }
            let width = dimension(icon, "width")?;
            let height = dimension(icon, "height")?;
            total = total
                .checked_add(width as usize * height as usize * 4)
                .context("texture size overflow")?;
            ensure!(total <= MAX_BYTES, "motion decoded texture budget exceeded");
            let raw = psb
                .resource_bytes(plain, resource as usize)
                .context("missing motion pixel resource")?;
            let palette = icon
                .field_u32("pal")
                .map(|p| {
                    psb.resource_bytes(plain, p as usize)
                        .context("missing motion palette")
                })
                .transpose()?;
            let mut rgba = decode_pixels(raw, palette, width, height, icon.field_str("compress"))?;
            if bgra && palette.is_none() {
                for p in rgba.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
            }
            // Diagnostic: CRYSIRROR_MOTION_DUMP_TEXTURES=<dir> writes each
            // decoded texture as raw bytes (`<resource>-<w>x<h>.rgba`).
            if let Some(dir) = std::env::var_os("CRYSIRROR_MOTION_DUMP_TEXTURES") {
                let path = std::path::Path::new(&dir).join(format!("{resource}-{width}x{height}.rgba"));
                let _ = std::fs::write(path, &rgba);
            }
            textures.insert(
                resource,
                MotionTexture {
                    resource,
                    width,
                    height,
                    rgba: rgba.into(),
                },
            );
        }
    }
    Ok(textures.into_values().collect())
}
/// Bounded RL decoder; malformed runs cannot allocate beyond declared image size.
pub fn decode_pixels(
    raw: &[u8],
    palette: Option<&[u8]>,
    width: u32,
    height: u32,
    compression: Option<&str>,
) -> Result<Vec<u8>> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .context("texture size overflow")?;
    ensure!(
        pixels > 0 && pixels <= MAX_BYTES / 4,
        "invalid motion texture size"
    );
    let stride = if palette.is_some() { 1 } else { 4 };
    let expected = pixels * stride;
    let mut data = Vec::with_capacity(expected);
    match compression {
        None | Some("") => {
            ensure!(raw.len() == expected, "motion raw texture size mismatch");
            data.extend_from_slice(raw)
        }
        Some("RL") => {
            let mut cursor = 0;
            while cursor < raw.len() {
                let command = raw[cursor];
                cursor += 1;
                let repeat = command & 128 != 0;
                let count = if repeat {
                    (command & 127) as usize + 3
                } else {
                    command as usize + 1
                };
                let bytes = count * stride;
                ensure!(
                    data.len() + bytes <= expected,
                    "motion RL exceeds declared size"
                );
                let take = if repeat { stride } else { bytes };
                let unit = raw
                    .get(cursor..cursor + take)
                    .context("truncated motion RL stream")?;
                cursor += take;
                if repeat {
                    for _ in 0..count {
                        data.extend_from_slice(unit)
                    }
                } else {
                    data.extend_from_slice(unit)
                }
            }
            ensure!(data.len() == expected, "motion RL size mismatch");
        }
        Some(codec) => bail!("unsupported motion texture compression {codec:?}"),
    }
    if let Some(palette) = palette {
        ensure!(
            palette.len() >= 1024,
            "motion palette is shorter than 256 ARGB entries"
        );
        let mut rgba = Vec::with_capacity(pixels * 4);
        for index in data {
            let p = &palette[index as usize * 4..index as usize * 4 + 4];
            rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]])
        }
        Ok(rgba)
    } else {
        Ok(data)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_runs_and_palette() {
        let mut p = vec![0; 1024];
        p[4..8].copy_from_slice(&[3, 5, 7, 9]);
        assert_eq!(
            decode_pixels(&[128, 1, 0, 0], Some(&p), 4, 1, Some("RL")).unwrap(),
            [7, 5, 3, 9, 7, 5, 3, 9, 7, 5, 3, 9, 0, 0, 0, 0]
        );
        assert!(decode_pixels(&[255, 0], Some(&p), 4, 1, Some("RL")).is_err());
        assert!(decode_pixels(&[128, 1, 2], None, 3, 1, Some("RL")).is_err());
    }
    #[test]
    fn clock_completion_and_intro_loop() {
        let mut c = MotionClock::new(MotionTiming {
            duration: 60.,
            loop_start: None,
        });
        assert!(!c.advance_ms(500.).unwrap());
        assert!(c.advance_ms(500.).unwrap());
        assert!(!c.advance_ms(500.).unwrap());
        assert_eq!(c.tick, 60.);
        let mut c = MotionClock::new(MotionTiming {
            duration: 60.,
            loop_start: Some(30.),
        });
        c.advance_ms(1750.).unwrap();
        assert_eq!(c.tick, 45.);
        assert!(c.playing);
    }
    #[test]
    #[ignore = "requires owner-provided MOTION_FILE, no game assets in repository"]
    fn external_motion_samples_to_engine_meshes() {
        let path = std::env::var("MOTION_FILE").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let model = MotionModel::from_bytes(&bytes).unwrap();
        let object = std::env::var("MOTION_OBJECT").unwrap_or_else(|_| model.base_object().into());
        let animation = std::env::var("MOTION_NAME")
            .unwrap_or_else(|_| model.motions(&object).unwrap()[0].clone());
        let timing = model.timing(&object, &animation).unwrap();
        let mut populated = 0;
        let mut max_parts = 0;
        for i in 0..=10 {
            let frame = model
                .frame(
                    &object,
                    &animation,
                    timing.duration * i as f64 / 10.,
                    &BTreeMap::new(),
                    [1., 0., 0., 1., 0., 0.],
                    [1920., 1080.],
                    1.,
                )
                .unwrap();
            if !frame.parts.is_empty() {
                populated += 1
            }
            max_parts = max_parts.max(frame.parts.len());
        }
        assert!(populated > 0, "motion never emits engine geometry");
        eprintln!("motion {object}/{animation}: duration={} loop={:?} textures={} max_parts={max_parts} populated_samples={populated}",timing.duration,timing.loop_start,model.textures.len());
    }
}

/// Bounded worker owner. Jobs hold immutable model snapshots; publishing and VM
/// callbacks stay on the caller's owner thread. Dropping a job cancels its work.
pub struct MotionWorkers(crysirror_tasks::TaskPool);
pub type MotionJob<T> = crysirror_tasks::Task<T>;
#[derive(Clone)]
pub struct MotionSample {
    pub model: Arc<MotionModel>,
    pub object: String,
    pub animation: String,
    pub tick: f64,
    pub variables: BTreeMap<String, f32>,
    pub affine: [f32; 6],
    pub size: [f32; 2],
    pub opacity: f32,
    /// The player's previous step (same model, object and animation).
    pub previous: Option<Arc<EmoteStaticScene>>,
}
/// A sampled frame and the step state that continues it.
pub struct MotionStep {
    pub frame: PuppetFrame,
    pub scene: Arc<EmoteStaticScene>,
}
impl MotionWorkers {
    pub fn new() -> Result<Self> {
        Ok(Self(crysirror_tasks::TaskPool::new("motion", 2, 32)?))
    }
    pub fn load(&self, bytes: Vec<u8>) -> Result<MotionJob<Arc<MotionModel>>> {
        self.0.spawn(move |cancel| {
            cancel.check()?;
            let model = MotionModel::from_bytes(&bytes)?;
            cancel.check()?;
            Ok(Arc::new(model))
        })
    }
    pub fn sample(&self, sample: MotionSample) -> Result<MotionJob<MotionStep>> {
        self.0.spawn(move |cancel| {
            cancel.check()?;
            let (frame, scene) = sample.model.step(
                &sample.object,
                &sample.animation,
                sample.tick,
                &sample.variables,
                sample.affine,
                sample.size,
                sample.opacity,
                sample.previous.as_deref(),
            )?;
            frame.validate()?;
            cancel.check()?;
            Ok(MotionStep { frame, scene: Arc::new(scene) })
        })
    }
}
