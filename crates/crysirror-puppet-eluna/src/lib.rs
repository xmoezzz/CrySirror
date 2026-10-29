// SPDX-License-Identifier: MPL-2.0
//! The eluna E-mote player as a [`PuppetSource`] (ADR 0029).
//!
//! E-mote models are loaded from the owner's local PSB files with the
//! vendored eluna runtime (the same player the Siglus adapter uses). This
//! bridge only *supplies data* through the engine interface: frames,
//! parameters (E-mote variables) and motions (timelines). Packages never
//! contain PSB; [`crysirror_puppet::bake`] turns a source into the engine's
//! own rig format.
//!
//! Sprite geometry follows siglus_scene_vm `render_plan/emote.rs`
//! (`sprite_vertices`, `mesh_sprite_vertices`, `transform_sprite_point`,
//! `native_corner_color`, stencil chains) at the vendored commit; the
//! difference is that positions stay in model space instead of the Siglus
//! render-target clip space.
#![forbid(unsafe_code)]
pub mod motion;
use anyhow::{bail, ensure, Context, Result};
use crysirror_puppet::{FramePart, MotionInfo, ParameterInfo, PartBlend, PlayOptions, PuppetFrame, PuppetSource};
use eluna::{EmoteDrawPass, EmoteStaticScene, EmoteStaticSprite, TimelinePlayMode};
use siglus_scene_vm::emote::SiglusEmoteRuntime;
use std::collections::{HashMap, HashSet};

/// E-mote ticks per second (the player runs on a 60 Hz tick clock).
const TICKS_PER_SECOND: f32 = 60.;

/// What the bridge could not express in the engine frame model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeDiagnostics {
    /// Feedback (previous-framebuffer) sprites, which have no texture.
    pub feedback_sprites: usize,
    /// Sprites whose texture resource was not decoded.
    pub missing_textures: usize,
    /// Mask chains mixing inside (phase 1) and outside (phase 2) masks;
    /// only the inside masks are kept.
    pub mixed_mask_chains: usize,
}

pub struct ElunaPuppet {
    runtime: SiglusEmoteRuntime,
    /// Model-space origin (bounds minimum at load) and canvas size.
    origin: [f32; 2],
    size: [f32; 2],
    /// Decoded texture resource indices, in engine texture order.
    textures: Vec<u32>,
    /// Fractional milliseconds not yet passed to the integer-ms player.
    pending_ms: f32,
    diagnostics: std::cell::Cell<BridgeDiagnostics>,
}

impl ElunaPuppet {
    /// Loads PSB sources (main file first) with an optional decryption key.
    pub fn from_psb(sources: &[&[u8]], key: Option<u32>) -> Result<Self> {
        let runtime = SiglusEmoteRuntime::from_psb_sources(sources, key)?;
        let mut textures: Vec<u32> = runtime.decoded_textures().keys().copied().collect();
        textures.sort_unstable();
        let mut this = Self {
            runtime,
            origin: [0.; 2],
            size: [1.; 2],
            textures,
            pending_ms: 0.,
            diagnostics: Default::default(),
        };
        let scene = this.runtime.current_scene();
        let (min, max) = match scene.bounds {
            Some(b) => ([b.min_x, b.min_y], [b.max_x, b.max_y]),
            None => {
                let frame = this.frame_in_model_space()?;
                let mut lo = [f32::MAX; 2];
                let mut hi = [f32::MIN; 2];
                for p in frame.parts.iter().flat_map(|p| &p.positions) {
                    for c in 0..2 {
                        lo[c] = lo[c].min(p[c]);
                        hi[c] = hi[c].max(p[c]);
                    }
                }
                ensure!(lo[0] < hi[0] && lo[1] < hi[1], "E-mote model has no visible geometry at rest");
                (lo, hi)
            }
        };
        this.origin = min;
        this.size = [(max[0] - min[0]).max(1.), (max[1] - min[1]).max(1.)];
        Ok(this)
    }
    pub fn diagnostics(&self) -> BridgeDiagnostics {
        self.diagnostics.get()
    }

    fn frame_in_model_space(&self) -> Result<PuppetFrame> {
        let scene = self.runtime.current_scene();
        let texture_slot: HashMap<u32, u32> = self.textures.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect();
        let mut diag = BridgeDiagnostics::default();
        let usable = |s: &EmoteStaticSprite, diag: &mut BridgeDiagnostics| {
            if s.feedback_history {
                diag.feedback_sprites += 1;
                return false;
            }
            if !texture_slot.contains_key(&s.texture_resource_index) {
                diag.missing_textures += 1;
                return false;
            }
            true
        };
        let present: Vec<&EmoteStaticSprite> = scene.sprites.iter().filter(|s| s.visible && s.opacity > 0.).collect();
        let layer_infos: HashMap<&Vec<u64>, &eluna::EmoteDrawFrameInfo> = scene
            .layer_states
            .iter()
            .map(|st| (&st.draw_frame_info.native_draw_key, &st.draw_frame_info))
            .collect();
        // Each sprite's mask list comes from its own stencil chain.
        let mut parts: Vec<FramePart> = Vec::new();
        let mut index_of: HashMap<Vec<u64>, usize> = HashMap::new();
        let mut pending_masks: Vec<(usize, Vec<Vec<u64>>, bool)> = Vec::new();
        for sprite in &present {
            let is_mask = matches!(sprite.draw_frame_info.pass, EmoteDrawPass::MaskGeneration | EmoteDrawPass::StencilCompositeMask);
            if !usable(sprite, &mut diag) {
                continue;
            }
            let (inside, outside) = stencil_sources(scene, &layer_infos, sprite);
            if !inside.is_empty() && !outside.is_empty() {
                diag.mixed_mask_chains += 1;
            }
            let (masks, invert) = if inside.is_empty() { (outside, true) } else { (inside, false) };
            let i = parts.len();
            index_of.insert(sprite.draw_frame_info.native_draw_key.clone(), i);
            pending_masks.push((i, masks, invert));
            let mut part = sprite_part(sprite, texture_slot[&sprite.texture_resource_index]);
            part.visible = !is_mask;
            parts.push(part);
        }
        // Resolve mask references to frame indices; mask-only sources that
        // were filtered out are dropped.
        for (i, keys, invert) in pending_masks {
            let masks: Vec<usize> = keys.iter().filter_map(|k| index_of.get(k).copied()).collect();
            parts[i].invert_mask = invert && !masks.is_empty();
            parts[i].masks = masks;
        }
        self.diagnostics.set(diag);
        Ok(PuppetFrame { size: self.size, parts, textures: self.texture_names() })
    }
}

/// Stencil chain of `sprite` as in siglus_scene_vm `stencil_groups_for_sprite`:
/// ancestors with phase 1 (inside) or 2 (outside), expanding composite masks.
fn stencil_sources(
    scene: &EmoteStaticScene,
    layer_infos: &HashMap<&Vec<u64>, &eluna::EmoteDrawFrameInfo>,
    sprite: &EmoteStaticSprite,
) -> (Vec<Vec<u64>>, Vec<Vec<u64>>) {
    let mut inside = Vec::new();
    let mut outside = Vec::new();
    let mut cursor = sprite.draw_frame_info.stencil_parent_native_key.as_ref();
    let mut visited = HashSet::new();
    while let Some(key) = cursor {
        if !visited.insert(key.clone()) {
            break;
        }
        let Some(info) = layer_infos.get(key).copied() else { break };
        if matches!(info.stencil_phase, 1 | 2) {
            let sources = if (info.stencil_type & 4) != 0 {
                scene.composite_mask_sources_by_key.get(&info.native_draw_key).cloned().unwrap_or_default()
            } else {
                vec![info.native_draw_key.clone()]
            };
            if info.stencil_phase == 1 { inside.extend(sources) } else { outside.extend(sources) }
        }
        cursor = info.stencil_parent_native_key.as_ref();
    }
    (inside, outside)
}

fn part_id(s: &EmoteStaticSprite) -> String {
    let key: Vec<String> = s.draw_frame_info.native_draw_key.iter().map(u64::to_string).collect();
    format!("{}#{}", s.label.as_deref().unwrap_or(&s.motion_name), key.join("."))
}
fn corner_color(s: &EmoteStaticSprite, packed: u32) -> [f32; 4] {
    let c = |shift: u32| ((packed >> shift) & 0xff) as f32 / 255.;
    // E-mote MODULATE2X (`bm & 0xF0 == 0x10`, the default blend): the
    // texture is multiplied by twice the vertex colour, so 0x80 is neutral
    // (eluna's frame decoder defaults such frames to 0x808080FF).
    let gain = if s.blend_mode & 0xF0 == 0x10 { 255. / 128. } else { 1. };
    [c(24) * gain, c(16) * gain, c(8) * gain, (c(0) * s.opacity).clamp(0., 1.)]
}
fn bilerp(c: &[[f32; 4]; 4], u: f32, v: f32) -> [f32; 4] {
    let mut out = [0.; 4];
    for i in 0..4 {
        let top = c[0][i] + (c[1][i] - c[0][i]) * u;
        let bottom = c[2][i] + (c[3][i] - c[2][i]) * u;
        out[i] = top + (bottom - top) * v;
    }
    out
}
fn transform_point(s: &EmoteStaticSprite, p: [f32; 2]) -> [f32; 2] {
    let sx = if s.scale_x.is_finite() { s.scale_x } else { 1. };
    let sy = if s.scale_y.is_finite() { s.scale_y } else { 1. };
    let a = if s.rotation_degrees.is_finite() { s.rotation_degrees.to_radians() } else { 0. };
    let (sin, cos) = a.sin_cos();
    let dx = (p[0] - s.center_x) * sx;
    let dy = (p[1] - s.center_y) * sy;
    let local = [s.center_x + dx * cos - dy * sin, s.center_y + dx * sin + dy * cos];
    let m = s.world_transform;
    [m[0] * local[0] + m[1] * local[1] + m[4], m[2] * local[0] + m[3] * local[1] + m[5]]
}
fn blend(mode: u32) -> PartBlend {
    // siglus_scene_vm native_blend_index / native_blend_state.
    match mode & 0xFF0F {
        1 => PartBlend::Add,
        2 | 5 => PartBlend::Subtract,
        3 => PartBlend::Multiply,
        4 => PartBlend::Screen,
        _ => PartBlend::Normal,
    }
}
fn sprite_part(s: &EmoteStaticSprite, texture: u32) -> FramePart {
    let corners = [
        corner_color(s, s.corner_colors[0]),
        corner_color(s, s.corner_colors[1]),
        corner_color(s, s.corner_colors[2]),
        corner_color(s, s.corner_colors[3]),
    ];
    let (dx, dy) = match &s.mesh {
        Some(m) => (m.division_x.max(1) as usize, m.division_y.max(1) as usize),
        None => (1, 1),
    };
    let (mut positions, mut uvs, mut colors) = (Vec::new(), Vec::new(), Vec::new());
    for iy in 0..=dy {
        for ix in 0..=dx {
            let u = ix as f32 / dx as f32;
            let v = iy as f32 / dy as f32;
            let local = match &s.mesh {
                Some(m) => {
                    let p = m.sample(u, v);
                    [s.left() + p[0] * s.width, s.top() + p[1] * s.height]
                }
                None => [s.left() + u * s.width, s.top() + v * s.height],
            };
            positions.push(transform_point(s, local));
            uvs.push([s.uv_left + (s.uv_right - s.uv_left) * u, s.uv_top + (s.uv_bottom - s.uv_top) * v]);
            colors.push(bilerp(&corners, u, v));
        }
    }
    let mut indices = Vec::with_capacity(dx * dy * 6);
    let row = (dx + 1) as u32;
    for y in 0..dy as u32 {
        for x in 0..dx as u32 {
            let tl = y * row + x;
            let (tr, bl, br) = (tl + 1, tl + row, tl + row + 1);
            indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
        }
    }
    FramePart {
        id: part_id(s),
        texture,
        positions,
        uvs,
        indices,
        colors,
        blend: blend(s.blend_mode),
        masks: Vec::new(),
        invert_mask: false,
        visible: true,
    }
}

impl PuppetSource for ElunaPuppet {
    fn size(&self) -> [f32; 2] {
        self.size
    }
    fn parameters(&self) -> Vec<ParameterInfo> {
        let mut out: Vec<ParameterInfo> = self
            .runtime
            .variable_infos()
            .into_iter()
            .map(|v| {
                let mut keys: Vec<f32> = v.frames.iter().map(|f| f.value).filter(|x| x.is_finite()).collect();
                keys.sort_by(f32::total_cmp);
                keys.dedup();
                let lo = v.min_value.or(keys.first().copied()).unwrap_or(v.default_value).min(v.default_value);
                let mut hi = v.max_value.or(keys.last().copied()).unwrap_or(v.default_value).max(v.default_value);
                if hi <= lo {
                    hi = lo + 1.;
                }
                ParameterInfo { id: v.name, min: lo, max: hi, default: v.default_value, keys }
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
    fn motions(&self) -> Vec<MotionInfo> {
        self.runtime
            .timeline_summaries()
            .into_iter()
            .map(|t| MotionInfo {
                id: t.name,
                duration_ms: t.total_ticks.filter(|t| *t > 0.).map(|t| t * 1000. / TICKS_PER_SECOND),
                looping: t.looping,
            })
            .collect()
    }
    fn parameter_value(&self, id: &str) -> Option<f32> {
        self.runtime.variable_value(id)
    }
    fn set_parameter(&mut self, id: &str, value: f32, duration_ms: f32) -> Result<()> {
        ensure!(self.runtime.variable_value(id).is_some(), "unknown E-mote variable '{id}'");
        self.runtime.set_variable(id, value, duration_ms.max(0.) * TICKS_PER_SECOND / 1000., 0.)
    }
    /// E-mote variables are not latched: timelines keep writing them, so a
    /// release has nothing to undo.
    fn release_parameter(&mut self, id: &str) -> Result<()> {
        ensure!(self.runtime.variable_value(id).is_some(), "unknown E-mote variable '{id}'");
        Ok(())
    }
    fn play(&mut self, motion: &str, options: PlayOptions) -> Result<()> {
        let info = self.motions().into_iter().find(|m| m.id == motion).with_context(|| format!("unknown E-mote timeline '{motion}'"))?;
        if (options.weight - 1.).abs() > 1e-6 {
            bail!("E-mote timelines have no play weight; use 1.0");
        }
        let mode = TimelinePlayMode::ONCE.with_looping(options.looping.unwrap_or(info.looping));
        self.runtime.play_timeline_mode(motion, mode)
    }
    fn stop(&mut self, motion: &str) -> Result<()> {
        if motion.is_empty() {
            self.runtime.stop_all_timelines()
        } else {
            ensure!(self.motions().iter().any(|m| m.id == motion), "unknown E-mote timeline '{motion}'");
            self.runtime.stop_timeline(motion)
        }
    }
    fn advance(&mut self, dt_ms: f32) -> Result<()> {
        ensure!(dt_ms.is_finite() && dt_ms >= 0., "invalid time step");
        self.pending_ms += dt_ms;
        let whole = self.pending_ms.floor();
        self.pending_ms -= whole;
        let mut left = whole as i64;
        while left > 0 {
            let step = left.min(i32::MAX as i64) as i32;
            self.runtime.progress_ms(step)?;
            left -= step as i64;
        }
        Ok(())
    }
    fn frame(&self) -> Result<PuppetFrame> {
        let mut f = self.frame_in_model_space()?;
        for p in &mut f.parts {
            for v in &mut p.positions {
                v[0] -= self.origin[0];
                v[1] -= self.origin[1];
            }
        }
        Ok(f)
    }
    fn texture_names(&self) -> Vec<String> {
        self.textures.iter().map(|r| format!("emote-resource-{r}")).collect()
    }
    fn texture_rgba(&self, index: u32) -> Option<(u32, u32, Vec<u8>)> {
        let r = self.textures.get(index as usize)?;
        let t = self.runtime.decoded_textures().get(r)?;
        Some((t.width, t.height, t.rgba.as_ref().clone()))
    }
}

/// Engine blend for a native E-mote `bm` value.
pub fn blend_for_native_mode(mode: u32) -> PartBlend {
    blend(mode)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_blend_modes_map_to_engine_blends() {
        assert_eq!(blend_for_native_mode(0), PartBlend::Normal);
        assert_eq!(blend_for_native_mode(1), PartBlend::Add);
        assert_eq!(blend_for_native_mode(2), PartBlend::Subtract);
        assert_eq!(blend_for_native_mode(5), PartBlend::Subtract);
        assert_eq!(blend_for_native_mode(3), PartBlend::Multiply);
        assert_eq!(blend_for_native_mode(4), PartBlend::Screen);
        // High nibble 0x10 (MODULATE2X) does not change the equation.
        assert_eq!(blend_for_native_mode(0x11), PartBlend::Add);
    }
    #[test]
    fn invalid_psb_fails_visibly() {
        assert!(ElunaPuppet::from_psb(&[b"not a psb"], None).is_err());
    }
}
