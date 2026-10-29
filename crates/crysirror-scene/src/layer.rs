// SPDX-License-Identifier: MPL-2.0
//! Retained layers, parent transforms, clips and painter ordering. Derived from
//! siglus_rs layer.rs storage/ordering and render_math.rs projection semantics.
mod crossfade;
pub use crossfade::{CompositeFrame, RuleWipe, SubtreeCrossfade};
use crate::projection;
use anyhow::{Context, Result, bail, ensure};
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
    /// Source-atop: drawn only where the destination has alpha, which it
    /// keeps (KiriKiri holdAlpha draws). Meaningful in isolated canvas
    /// snapshots, where the destination is the surface's own content.
    Atop,
    /// Photoshop multiply keeping the destination alpha (KiriKiri
    /// omPsMultiplicative with holdAlpha): D·(1 - a + a·S).
    AtopMultiply,
    /// Photoshop screen keeping the destination alpha (omPsScreen with
    /// holdAlpha): D·(1 - a·S) + a·S·Da.
    AtopScreen,
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

/// A glyph with an explicit baseline in its containing layer's local pixels.
/// Layout, shaping and reveal policy remain with the producer; rendering is an
/// engine service. Optional styles override the containing run's defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionedGlyph {
    pub character: char,
    pub baseline: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
}

/// Unit of an explicitly positioned run. Each item contains one Unicode
/// scalar; this deliberately does not reinterpret native scalar timing as
/// grapheme timing. Other segmentation modes must be added explicitly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlyphRevealUnit {
    #[default]
    UnicodeScalar,
}

/// Engine-owned positioned-text clock. Layout, markup and wait policies are
/// callers' concerns; no VM needs its own generic typewriter timer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphReveal {
    #[serde(default)]
    pub unit: GlyphRevealUnit,
    /// Milliseconds per positioned scalar. Zero reveals the run instantly.
    pub interval_ms: u64,
    /// Prefix already visible when new text is appended to a page.
    #[serde(default)]
    pub initial_visible: usize,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default)]
    pub forced_complete: bool,
    #[serde(default)]
    pub paused: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphRevealStatus {
    pub visible: usize,
    pub total: usize,
    pub complete: bool,
}
impl GlyphReveal {
    pub fn validate(&self, total: usize) -> Result<()> {
        ensure!(
            self.initial_visible <= total && self.interval_ms <= 86_400_000,
            "invalid glyph reveal prefix/interval"
        );
        Ok(())
    }
    pub fn visible_count(&self, total: usize) -> usize {
        if self.forced_complete || self.interval_ms == 0 {
            return total;
        }
        self.initial_visible
            .saturating_add(
                usize::try_from(self.elapsed_ms / self.interval_ms).unwrap_or(usize::MAX),
            )
            .min(total)
    }
    pub fn status(&self, total: usize) -> GlyphRevealStatus {
        let visible = self.visible_count(total);
        GlyphRevealStatus {
            visible,
            total,
            complete: visible == total,
        }
    }
    fn advance(&mut self, total: usize, delta_ms: u64) {
        if !self.paused && !self.status(total).complete {
            let duration = (total.saturating_sub(self.initial_visible) as u64)
                .saturating_mul(self.interval_ms);
            self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms).min(duration);
        }
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
    /// Retained explicit glyph positions, rendered as independent cached glyph
    /// quads. No VM renderer or flattened text/screen image is involved.
    GlyphRun {
        font: String,
        size: f32,
        #[serde(default = "white")]
        color: [u8; 4],
        #[serde(default)]
        bold: bool,
        glyphs: Vec<PositionedGlyph>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reveal: Option<GlyphReveal>,
        /// Blurred coverage (text shadows/edges): each glyph's coverage is
        /// spread by `radius` and scaled by `level`/256 (KiriKiri drawText).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blur: Option<GlyphBlur>,
    },
    /// Frozen, source-local static scene, rasterized once at `size` before the
    /// outer Layer's transform, opacity, UV crop and filter are applied.
    /// Inputs contain no nested snapshots, external parents or dynamic state.
    Snapshot {
        size: [u32; 2],
        /// Shared, immutable frozen tree (cheap to clone with the system).
        layers: Arc<Vec<LayerSpec>>,
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
                    && matches!(layout.as_str(), crysirror_core::service::SIMPLE_TEXT_LAYOUT | crysirror_core::service::UNWRAPPED_TEXT_LAYOUT),
                "unsupported text paint/profile"
            ),
            Self::GlyphRun {
                font,
                size,
                glyphs,
                reveal,
                blur,
                ..
            } => {
                ensure!(
                    !font.is_empty() && font.len() <= 256 && !font.chars().any(char::is_control),
                    "invalid glyph-run font alias"
                );
                ensure!(blur.is_none_or(|b| b.radius <= 64), "glyph blur radius must be 0..=64");
                let valid_size = |s: f32| s.is_finite() && (1.0..=512.0).contains(&s);
                ensure!(
                    valid_size(*size) && glyphs.len() <= 65_536,
                    "invalid glyph-run size/budget"
                );
                for glyph in glyphs {
                    ensure!(
                        !glyph.character.is_control()
                            && glyph
                                .baseline
                                .iter()
                                .all(|v| v.is_finite() && v.abs() <= 1_000_000.)
                            && glyph.size.is_none_or(valid_size),
                        "invalid positioned glyph"
                    );
                }
                if let Some(reveal) = reveal {
                    reveal.validate(glyphs.len())?;
                }
            }
            Self::Snapshot { size, layers } => validate_snapshot(*size, layers)?,
            Self::Solid { .. } => {}
        }
        Ok(())
    }
    pub fn glyph_reveal_status(&self) -> Option<GlyphRevealStatus> {
        match self {
            Self::GlyphRun { glyphs, reveal, .. } => Some(reveal.as_ref().map_or(
                GlyphRevealStatus {
                    visible: glyphs.len(),
                    total: glyphs.len(),
                    complete: true,
                },
                |r| r.status(glyphs.len()),
            )),
            _ => None,
        }
    }
}
/// Retained layers across all clients. Native UIs (a KiriKiri settings
/// screen) present many canvas surfaces whose nodes each become a layer.
pub const MAX_LAYERS: usize = 16384;
/// Snapshots may contain snapshot draws (a captured tree that already held a
/// frozen region); renderers rasterize inner payloads first. Bounded so that
/// validation and rasterization recursion stay small.
pub const MAX_SNAPSHOT_NESTING: usize = 3;
/// Levels of snapshot payloads below `layers` (0 when none holds a snapshot).
pub fn snapshot_nesting(layers: &[LayerSpec]) -> usize {
    layers
        .iter()
        .filter_map(|l| match &l.paint {
            Paint::Snapshot { layers, .. } => Some(1 + snapshot_nesting(layers)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}
fn validate_snapshot(size: [u32; 2], layers: &[LayerSpec]) -> Result<()> {
    ensure!(size.iter().all(|v| (1..=8192).contains(v)), "snapshot resolution budget");
    ensure!(!layers.is_empty() && layers.len() <= 4096, "snapshot layer budget");
    let by_id: BTreeMap<_, _> = layers.iter().map(|s| (s.id, s)).collect();
    ensure!(by_id.len() == layers.len(), "snapshot duplicate layer IDs");
    ensure!(layers.iter().filter(|s| s.parent.is_none()).count() == 1, "snapshot needs one local root");
    let parents: BTreeSet<_> = layers.iter().filter_map(|s| s.parent).collect();
    ensure!(snapshot_nesting(layers) < MAX_SNAPSHOT_NESTING, "snapshot nesting budget");
    for layer in layers {
        ensure!(!matches!(layer.paint, Paint::Puppet { .. }), "snapshot dynamic puppet is unsupported");
        ensure!(layer.paint.glyph_reveal_status().is_none_or(|r| r.complete), "snapshot dynamic glyph reveal is unsupported");
        ensure!(matches!(layer.blend, Blend::Normal | Blend::Atop | Blend::AtopMultiply | Blend::AtopScreen), "snapshot requires normal or atop layer blend");
        if parents.contains(&layer.id) {
            ensure!(layer.opacity == 255, "snapshot group opacity requires isolated composition");
        }
        let t = &layer.transform;
        ensure!(!t.camera_enabled && !t.billboard && t.rotate_x == 0. && t.rotate_y == 0., "snapshot requires 2D layer transforms");
        let mut parent = layer.parent;
        let mut depth = 0;
        while let Some(id) = parent {
            depth += 1;
            ensure!(depth < 30, "snapshot depth/cycle budget");
            parent = by_id.get(&id).context("snapshot parent outside frozen tree")?.parent;
        }
    }
    // Nested payloads are validated by this install; the nesting budget above
    // bounds that recursion.
    LayerSystem::default().install(ClientId(0), layers)?;
    Ok(())
}

/// Glyph coverage blur of a [`Paint::GlyphRun`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphBlur {
    /// Kernel radius in pixels, 1..=64 (0 only scales coverage by `level`).
    pub radius: u8,
    /// Coverage gain in 1/256 units; values above 255 amplify (thick edges).
    pub level: u16,
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
    /// Engine-isolated subtree inputs; renderer composites once, after child draws.
    pub composite: Option<Arc<CompositeFrame>>,
    pub owner: u32,
    pub id: u32,
    pub sub: u8,
    pub order: [i32; 2],
    pub size: [u32; 2],
    /// Exact local width/height before transform; `size` rounds texture extents.
    pub extent: [f32; 2],
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
    #[serde(default)]
    crossfades: BTreeMap<u32, BTreeMap<u32, SubtreeCrossfade>>,
    clients: BTreeMap<u32, BTreeMap<u32, LayerSpec>>,
    /// Puppet layers' states by client and layer id (ADR 0029).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    puppets: BTreeMap<u32, BTreeMap<u32, PuppetSlot>>,
    /// Last evaluated puppet meshes; derived, rebuilt by the runtime.
    #[serde(skip)]
    puppet_frames: BTreeMap<(u32, u32), Arc<crysirror_puppet::PuppetFrame>>,
    #[serde(skip)]
    dynamic_puppets: BTreeSet<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transition: Option<ActiveTransition>,
    #[serde(default)]
    transition_serial: u64,
    /// An open multi-batch transaction (see [`LayerSystem::transaction`]).
    #[serde(skip)]
    txn: TxnSlot,
}

/// Open transactions, innermost last. A clone of a system is outside them.
#[derive(Debug, Default)]
struct TxnSlot(Vec<LayerTxn>);
impl Clone for TxnSlot {
    fn clone(&self) -> Self {
        Self(Vec::new())
    }
}

type SideState = (
    BTreeMap<u32, BTreeMap<u32, SubtreeCrossfade>>,
    BTreeMap<u32, BTreeMap<u32, PuppetSlot>>,
    BTreeMap<(u32, u32), Arc<crysirror_puppet::PuppetFrame>>,
    BTreeSet<(u32, u32)>,
    Option<ActiveTransition>,
    u64,
);

/// Originals of every layer the transaction's batches touched (first touch
/// wins), plus the side state and owner set at its start.
#[derive(Debug)]
struct LayerTxn {
    journal: BTreeMap<(u32, u32), Option<LayerSpec>>,
    owners: BTreeSet<u32>,
    side: SideState,
}

/// Full-screen transition from the engine's previously presented scene to the
/// current one. Named engine effects have explicit contracts; the `Siglus`
/// variant separately preserves its native WIPE table (ADR 0022).
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
    /// Ordered Bayer-4 fade in logical canvas pixels. This is a shared engine
    /// effect with an independent code, not an alias for a Siglus wipe type.
    OrderedDither {
        cell: [u32; 2],
        #[serde(default)]
        diagonal: bool,
        steps: u32,
    },
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
/// Transition codes implemented by shared GPU pipelines. Engine-owned effects
/// use their own range; Siglus page curls (300/301) remain unsupported.
pub fn transition_code_supported(code: i32) -> bool {
    matches!(code, 0..=2 | 5..=199 | 200 | 210..=215 | 220 | 221 | 230 | 231 | 240..=243 | 900 | 901 | ORDERED_DITHER_TRANSITION_CODE)
}
pub const ORDERED_DITHER_TRANSITION_CODE: i32 = 1000;
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
            TransitionEffect::OrderedDither {
                cell,
                diagonal,
                steps,
            } => {
                o[0] = cell[0] as i32;
                o[1] = cell[1] as i32;
                o[2] = i32::from(*diagonal);
                o[3] = *steps as i32;
                (ORDERED_DITHER_TRANSITION_CODE, o, None)
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
            TransitionEffect::OrderedDither { cell, steps, .. } => {
                ensure!(
                    cell.iter().all(|v| (1..=8192).contains(v)) && (1..=65_536).contains(steps),
                    "ordered dither cell/step limits"
                );
            }
            TransitionEffect::Siglus {
                code,
                options,
                mask,
            } => {
                ensure!(
                    transition_code_supported(*code) && *code != ORDERED_DITHER_TRANSITION_CODE,
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
    /// Complete a positioned text run through the engine clock/state service.
    CompleteReveal {
        id: u32,
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
    /// Crossfade two equally sized sibling subtrees using isolated engine targets;
    /// with `rule`, each pixel's progress follows a grayscale rule image.
    BeginCrossfade {
        id: u32,
        source: u32,
        duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule: Option<RuleWipe>,
        /// Driven by `SetCrossfadeElapsed` instead of the engine clock.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        external: bool,
    },
    /// Progress of an `external` crossfade, in ms of its duration.
    SetCrossfadeElapsed { id: u32, elapsed_ms: u64 },
    /// Release a subtree transition after observing completion or cancellation.
    EndCrossfade { id: u32 },
    /// Finish this client's transition immediately (skip).
    EndTransition,
}
impl LayerSystem {
    /// Inspect the same client-scoped logical node used by render extraction.
    pub fn get(&self, client: ClientId, id: u32) -> Option<&LayerSpec> {
        self.clients.get(&client.0)?.get(&id)
    }
    /// Frozen static, visible subtree input for shared retained Canvas capture.
    /// The requested root is evaluated in its own local image space.
    /// Frozen specs of `root`'s visible subtree. An isolated crossfade inside
    /// it is captured at its nearest endpoint (not the blended intermediate).
    pub(crate) fn snapshot_subtree_specs(&self, client: ClientId, root: u32) -> Result<Vec<LayerSpec>> {
        let owner = self.clients.get(&client.0).context("snapshot client missing")?;
        ensure!(owner.contains_key(&root), "snapshot layer {root} missing");
        let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for (&id, layer) in owner {
            if let Some(parent) = layer.parent { children.entry(parent).or_default().push(id); }
        }
        let mut selected = BTreeSet::new();
        let mut stack = vec![(root, 0usize)];
        let mut out = Vec::new();
        while let Some((id, depth)) = stack.pop() {
            let original = &owner[&id];
            if id != root && (!original.visible || original.opacity == 0) { continue; }
            ensure!(depth < 30, "snapshot subtree depth budget");
            ensure!(selected.insert(id), "snapshot subtree cycle");
            ensure!(selected.len() <= 4096, "snapshot subtree node budget");
            let mut layer = original.clone();
            if id == root {
                layer.parent = None;
                layer.rect[0] = 0.; layer.rect[1] = 0.;
                layer.transform = SpriteTransform::default();
                layer.visible = true; layer.opacity = 255;
            }
            ensure!(layer.blend == Blend::Normal, "snapshot requires normal layer blend");
            ensure!(!matches!(layer.paint, Paint::Puppet { .. }), "snapshot of dynamic puppet needs an evaluated immutable resource");
            if let Paint::Snapshot { layers, .. } = &layer.paint {
                ensure!(1 + snapshot_nesting(layers) < MAX_SNAPSHOT_NESTING, "snapshot nesting budget");
            }
            ensure!(!self.dynamic_puppets.contains(&(client.0, id)) && !self.puppet_frames.contains_key(&(client.0, id)),
                "snapshot of evaluated/dynamic puppet needs an immutable mesh resource");
            ensure!(layer.paint.glyph_reveal_status().is_none_or(|r| r.complete), "snapshot of dynamic glyph reveal is unsupported");
            let t = &layer.transform;
            ensure!(!t.camera_enabled && !t.billboard && t.rotate_x == 0. && t.rotate_y == 0., "snapshot requires 2D layer transforms");
            if let Some(ids) = children.get(&id) {
                if ids.iter().any(|child| owner[child].visible && owner[child].opacity != 0) {
                    ensure!(layer.opacity == 255, "snapshot group opacity requires isolated composition");
                }
                stack.extend(ids.iter().map(|child| (*child, depth + 1)));
            }
            out.push(layer);
        }
        // An active isolated composition wholly inside the captured subtree is
        // frozen at its nearest endpoint (before below half progress, else the
        // source in the destination's place). One reaching outside fails.
        for (id, fade) in self.crossfades.get(&client.0).into_iter().flatten() {
            let inside = |n: u32| selected.contains(&n) && n != root;
            if inside(*id) && inside(fade.source) {
                let before = fade.elapsed_ms * 2 < fade.duration_ms;
                let drop = descendants_of(&out, if before { fade.source } else { *id });
                let dest_rect = owner[id].rect;
                let dest_visible = owner[id].visible;
                out.retain(|l| !drop.contains(&l.id));
                if !before {
                    if let Some(src) = out.iter_mut().find(|l| l.id == fade.source) {
                        src.rect[0] = dest_rect[0];
                        src.rect[1] = dest_rect[1];
                        src.visible = dest_visible;
                    }
                }
                continue;
            }
            for &node in &selected {
                let mut cursor = Some(node);
                while let Some(n) = cursor {
                    ensure!(n != *id && n != fade.source, "snapshot of active composite is unsupported");
                    cursor = owner.get(&n).and_then(|s| s.parent);
                }
            }
        }
        out.sort_by_key(|layer| (layer.order, layer.id));
        Ok(out)
    }
    /// Apply atomically. Commands mutate in place while an undo journal keeps
    /// the original value of every touched layer (and a copy of the small side
    /// state); an error restores them. Only touched layers are re-validated.
    fn side_state(&self) -> SideState {
        (
            self.crossfades.clone(),
            self.puppets.clone(),
            self.puppet_frames.clone(),
            self.dynamic_puppets.clone(),
            self.transition.clone(),
            self.transition_serial,
        )
    }
    /// Run `f` as one atomic unit over several `apply_batch` calls without
    /// cloning the layer tree: on error every layer the calls touched and the
    /// side state are restored. A nested transaction that succeeds folds its
    /// originals into the enclosing one.
    pub fn transaction<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.txn.0.push(LayerTxn {
            journal: BTreeMap::new(),
            owners: self.clients.keys().copied().collect(),
            side: self.side_state(),
        });
        let depth = self.txn.0.len();
        let result = f(self);
        ensure!(self.txn.0.len() == depth, "layer transaction stack");
        let txn = self.txn.0.pop().expect("transaction state");
        if result.is_ok() {
            if let Some(outer) = self.txn.0.last_mut() {
                for (key, original) in txn.journal {
                    outer.journal.entry(key).or_insert(original);
                }
            }
        } else {
            for ((client, id), original) in txn.journal {
                let owner = self.clients.entry(client).or_default();
                match original {
                    Some(spec) => owner.insert(id, spec),
                    None => owner.remove(&id),
                };
            }
            self.clients
                .retain(|client, owner| txn.owners.contains(client) || !owner.is_empty());
            (
                self.crossfades,
                self.puppets,
                self.puppet_frames,
                self.dynamic_puppets,
                self.transition,
                self.transition_serial,
            ) = txn.side;
        }
        result
    }
    pub fn apply_batch(&mut self, client: ClientId, commands: &[LayerCommand]) -> Result<()> {
        ensure!(commands.len() <= 4096, "layer command limit");
        let side = self.side_state();
        let had_owner = self.clients.contains_key(&client.0);
        let mut journal: BTreeMap<u32, Option<LayerSpec>> = BTreeMap::new();
        let result = self
            .apply_in_place(client, commands, &mut journal)
            .and_then(|()| {
                self.sync_puppets();
                self.validate_touched(client, journal.keys().copied())
            });
        if let Err(e) = result {
            let owner = self.clients.entry(client.0).or_default();
            for (id, original) in journal {
                match original {
                    Some(spec) => owner.insert(id, spec),
                    None => owner.remove(&id),
                };
            }
            if !had_owner && owner.is_empty() {
                self.clients.remove(&client.0);
            }
            (
                self.crossfades,
                self.puppets,
                self.puppet_frames,
                self.dynamic_puppets,
                self.transition,
                self.transition_serial,
            ) = side;
            return Err(e);
        }
        if let Some(txn) = self.txn.0.last_mut() {
            for (id, original) in journal {
                txn.journal.entry((client.0, id)).or_insert(original);
            }
        }
        Ok(())
    }
    fn apply_in_place(
        &mut self,
        client: ClientId,
        commands: &[LayerCommand],
        journal: &mut BTreeMap<u32, Option<LayerSpec>>,
    ) -> Result<()> {
        let staged = self;
        // parent → children, built on the first Remove and kept current.
        let mut children: Option<BTreeMap<u32, BTreeSet<u32>>> = None;
        for command in commands {
            match command {
                LayerCommand::BeginCrossfade { id, source, duration_ms, rule, external } => {
                    staged.begin_crossfade(client, *id, *source, *duration_ms, rule.clone(), *external)?;
                    continue;
                }
                LayerCommand::SetCrossfadeElapsed { id, elapsed_ms } => {
                    staged.set_crossfade_elapsed(client, *id, *elapsed_ms)?;
                    continue;
                }
                LayerCommand::EndCrossfade { id } => {
                    if let Some(fades) = staged.crossfades.get_mut(&client.0) { fades.remove(id); }
                    continue;
                }
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
                    // Snapshot payloads are immutable: checked once on entry,
                    // not on every revalidation.
                    if matches!(layer.paint, Paint::Snapshot { .. }) {
                        layer.paint.validate()?;
                    }
                    let previous = owner.insert(layer.id, layer.clone());
                    if let Some(index) = &mut children {
                        if let Some(p) = previous.as_ref().and_then(|s| s.parent) {
                            if let Some(set) = index.get_mut(&p) {
                                set.remove(&layer.id);
                            }
                        }
                        if let Some(p) = layer.parent {
                            index.entry(p).or_default().insert(layer.id);
                        }
                    }
                    journal.entry(layer.id).or_insert(previous);
                }
                LayerCommand::Remove { id } => {
                    ensure!(owner.contains_key(id), "layer {id} missing for removal");
                    let index = children.get_or_insert_with(|| {
                        let mut index: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
                        for (id, s) in owner.iter() {
                            if let Some(p) = s.parent {
                                index.entry(p).or_default().insert(*id);
                            }
                        }
                        index
                    });
                    let mut remove = BTreeSet::from([*id]);
                    let mut stack = vec![*id];
                    while let Some(n) = stack.pop() {
                        for c in index.get(&n).into_iter().flatten() {
                            if remove.insert(*c) {
                                stack.push(*c);
                            }
                        }
                    }
                    for r in &remove {
                        if let Some(spec) = owner.remove(r) {
                            if let Some(set) = spec.parent.and_then(|p| index.get_mut(&p)) {
                                set.remove(r);
                            }
                            journal.entry(*r).or_insert(Some(spec));
                        }
                        index.remove(r);
                    }
                    if let Some(fades) = staged.crossfades.get_mut(&client.0) { fades.retain(|id, f| !remove.contains(id) && !remove.contains(&f.source)); }
                    staged.dynamic_puppets.retain(|(c,id)| *c != client.0 || !remove.contains(id));
                    staged.puppet_frames.retain(|(c,id),_| *c != client.0 || !remove.contains(id));
                }
                other => {
                    let id = match other {
                        LayerCommand::SetTransform { id, .. }
                        | LayerCommand::SetVisible { id, .. }
                        | LayerCommand::CompleteReveal { id }
                        | LayerCommand::SetOpacity { id, .. }
                        | LayerCommand::SetOrder { id, .. }
                        | LayerCommand::SetClip { id, .. }
                        | LayerCommand::SetFilter { id, .. } => id,
                        _ => unreachable!(),
                    };
                    let layer = owner.get_mut(id).context("layer missing")?;
                    journal.entry(*id).or_insert_with(|| Some(layer.clone()));
                    match other {
                        LayerCommand::SetTransform { transform, .. } => {
                            layer.transform = transform.clone()
                        }
                        LayerCommand::SetVisible { visible, .. } => layer.visible = *visible,
                        LayerCommand::CompleteReveal { .. } => match &mut layer.paint {
                            Paint::GlyphRun { reveal, .. } => {
                                if let Some(reveal) = reveal {
                                    reveal.forced_complete = true;
                                }
                            }
                            _ => bail!("reveal target is not a positioned glyph run"),
                        },
                        LayerCommand::SetOpacity { opacity, .. } => layer.opacity = *opacity,
                        LayerCommand::SetOrder { order, .. } => layer.order = *order,
                        LayerCommand::SetClip { clip, .. } => layer.clip = *clip,
                        LayerCommand::SetFilter { filter, .. } => layer.filter = filter.clone(),
                        _ => unreachable!(),
                    }
                }
            }
        }
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
        self.dynamic_puppets.retain(|(c, id)| clients.get(c).is_some_and(|s| s.contains_key(id)));
        let dynamic = &self.dynamic_puppets;
        self.puppet_frames
            .retain(|(c, id), _| dynamic.contains(&(*c, *id)) || puppets.get(c).is_some_and(|s| s.contains_key(id)));
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
    /// Validate and publish an engine puppet frame on an owned layer.
    /// The frame is derived state; no fake rig asset or VM renderer is needed.
    pub fn set_dynamic_puppet(&mut self, client: u32, id: u32, frame: Arc<crysirror_puppet::PuppetFrame>) -> Result<()> {
        frame.validate()?;
        ensure!(self.clients.get(&client).is_some_and(|s| s.contains_key(&id)), "dynamic puppet layer missing");
        ensure!(self.puppet_slot(client, id).is_none(), "static rig layer cannot also own a dynamic puppet");
        self.dynamic_puppets.insert((client, id));
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
    /// Inspect the same client-scoped progress the renderer uses.
    pub fn reveal_status(&self, client: ClientId, id: u32) -> Option<GlyphRevealStatus> {
        self.clients
            .get(&client.0)?
            .get(&id)?
            .paint
            .glyph_reveal_status()
    }
    /// Advance engine-owned text and transition clocks.
    pub fn tick(&mut self, delta_ms: u64) {
        for fade in self.crossfades.values_mut().flat_map(|s| s.values_mut()).filter(|f| !f.external) {
            fade.elapsed_ms = fade.elapsed_ms.saturating_add(delta_ms).min(fade.duration_ms);
        }
        for layer in self
            .clients
            .values_mut()
            .flat_map(|owner| owner.values_mut())
        {
            if let Paint::GlyphRun {
                glyphs,
                reveal: Some(reveal),
                ..
            } = &mut layer.paint
            {
                reveal.advance(glyphs.len(), delta_ms);
            }
        }
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
    /// Global invariants plus the layers a batch touched (still present).
    fn validate_touched(&self, client: ClientId, ids: impl Iterator<Item = u32>) -> Result<()> {
        if let Some(t) = &self.transition {
            t.spec.validate()?;
            ensure!(t.serial <= self.transition_serial, "transition serial");
        }
        ensure!(
            self.clients.len() <= 64
                && self.clients.values().map(BTreeMap::len).sum::<usize>() <= MAX_LAYERS,
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
        if let Some(owner) = self.clients.get(&client.0) {
            for id in ids {
                if let Some(s) = owner.get(&id) {
                    // Snapshot payloads were checked when they entered.
                    validate_layer(owner, id, s, false)?;
                }
            }
        }
        self.validate_crossfades()
    }
    /// Full validation, including frozen snapshot payloads (restore paths).
    pub fn validate(&self) -> Result<()> {
        self.validate_with(true)
    }
    fn validate_with(&self, snapshots: bool) -> Result<()> {
        if let Some(t) = &self.transition {
            t.spec.validate()?;
            ensure!(t.serial <= self.transition_serial, "transition serial");
        }
        ensure!(
            self.clients.len() <= 64
                && self.clients.values().map(BTreeMap::len).sum::<usize>() <= MAX_LAYERS,
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
                validate_layer(owner, *id, s, snapshots)?;
            }
        }
        self.validate_crossfades()
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
        self.composite_frame(canvas)
    }
    fn frame_plain(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        // State is valid by construction: every mutation path validates what
        // it changed (apply_batch, install, restore via `validate`).
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
                    // Unvalidated (e.g. directly deserialized) state: a chain
                    // longer than the owner's layer count is a cycle.
                    ensure!(chain.len() <= owner.len(), "layer parent cycle");
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
                    composite: None,
                    owner: *client,
                    id: spec.id,
                    sub: 0,
                    order: spec.order,
                    size: [spec.rect[2].ceil() as u32, spec.rect[3].ceil() as u32],
                    extent: [spec.rect[2], spec.rect[3]],
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
        composite: None,
        owner,
        id,
        sub,
        order,
        size: [w.ceil().max(1.) as u32, h.ceil().max(1.) as u32],
        extent: [w, h],
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

/// `root` and every spec below it within `specs`.
fn descendants_of(specs: &[LayerSpec], root: u32) -> BTreeSet<u32> {
    let mut ids = BTreeSet::from([root]);
    loop {
        let before = ids.len();
        for s in specs {
            if s.parent.is_some_and(|p| ids.contains(&p)) {
                ids.insert(s.id);
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

/// Per-layer invariants. The parent walk is bounded by depth instead of a
/// visited set: a cycle exceeds the depth limit, so no allocation is needed.
fn validate_layer(owner: &BTreeMap<u32, LayerSpec>, id: u32, s: &LayerSpec, snapshots: bool) -> Result<()> {
    ensure!(id == s.id && rect_valid(s.rect), "invalid layer identity/bounds");
    if snapshots || !matches!(s.paint, Paint::Snapshot { .. }) {
        s.paint.validate()?;
    }
    s.transform.validate()?;
    if let Some(clip) = s.clip {
        ensure!(rect_valid(clip), "invalid layer clip");
    }
    if let Some(f) = &s.filter {
        f.validate()?;
    }
    if let Some(uv) = s.uv {
        ensure!(
            !matches!(s.paint, Paint::GlyphRun { .. }),
            "glyph runs do not take image UV crops"
        );
        ensure!(
            uv.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) && uv[2] > uv[0] && uv[3] > uv[1],
            "invalid source UV"
        );
    }
    let mut depth = 0;
    let mut p = s.parent;
    while let Some(parent) = p {
        depth += 1;
        ensure!(depth <= 64 && parent != id, "layer parent cycle/depth");
        p = owner.get(&parent).context("layer parent missing")?.parent;
    }
    Ok(())
}
