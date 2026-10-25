// SPDX-License-Identifier: MPL-2.0
//! Retained layers, parent transforms, clips and painter ordering. Derived from
//! siglus_rs layer.rs storage/ordering and render_math.rs projection semantics.
use crate::projection;
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    state::ClientId,
};
use glam::{Mat3, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn one() -> f32 {
    1.
}
fn up() -> [f32; 3] {
    [0., 1., 0.]
}
fn eye() -> [f32; 3] {
    [0., 0., -1000.]
}
fn fov() -> f32 {
    45.
}
fn yes() -> bool {
    true
}
fn white() -> [u8; 4] {
    [255; 4]
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpriteTransform {
    pub z: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub scale_z: f32,
    pub rotate: f32,
    pub rotate_x: f32,
    pub rotate_y: f32,
    pub pivot_x: f32,
    pub pivot_y: f32,
    pub pivot_z: f32,
    pub object_anchor: bool,
    pub texture_center_x: f32,
    pub texture_center_y: f32,
    pub camera_enabled: bool,
    pub camera_eye: [f32; 3],
    pub camera_target: [f32; 3],
    pub camera_up: [f32; 3],
    pub camera_view_angle_deg: f32,
    pub billboard: bool,
    pub culling: bool,
}
impl Default for SpriteTransform {
    fn default() -> Self {
        Self {
            z: 0.,
            scale_x: one(),
            scale_y: one(),
            scale_z: one(),
            rotate: 0.,
            rotate_x: 0.,
            rotate_y: 0.,
            pivot_x: 0.,
            pivot_y: 0.,
            pivot_z: 0.,
            object_anchor: false,
            texture_center_x: 0.,
            texture_center_y: 0.,
            camera_enabled: false,
            camera_eye: eye(),
            camera_target: [0.; 3],
            camera_up: up(),
            camera_view_angle_deg: fov(),
            billboard: false,
            culling: false,
        }
    }
}
impl SpriteTransform {
    pub fn validate(&self) -> Result<()> {
        let values = [
            self.z,
            self.scale_x,
            self.scale_y,
            self.scale_z,
            self.rotate,
            self.rotate_x,
            self.rotate_y,
            self.pivot_x,
            self.pivot_y,
            self.pivot_z,
            self.texture_center_x,
            self.texture_center_y,
            self.camera_view_angle_deg,
        ];
        ensure!(
            values
                .iter()
                .chain(&self.camera_eye)
                .chain(&self.camera_target)
                .chain(&self.camera_up)
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "invalid sprite transform"
        );
        ensure!(
            self.scale_x.abs() > 1e-6 && self.scale_y.abs() > 1e-6 && self.scale_z.abs() > 1e-6,
            "singular sprite transform"
        );
        if self.camera_enabled {
            let d = Vec3::from_array(self.camera_target) - Vec3::from_array(self.camera_eye);
            ensure!(
                d.cross(Vec3::from_array(self.camera_up)).length_squared() > 1e-12
                    && (1.0..179.).contains(&self.camera_view_angle_deg),
                "invalid sprite camera"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    /// Source-over alpha blending.
    #[default]
    Normal,
    /// Additive.
    Add,
    /// Subtractive.
    Subtract,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
}
/// Per-layer 2D filter stack with the stage order and parameter meaning of
/// the Siglus/tona3 sprite effect (ADR 0021). Color math runs on 8-bit
/// gamma-encoded values like the original: distortion sampling → tint/alpha →
/// tone curve → invert → mono → bright → dark → color rate → add → mask →
/// gray-as-alpha → alpha test.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerFilter {
    /// Mix toward grayscale (0..1).
    #[serde(default)]
    pub mono: f32,
    /// Mix toward the inverted color (0..1).
    #[serde(default)]
    pub invert: f32,
    /// Mix toward white (0..1).
    #[serde(default)]
    pub bright: f32,
    /// Mix toward black (0..1).
    #[serde(default)]
    pub dark: f32,
    /// Mix toward `color` (0..1).
    #[serde(default)]
    pub color_rate: f32,
    /// Target color of `color_rate` (0..1 per channel).
    #[serde(default)]
    pub color: [f32; 3],
    /// Signed per-channel addition (-1..1).
    #[serde(default)]
    pub add: [f32; 3],
    /// Tone-curve LUT lookup, or `null`.
    #[serde(default)]
    pub tone_curve: Option<ToneCurve>,
    /// Grayscale mask image multiplied into alpha, or `null`.
    #[serde(default)]
    pub mask: Option<LayerMask>,
    /// Discard pixels whose alpha is below 1/255.
    #[serde(default)]
    pub alpha_test: bool,
    /// Source-sampling distortion, or `null`.
    #[serde(default)]
    pub distortion: Option<Distortion>,
}
/// Tone-curve lookup: each channel indexes column `value` of `row` in a
/// LUT image (e.g. 256×N), after mixing `saturation` toward mono.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToneCurve {
    /// LUT image asset.
    pub asset: String,
    /// LUT row to sample.
    #[serde(default)]
    pub row: f32,
    /// Mix toward mono before the lookup (0..1).
    #[serde(default)]
    pub saturation: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerMask {
    /// Grayscale mask image asset.
    pub asset: String,
    /// Replace alpha with the final gray level (Siglus mask mode 1).
    #[serde(default)]
    pub gray_alpha: bool,
}
/// Source-sampling effects (Siglus wipe modes 1–6), in texture UV space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Distortion {
    /// Cells of `cut` UV width; height = cut × `aspect`.
    Mosaic {
        cut: f32,
        #[serde(default = "one_f")]
        aspect: f32,
    },
    /// Sinusoidal row (`vertical: false`) or column offset driven by `progress`.
    Raster {
        #[serde(default)]
        vertical: bool,
        fractions: f32,
        waves: f32,
        power: f32,
        progress: f32,
    },
    /// Radial blur toward `center` (UV), 10 weighted taps.
    RadialBlur {
        center: [f32; 2],
        power: f32,
        coeff: f32,
    },
    /// Multiply alpha by `fade` where luminance ≤ `threshold` (or ≥ when `invert`).
    ThresholdFade {
        fade: f32,
        threshold: f32,
        #[serde(default)]
        invert: bool,
    },
}
fn one_f() -> f32 {
    1.
}
impl LayerFilter {
    pub fn validate(&self) -> Result<()> {
        let unit = |v: f32| v.is_finite() && (0.0..=1.).contains(&v);
        ensure!(
            [
                self.mono,
                self.invert,
                self.bright,
                self.dark,
                self.color_rate
            ]
            .into_iter()
            .all(unit)
                && self.color.into_iter().all(unit)
                && self
                    .add
                    .iter()
                    .all(|v| v.is_finite() && (-1.0..=1.).contains(v)),
            "invalid layer filter color parameters"
        );
        if let Some(t) = &self.tone_curve {
            VfsPath::new(Mount::Asset, &t.asset).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(unit(t.row) && unit(t.saturation), "invalid tone curve");
        }
        if let Some(m) = &self.mask {
            VfsPath::new(Mount::Asset, &m.asset).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        if let Some(d) = self.distortion {
            let f = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
            ensure!(
                match d {
                    Distortion::Mosaic { cut, aspect } =>
                        f(cut, 1e-5, 1.) && f(aspect, 1e-3, 1000.),
                    Distortion::Raster {
                        fractions,
                        waves,
                        power,
                        progress,
                        ..
                    } =>
                        f(fractions, 1., 10_000.)
                            && f(waves, -1000., 1000.)
                            && f(power, -1000., 1000.)
                            && f(progress, 0., 1.),
                    Distortion::RadialBlur {
                        center,
                        power,
                        coeff,
                    } =>
                        center.iter().all(|c| f(*c, -10., 10.))
                            && f(power, 0., 10_000.)
                            && f(coeff, 0., 1000.),
                    Distortion::ThresholdFade {
                        fade, threshold, ..
                    } => f(fade, 0., 1.) && f(threshold, 0., 1.),
                },
                "invalid layer distortion"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Paint {
    /// Solid RGBA8 fill.
    Solid {
        #[serde(default = "white")]
        color: [u8; 4],
    },
    /// Image asset with an RGBA8 tint.
    Image {
        asset: String,
        #[serde(default = "white")]
        tint: [u8; 4],
    },
    /// Text with an explicit project font alias, pixel size, color and layout.
    Text {
        text: String,
        font: String,
        size: u16,
        #[serde(default = "white")]
        color: [u8; 4],
        #[serde(default = "layout")]
        layout: String,
    },
    /// 2D deformable puppet (ADR 0029): an engine puppet rig JSON asset,
    /// fitted into the layer rect (aspect preserved).
    Puppet {
        asset: String,
        #[serde(default = "white")]
        tint: [u8; 4],
    },
}
fn layout() -> String {
    crysirror_core::service::SIMPLE_TEXT_LAYOUT.into()
}
impl Paint {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Image { asset, .. } => {
                VfsPath::new(Mount::Asset, asset).map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            Self::Puppet { asset, .. } => {
                VfsPath::new(Mount::Asset, asset).map_err(|e| anyhow::anyhow!("{e}"))?;
                ensure!(
                    !asset.to_ascii_lowercase().ends_with(".psb"),
                    "puppet layers take engine rigs, not PSB (import it first)"
                );
            }
            Self::Text {
                text,
                font,
                size,
                layout,
                ..
            } => ensure!(
                text.len() <= 1024 * 1024
                    && !font.is_empty()
                    && (1..=512).contains(size)
                    && layout == crysirror_core::service::SIMPLE_TEXT_LAYOUT,
                "unsupported text paint/profile"
            ),
            Self::Solid { .. } => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    /// Client-local layer id.
    pub id: u32,
    /// Parent layer id; transform, visibility, opacity and clip are inherited.
    #[serde(default)]
    pub parent: Option<u32>,
    /// Rectangle [x, y, w, h] in canvas pixels.
    pub rect: [f32; 4],
    /// Solid fill, image or text.
    pub paint: Paint,
    /// Pivot, anchor, rotation, scale and z.
    #[serde(default)]
    pub transform: SpriteTransform,
    /// Explicit [major, minor] draw order.
    #[serde(default)]
    pub order: [i32; 2],
    /// Clip rectangle [x, y, w, h], or `null`.
    #[serde(default)]
    pub clip: Option<[f32; 4]>,
    /// Opacity 0..255, multiplied down the tree.
    #[serde(default = "alpha")]
    pub opacity: u8,
    /// Inherited visibility.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Blend mode.
    #[serde(default)]
    pub blend: Blend,
    /// Source UV rectangle [u0, v0, u1, v1] of an image paint.
    #[serde(default)]
    pub uv: Option<[f32; 4]>,
    /// Siglus-style 2D filter stack (ADR 0021).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<LayerFilter>,
}
fn alpha() -> u8 {
    255
}
#[derive(Clone, Debug)]
pub struct Primitive {
    pub owner: u32,
    pub id: u32,
    pub sub: u8,
    pub order: [i32; 2],
    pub size: [u32; 2],
    pub paint: Paint,
    pub quad: [[f32; 4]; 4],
    pub clip: Option<[f32; 4]>,
    pub opacity: u8,
    pub blend: Blend,
    pub uv: [f32; 4],
    pub filter: Option<LayerFilter>,
    /// Evaluated puppet meshes of a `Paint::Puppet` layer (ADR 0029).
    pub puppet: Option<Arc<crysirror_puppet::PuppetFrame>>,
}
/// Commands to a puppet layer; parameter and motion names are checked
/// against the rig when the runtime applies them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PuppetCommand {
    /// Ease a parameter to `value` and hold it (overrides motions).
    SetParameter {
        parameter: String,
        value: f32,
        #[serde(default)]
        duration_ms: f32,
    },
    /// Return a parameter to motion/physics control.
    ReleaseParameter { parameter: String },
    /// Start a motion (restarts it if playing).
    Play {
        motion: String,
        #[serde(default)]
        looping: Option<bool>,
        #[serde(default = "one")]
        weight: f32,
    },
    /// Fade a motion out; an empty name stops all motions.
    Stop {
        #[serde(default)]
        motion: String,
    },
}
impl PuppetCommand {
    pub fn validate(&self) -> Result<()> {
        let ok = match self {
            Self::SetParameter {
                parameter,
                value,
                duration_ms,
            } => {
                !parameter.is_empty()
                    && value.is_finite()
                    && duration_ms.is_finite()
                    && (0.0..=600_000.).contains(duration_ms)
            }
            Self::ReleaseParameter { parameter } => !parameter.is_empty(),
            Self::Play { motion, weight, .. } => {
                !motion.is_empty() && weight.is_finite() && (0.0..=1.).contains(weight)
            }
            Self::Stop { .. } => true,
        };
        ensure!(ok, "invalid puppet command");
        Ok(())
    }
}
/// Saved puppet state of one layer plus commands not yet applied.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PuppetSlot {
    /// Rig asset the state belongs to; a different rig restarts it.
    pub asset: String,
    #[serde(default)]
    pub state: Option<crysirror_puppet::PuppetState>,
    #[serde(default)]
    pub pending: Vec<PuppetCommand>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSystem {
    clients: BTreeMap<u32, BTreeMap<u32, LayerSpec>>,
    /// Puppet layers' states by client and layer id (ADR 0029).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    puppets: BTreeMap<u32, BTreeMap<u32, PuppetSlot>>,
    /// Last evaluated puppet meshes; derived, rebuilt by the runtime.
    #[serde(skip)]
    puppet_frames: BTreeMap<(u32, u32), Arc<crysirror_puppet::PuppetFrame>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transition: Option<ActiveTransition>,
    #[serde(default)]
    transition_serial: u64,
}

/// Full-screen transition from the previously presented frame to the current
/// one (ADR 0022). Effect codes and option meanings are the Siglus `WIPE`
/// table, composed by the same shader arithmetic as siglus_rs `wipe.wgsl`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionSpec {
    /// Transition effect.
    pub effect: TransitionEffect,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Progress curve (Siglus speed modes 0-3).
    #[serde(default)]
    pub easing: Easing,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitionEffect {
    /// Plain cross-dissolve.
    Crossfade,
    /// Grayscale mask image: bright pixels reveal first; `fade` 0..=8 is the
    /// Siglus soft-edge step (0 = hard edge).
    Mask {
        asset: String,
        #[serde(default)]
        fade: u8,
    },
    /// direction 0 down, 1 up, 2 right, 3 left (Siglus type 200, both scenes move).
    Slide { direction: u8 },
    /// Mosaic out of the old scene and into the new one (type 230).
    Mosaic { size: u32 },
    /// A raw Siglus wipe type with its option list. Pattern types (5..=199)
    /// without `mask` use the original generated masks.
    Siglus {
        code: i32,
        #[serde(default)]
        options: Vec<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mask: Option<String>,
    },
}
/// Siglus wipe speed modes 0-3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    /// Siglus speed mode 0.
    #[default]
    Linear,
    /// Mode 1: p².
    EaseIn,
    /// Mode 2: 1 − (1 − p)².
    EaseOut,
    /// Mode 3: smoothstep.
    Smooth,
}
impl Easing {
    /// siglus_rs `runtime/wipe.rs::eased_progress`.
    pub fn apply(self, raw: f32) -> f32 {
        let p = raw.clamp(0., 1.);
        match self {
            Easing::Linear => p,
            Easing::EaseIn => p * p,
            Easing::EaseOut => 1. - (1. - p) * (1. - p),
            Easing::Smooth => p * p * (3. - 2. * p),
        }
    }
}
/// Siglus wipe types composed by the shader; page curls (300/301) are not.
pub fn transition_code_supported(code: i32) -> bool {
    matches!(code, 0..=2 | 5..=199 | 200 | 210..=215 | 220 | 221 | 230 | 231 | 240..=243 | 900 | 901)
}
/// Types whose mask comes from `wipe_mask::generate` when none is given.
pub fn transition_code_uses_generated_mask(code: i32) -> bool {
    (5..200).contains(&code) && code != 50
}
impl TransitionSpec {
    /// (code, 16 options, explicit mask asset).
    pub fn resolve(&self) -> (i32, [i32; 16], Option<String>) {
        let mut o = [0; 16];
        match &self.effect {
            TransitionEffect::Crossfade => (0, o, None),
            TransitionEffect::Mask { asset, fade } => {
                o[0] = i32::from(*fade);
                (900, o, Some(asset.clone()))
            }
            TransitionEffect::Slide { direction } => {
                o[0] = i32::from(*direction);
                o[1] = 1;
                o[2] = 1;
                (200, o, None)
            }
            TransitionEffect::Mosaic { size } => {
                o[0] = *size as i32;
                (230, o, None)
            }
            TransitionEffect::Siglus {
                code,
                options,
                mask,
            } => {
                for (d, s) in o.iter_mut().zip(options) {
                    *d = *s;
                }
                (*code, o, mask.clone())
            }
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=600_000).contains(&self.duration_ms),
            "transition duration must be 1..=600000 ms"
        );
        match &self.effect {
            TransitionEffect::Mask { asset, fade } => {
                ensure!(*fade <= 8 && !asset.is_empty(), "invalid mask transition");
            }
            TransitionEffect::Slide { direction } => {
                ensure!(*direction < 4, "slide direction 0..=3")
            }
            TransitionEffect::Mosaic { size } => {
                ensure!((1..=1024).contains(size), "mosaic size 1..=1024")
            }
            TransitionEffect::Siglus {
                code,
                options,
                mask,
            } => {
                ensure!(
                    transition_code_supported(*code),
                    "Siglus wipe type {code} is not supported by the compositor"
                );
                ensure!(
                    options.len() <= 15,
                    "at most 15 Siglus wipe options (slot 15 is the seed)"
                );
                ensure!(
                    mask.as_ref().map_or(true, |m| !m.is_empty()),
                    "empty mask asset"
                );
            }
            TransitionEffect::Crossfade => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveTransition {
    pub owner: u32,
    pub serial: u64,
    pub spec: TransitionSpec,
    pub elapsed_ms: u64,
}
/// What the renderer needs for this frame's transition.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionFrame {
    pub owner: u32,
    /// Changes for every started transition; the renderer freezes the
    /// previously presented frame when it sees a new serial.
    pub serial: u64,
    pub code: i32,
    pub options: [i32; 16],
    pub progress: f32,
    pub mask: Option<String>,
    pub generated_mask: bool,
}
pub fn rect_valid(rect: [f32; 4]) -> bool {
    rect.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.)
        && rect[2] > 0.
        && rect[3] > 0.
        && rect[2] <= 16384.
        && rect[3] <= 16384.
}
pub fn intersect(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let r = (a[0] + a[2]).min(b[0] + b[2]);
    let d = (a[1] + a[3]).min(b[1] + b[3]);
    (r > x && d > y).then_some([x, y, r - x, d - y])
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerCommand {
    Put {
        layer: LayerSpec,
    },
    Remove {
        id: u32,
    },
    SetTransform {
        id: u32,
        transform: SpriteTransform,
    },
    SetVisible {
        id: u32,
        visible: bool,
    },
    SetOpacity {
        id: u32,
        opacity: u8,
    },
    SetOrder {
        id: u32,
        order: [i32; 2],
    },
    SetClip {
        id: u32,
        clip: Option<[f32; 4]>,
    },
    SetFilter {
        id: u32,
        filter: Option<LayerFilter>,
    },
    /// Drive a puppet layer (ADR 0029).
    Puppet {
        id: u32,
        command: PuppetCommand,
    },
    /// Start a screen transition owned by this client.
    Transition {
        transition: TransitionSpec,
    },
    /// Finish this client's transition immediately (skip).
    EndTransition,
}
impl LayerSystem {
    pub fn apply_batch(&mut self, client: ClientId, commands: &[LayerCommand]) -> Result<()> {
        ensure!(commands.len() <= 4096, "layer command limit");
        let mut staged = self.clone();
        for command in commands {
            match command {
                LayerCommand::Transition { transition } => {
                    transition.validate()?;
                    ensure!(
                        staged
                            .transition
                            .as_ref()
                            .map_or(true, |t| t.owner == client.0),
                        "another client owns the active transition"
                    );
                    staged.transition_serial += 1;
                    staged.transition = Some(ActiveTransition {
                        owner: client.0,
                        serial: staged.transition_serial,
                        spec: transition.clone(),
                        elapsed_ms: 0,
                    });
                    continue;
                }
                LayerCommand::Puppet { id, command } => {
                    command.validate()?;
                    let layer = staged
                        .clients
                        .get(&client.0)
                        .and_then(|o| o.get(id))
                        .context("layer missing")?;
                    let Paint::Puppet { asset, .. } = &layer.paint else {
                        bail!("layer {id} is not a puppet")
                    };
                    let asset = asset.clone();
                    let slot = staged
                        .puppets
                        .entry(client.0)
                        .or_default()
                        .entry(*id)
                        .or_default();
                    slot.asset = asset;
                    ensure!(
                        slot.pending.len() < 1024,
                        "too many pending puppet commands"
                    );
                    slot.pending.push(command.clone());
                    continue;
                }
                LayerCommand::EndTransition => {
                    ensure!(
                        staged
                            .transition
                            .as_ref()
                            .map_or(true, |t| t.owner == client.0),
                        "another client owns the active transition"
                    );
                    staged.transition = None;
                    continue;
                }
                _ => {}
            }
            let owner = staged.clients.entry(client.0).or_default();
            match command {
                LayerCommand::Put { layer } => {
                    owner.insert(layer.id, layer.clone());
                }
                LayerCommand::Remove { id } => {
                    ensure!(owner.contains_key(id), "layer missing");
                    let mut remove = BTreeSet::from([*id]);
                    loop {
                        let n = remove.len();
                        for (id, s) in owner.iter() {
                            if s.parent.is_some_and(|p| remove.contains(&p)) {
                                remove.insert(*id);
                            }
                        }
                        if remove.len() == n {
                            break;
                        }
                    }
                    owner.retain(|id, _| !remove.contains(id));
                }
                other => {
                    let id = match other {
                        LayerCommand::SetTransform { id, .. }
                        | LayerCommand::SetVisible { id, .. }
                        | LayerCommand::SetOpacity { id, .. }
                        | LayerCommand::SetOrder { id, .. }
                        | LayerCommand::SetClip { id, .. }
                        | LayerCommand::SetFilter { id, .. } => id,
                        _ => unreachable!(),
                    };
                    let layer = owner.get_mut(id).context("layer missing")?;
                    match other {
                        LayerCommand::SetTransform { transform, .. } => {
                            layer.transform = transform.clone()
                        }
                        LayerCommand::SetVisible { visible, .. } => layer.visible = *visible,
                        LayerCommand::SetOpacity { opacity, .. } => layer.opacity = *opacity,
                        LayerCommand::SetOrder { order, .. } => layer.order = *order,
                        LayerCommand::SetClip { clip, .. } => layer.clip = *clip,
                        LayerCommand::SetFilter { filter, .. } => layer.filter = filter.clone(),
                        _ => unreachable!(),
                    }
                }
            }
        }
        staged.sync_puppets();
        staged.validate()?;
        *self = staged;
        Ok(())
    }
    /// Keeps exactly one slot per puppet layer; a changed rig restarts it.
    fn sync_puppets(&mut self) {
        let clients = &self.clients;
        for (client, slots) in self.puppets.iter_mut() {
            slots.retain(|id, slot| {
                matches!(clients.get(client).and_then(|o| o.get(id)).map(|l| &l.paint),
                    Some(Paint::Puppet { asset, .. }) if *asset == slot.asset)
            });
        }
        for (client, owner) in &self.clients {
            for (id, layer) in owner {
                if let Paint::Puppet { asset, .. } = &layer.paint {
                    self.puppets
                        .entry(*client)
                        .or_default()
                        .entry(*id)
                        .or_insert_with(|| PuppetSlot {
                            asset: asset.clone(),
                            ..Default::default()
                        });
                }
            }
        }
        self.puppets.retain(|_, s| !s.is_empty());
        let puppets = &self.puppets;
        self.puppet_frames
            .retain(|(c, id), _| puppets.get(c).is_some_and(|s| s.contains_key(id)));
    }
    /// (client, layer id, rig asset) of every puppet layer.
    pub fn puppet_layers(&self) -> Vec<(u32, u32, String)> {
        self.puppets
            .iter()
            .flat_map(|(c, s)| s.iter().map(|(id, slot)| (*c, *id, slot.asset.clone())))
            .collect()
    }
    pub fn puppet_slot(&self, client: u32, id: u32) -> Option<&PuppetSlot> {
        self.puppets.get(&client)?.get(&id)
    }
    /// Stores the runtime's evaluation: new state, consumed commands, mesh.
    pub fn set_puppet(
        &mut self,
        client: u32,
        id: u32,
        state: crysirror_puppet::PuppetState,
        frame: Arc<crysirror_puppet::PuppetFrame>,
    ) -> Result<()> {
        let slot = self
            .puppets
            .get_mut(&client)
            .and_then(|s| s.get_mut(&id))
            .context("puppet layer missing")?;
        slot.state = Some(state);
        slot.pending.clear();
        self.puppet_frames.insert((client, id), frame);
        Ok(())
    }
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients
            .keys()
            .copied()
            .chain(self.transition.as_ref().map(|t| t.owner))
    }
    pub fn transition(&self) -> Option<&ActiveTransition> {
        self.transition.as_ref()
    }
    /// Advance the transition clock; it ends (and is removed) at its duration.
    pub fn tick(&mut self, delta_ms: u64) {
        if let Some(t) = &mut self.transition {
            t.elapsed_ms = t.elapsed_ms.saturating_add(delta_ms);
            if t.elapsed_ms >= t.spec.duration_ms {
                self.transition = None;
            }
        }
    }
    pub fn transition_frame(&self) -> Option<TransitionFrame> {
        let t = self.transition.as_ref()?;
        let (code, mut options, mask) = t.spec.resolve();
        // Slot 15 is the per-wipe random seed (types 242/243 and random masks).
        options[15] = (t.serial as u32 as i32) & 0x7fff_ffff;
        let raw = t.elapsed_ms as f32 / t.spec.duration_ms as f32;
        Some(TransitionFrame {
            owner: t.owner,
            serial: t.serial,
            code,
            options,
            progress: t.spec.easing.apply(raw),
            generated_mask: mask.is_none() && transition_code_uses_generated_mask(code),
            mask,
        })
    }
    pub fn install(&mut self, client: ClientId, specs: &[LayerSpec]) -> Result<()> {
        let mut staged = self.clone();
        let owner = staged.clients.entry(client.0).or_default();
        for s in specs {
            ensure!(
                owner.insert(s.id, s.clone()).is_none(),
                "duplicate layer id"
            );
        }
        staged.sync_puppets();
        staged.validate()?;
        *self = staged;
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(t) = &self.transition {
            t.spec.validate()?;
            ensure!(t.serial <= self.transition_serial, "transition serial");
        }
        ensure!(
            self.clients.len() <= 64
                && self.clients.values().map(BTreeMap::len).sum::<usize>() <= 4096,
            "layer count"
        );
        for (client, slots) in &self.puppets {
            for (id, slot) in slots {
                ensure!(
                    matches!(self.clients.get(client).and_then(|o| o.get(id)).map(|l| &l.paint),
                        Some(Paint::Puppet { asset, .. }) if *asset == slot.asset),
                    "puppet state without its puppet layer"
                );
                for c in &slot.pending {
                    c.validate()?;
                }
            }
        }
        for owner in self.clients.values() {
            for (id, s) in owner {
                ensure!(
                    *id == s.id && rect_valid(s.rect),
                    "invalid layer identity/bounds"
                );
                s.paint.validate()?;
                s.transform.validate()?;
                if let Some(clip) = s.clip {
                    ensure!(rect_valid(clip), "invalid layer clip");
                }
                if let Some(f) = &s.filter {
                    f.validate()?;
                }
                if let Some(uv) = s.uv {
                    ensure!(
                        uv.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                            && uv[2] > uv[0]
                            && uv[3] > uv[1],
                        "invalid source UV"
                    );
                }
                let mut seen = BTreeSet::from([*id]);
                let mut p = s.parent;
                while let Some(id) = p {
                    ensure!(
                        seen.len() <= 64 && seen.insert(id),
                        "layer parent cycle/depth"
                    );
                    p = owner.get(&id).context("layer parent missing")?.parent;
                }
            }
        }
        Ok(())
    }
    pub fn set_visible(&mut self, client: ClientId, id: u32, visible: bool) -> Result<()> {
        self.clients
            .get_mut(&client.0)
            .and_then(|c| c.get_mut(&id))
            .context("layer missing")?
            .visible = visible;
        Ok(())
    }
    pub fn frame(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        self.validate()?;
        ensure!(
            canvas.iter().all(|v| *v > 0 && *v <= 8192),
            "layer canvas bounds"
        );
        let mut out = Vec::new();
        for (client, owner) in &self.clients {
            for spec in owner.values() {
                let mut chain = vec![spec];
                let mut p = spec.parent;
                while let Some(id) = p {
                    let parent = owner.get(&id).context("layer parent missing")?;
                    chain.push(parent);
                    p = parent.parent;
                }
                chain.reverse();
                if chain.iter().any(|s| !s.visible || s.opacity == 0) {
                    continue;
                }
                let mut matrix = Mat3::IDENTITY;
                let mut clip = Some([0., 0., canvas[0] as f32, canvas[1] as f32]);
                let mut opacity = 255u32;
                for s in chain.iter().take(chain.len() - 1) {
                    let t = &s.transform;
                    ensure!(
                        !t.camera_enabled && !t.billboard && t.rotate_x == 0. && t.rotate_y == 0.,
                        "3D layer parents require a world hierarchy"
                    );
                    matrix *= affine(s);
                    opacity = opacity * u32::from(s.opacity) / 255;
                    if let Some(c) = s.clip {
                        clip = clip.and_then(|old| intersect(old, transformed_bounds(matrix, c)));
                    }
                }
                if let Some(c) = spec.clip {
                    let projected = projection::sprite_quad_geometry_rect(
                        &spec.transform,
                        spec.rect[0],
                        spec.rect[1],
                        c[0],
                        c[1],
                        c[0] + c[2],
                        c[1] + c[3],
                        canvas[0] as f32,
                        canvas[1] as f32,
                    );
                    clip = projected.and_then(|g| {
                        let points = g
                            .projected
                            .map(|p| matrix.transform_point2(Vec2::new(p.x, p.y)));
                        clip.and_then(|old| intersect(old, point_bounds(points)))
                    });
                }
                let Some(clip) = clip else {
                    continue;
                };
                let Some(geometry) = projection::sprite_quad_geometry_rect(
                    &spec.transform,
                    spec.rect[0],
                    spec.rect[1],
                    0.,
                    0.,
                    spec.rect[2],
                    spec.rect[3],
                    canvas[0] as f32,
                    canvas[1] as f32,
                ) else {
                    continue;
                };
                let quad = geometry.projected.map(|p| {
                    let v = matrix.transform_point2(Vec2::new(p.x, p.y));
                    [v.x, v.y, p.depth, p.clip_w]
                });
                ensure!(
                    quad.iter().flatten().all(|v| v.is_finite()),
                    "non-finite layer geometry"
                );
                out.push(Primitive {
                    owner: *client,
                    id: spec.id,
                    sub: 0,
                    order: spec.order,
                    size: [spec.rect[2].ceil() as u32, spec.rect[3].ceil() as u32],
                    paint: spec.paint.clone(),
                    quad,
                    clip: Some(clip),
                    opacity: (opacity * u32::from(spec.opacity) / 255) as u8,
                    blend: spec.blend,
                    uv: spec.uv.unwrap_or([0., 0., 1., 1.]),
                    filter: spec.filter.clone(),
                    puppet: self.puppet_frames.get(&(*client, spec.id)).cloned(),
                });
            }
        }
        out.sort_by_key(|p| (p.order, p.owner, p.id, p.sub));
        Ok(out)
    }
}
pub fn rectangle(
    owner: u32,
    id: u32,
    sub: u8,
    rect: [f32; 4],
    paint: Paint,
    clip: Option<[f32; 4]>,
    order: [i32; 2],
) -> Primitive {
    let [x, y, w, h] = rect;
    Primitive {
        owner,
        id,
        sub,
        order,
        size: [w.ceil().max(1.) as u32, h.ceil().max(1.) as u32],
        paint,
        quad: [
            [x, y, 0., 1.],
            [x + w, y, 0., 1.],
            [x + w, y + h, 0., 1.],
            [x, y + h, 0., 1.],
        ],
        clip,
        opacity: 255,
        blend: Blend::Normal,
        uv: [0., 0., 1., 1.],
        filter: None,
        puppet: None,
    }
}

// Clips are axis-aligned scissors; rotating a clip uses its four-corner AABB.
fn transformed_bounds(m: Mat3, r: [f32; 4]) -> [f32; 4] {
    let corners = [
        [r[0], r[1]],
        [r[0] + r[2], r[1]],
        [r[0] + r[2], r[1] + r[3]],
        [r[0], r[1] + r[3]],
    ]
    .map(|p| m.transform_point2(Vec2::from_array(p)));
    let min = corners
        .iter()
        .copied()
        .fold(Vec2::splat(f32::INFINITY), Vec2::min);
    let max = corners
        .iter()
        .copied()
        .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
    [min.x, min.y, max.x - min.x, max.y - min.y]
}

fn affine(s: &LayerSpec) -> Mat3 {
    let t = &s.transform;
    let (anchor, offset) = if t.object_anchor {
        (
            Vec2::new(s.rect[0], s.rect[1]),
            Vec2::new(
                t.texture_center_x + t.pivot_x,
                t.texture_center_y + t.pivot_y,
            ),
        )
    } else {
        (
            Vec2::new(s.rect[0] + t.pivot_x, s.rect[1] + t.pivot_y),
            Vec2::new(t.pivot_x, t.pivot_y),
        )
    };
    Mat3::from_translation(anchor)
        * Mat3::from_angle(t.rotate)
        * Mat3::from_scale(Vec2::new(t.scale_x, t.scale_y))
        * Mat3::from_translation(-offset)
}
fn point_bounds(corners: [Vec2; 4]) -> [f32; 4] {
    let min = corners
        .iter()
        .copied()
        .fold(Vec2::splat(f32::INFINITY), Vec2::min);
    let max = corners
        .iter()
        .copied()
        .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
    [min.x, min.y, max.x - min.x, max.y - min.y]
}
