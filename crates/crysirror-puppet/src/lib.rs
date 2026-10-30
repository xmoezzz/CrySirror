// SPDX-License-Identifier: MPL-2.0
//! Engine-owned 2D deformable puppets (ADR 0029), the class of runtime that
//! Live2D and E-mote provide.
//!
//! * [`PuppetRig`] is CrySirror's own packaged format: textured parts,
//!   per-parameter keyforms, deformer chains, motions and pendulum physics.
//!   It is plain JSON next to its textures; no vendor container is involved.
//! * [`Puppet`] evaluates a rig: parameters, motions, scripted parameter
//!   tweens and physics, producing a [`PuppetFrame`] of meshes.
//! * [`PuppetSource`] is the interface the engine drives. A rig is one
//!   source; other runtimes (for example the eluna E-mote player) can supply
//!   frames through the same interface, and [`bake`] samples any source into
//!   a rig so that packages only ever contain the engine format.
//!
//! No GPU, window, audio or file-system access happens here.
#![forbid(unsafe_code)]
pub mod bake;
pub mod cpu_snapshot;
mod eval;
mod source;

pub use bake::{bake, BakeOptions, BakeReport, BakedTexture};
pub use eval::{Puppet, PuppetState};
pub use source::{MotionInfo, ParameterInfo, PlayOptions, PuppetSource};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const FORMAT: &str = "crysirror-puppet";
pub const VERSION: u32 = 1;
pub const MAX_PARTS: usize = 4096;
pub const MAX_VERTICES: usize = 1 << 20;
pub const MAX_PARAMETERS: usize = 1024;

/// A complete puppet: everything the evaluator needs except texture pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PuppetRig {
    /// Always `"crysirror-puppet"`.
    pub format: String,
    /// Format version (1).
    pub version: u32,
    /// Canvas size [w, h] in puppet pixels; frames are expressed in it.
    pub size: [f32; 2],
    /// Texture images, relative to the rig file.
    pub textures: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub deformers: Vec<Deformer>,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub motions: Vec<Motion>,
    #[serde(default)]
    pub physics: Vec<Pendulum>,
    /// Where the rig came from (tool, source kind); informational only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub id: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

/// Rotation/translation/scale about a pivot, driven by keyforms and
/// applied to its parts and child deformers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deformer {
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// Pivot in the parent's (rest) space.
    #[serde(default)]
    pub pivot: [f32; 2],
    #[serde(default)]
    pub keyforms: Vec<DeformerKeyform>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeformerKeyform {
    pub parameter: String,
    pub keys: Vec<DeformerKey>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeformerKey {
    pub value: f32,
    /// Degrees, counter-clockwise on screen (y down).
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub translation: [f32; 2],
    #[serde(default = "one2")]
    pub scale: [f32; 2],
}
fn one2() -> [f32; 2] {
    [1., 1.]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartBlend {
    #[default]
    Normal,
    Add,
    /// Destination minus source (reverse subtract).
    Subtract,
    Multiply,
    Screen,
}

/// A textured triangle mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub id: String,
    /// Index into `textures`.
    pub texture: u32,
    /// Rest positions in puppet pixels.
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Per-vertex linear RGBA multipliers; empty means white.
    #[serde(default)]
    pub colors: Vec<[f32; 4]>,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub blend: PartBlend,
    #[serde(default)]
    pub deformer: Option<String>,
    /// Parts whose coverage clips this part (union); empty for none.
    #[serde(default)]
    pub masks: Vec<String>,
    /// Draw outside the masks instead of inside.
    #[serde(default)]
    pub invert_mask: bool,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub keyforms: Vec<PartKeyform>,
}
fn one() -> f32 {
    1.
}
fn yes() -> bool {
    true
}
/// Keys of one parameter. Contributions of different parameters add up
/// (offsets and order) or multiply (opacity).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartKeyform {
    pub parameter: String,
    pub keys: Vec<PartKey>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartKey {
    pub value: f32,
    /// Per-vertex offsets from the rest positions; empty for none.
    #[serde(default)]
    pub offsets: Vec<[f32; 2]>,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub order: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    #[default]
    Linear,
    Step,
    /// Smoothstep between keys.
    Smooth,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionBlend {
    /// Replace the parameter (weighted by the fade).
    #[default]
    Override,
    /// Add the curve's difference from the parameter default.
    Additive,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Motion {
    pub id: String,
    pub duration_ms: f32,
    #[serde(default)]
    pub looping: bool,
    #[serde(default)]
    pub blend: MotionBlend,
    #[serde(default = "fade")]
    pub fade_in_ms: f32,
    #[serde(default = "fade")]
    pub fade_out_ms: f32,
    pub curves: Vec<Curve>,
}
fn fade() -> f32 {
    200.
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    pub parameter: String,
    /// [time ms, value], sorted by time.
    pub keys: Vec<[f32; 2]>,
    #[serde(default)]
    pub interpolation: Interpolation,
}

/// A damped pendulum: weighted input parameters swing it, its angle drives
/// an output parameter (hair, accessories, breathing sway).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pendulum {
    pub id: String,
    pub inputs: Vec<PendulumInput>,
    /// Output parameter, set to `angle / max_angle` mapped onto its range.
    pub output: String,
    /// Angle (degrees) reached for a fully saturated input.
    #[serde(default = "max_angle")]
    pub max_angle: f32,
    /// Spring stiffness (1/s²) and damping (1/s).
    #[serde(default = "stiffness")]
    pub stiffness: f32,
    #[serde(default = "damping")]
    pub damping: f32,
}
fn max_angle() -> f32 {
    30.
}
fn stiffness() -> f32 {
    60.
}
fn damping() -> f32 {
    6.
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendulumInput {
    pub parameter: String,
    /// Weight of the parameter normalized to -1..1 over its range.
    pub weight: f32,
}

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

impl PuppetRig {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let rig: Self = serde_json::from_slice(bytes).context("invalid puppet rig JSON")?;
        rig.validate()?;
        Ok(rig)
    }
    pub fn parameter(&self, id: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.id == id)
    }
    /// Structural validation: every reference resolves, every array has the
    /// length its mesh implies, and the deformer graph is acyclic.
    pub fn validate(&self) -> Result<()> {
        ensure!(self.format == FORMAT && self.version == VERSION, "not a {FORMAT} v{VERSION} rig");
        ensure!(finite(&self.size) && self.size.iter().all(|v| *v > 0. && *v <= 16384.), "invalid puppet size");
        ensure!(!self.textures.is_empty() && self.textures.len() <= 256, "a puppet needs 1..=256 textures");
        for t in &self.textures {
            ensure!(!t.is_empty() && !t.starts_with('/') && !t.contains("..") && !t.contains('\\'), "texture '{t}' must be a relative path");
            ensure!(!t.to_ascii_lowercase().ends_with(".psb"), "puppet textures cannot be vendor containers");
        }
        ensure!(self.parameters.len() <= MAX_PARAMETERS && self.parts.len() <= MAX_PARTS, "puppet too large");
        let mut params = BTreeSet::new();
        for p in &self.parameters {
            ensure!(params.insert(p.id.as_str()), "duplicate parameter '{}'", p.id);
            ensure!(finite(&[p.min, p.max, p.default]) && p.min <= p.default && p.default <= p.max && p.min < p.max,
                "parameter '{}' needs min ≤ default ≤ max and min < max", p.id);
        }
        let param = |id: &str| -> Result<()> {
            ensure!(params.contains(id), "unknown parameter '{id}'");
            Ok(())
        };
        let keys_sorted = |values: Vec<f32>, what: &str| -> Result<()> {
            ensure!(!values.is_empty() && finite(&values), "{what}: needs finite keys");
            ensure!(values.windows(2).all(|w| w[0] < w[1]), "{what}: key values must increase");
            Ok(())
        };
        let mut deformers = BTreeSet::new();
        for d in &self.deformers {
            ensure!(deformers.insert(d.id.as_str()), "duplicate deformer '{}'", d.id);
            ensure!(finite(&d.pivot), "deformer '{}' pivot", d.id);
            for k in &d.keyforms {
                param(&k.parameter)?;
                keys_sorted(k.keys.iter().map(|k| k.value).collect(), &format!("deformer '{}'", d.id))?;
                for key in &k.keys {
                    ensure!(finite(&[key.rotation, key.translation[0], key.translation[1], key.scale[0], key.scale[1]]),
                        "deformer '{}' key", d.id);
                }
            }
        }
        for d in &self.deformers {
            // Walk to the root; a chain longer than the deformer count is a cycle.
            let mut cur = d.parent.as_deref();
            let mut steps = 0;
            while let Some(p) = cur {
                ensure!(deformers.contains(p), "deformer '{}' has unknown parent '{p}'", d.id);
                steps += 1;
                ensure!(steps <= self.deformers.len(), "deformer cycle through '{}'", d.id);
                cur = self.deformers.iter().find(|x| x.id == p).unwrap().parent.as_deref();
            }
        }
        let mut parts = BTreeSet::new();
        let mut vertices = 0usize;
        for p in &self.parts {
            ensure!(parts.insert(p.id.as_str()), "duplicate part '{}'", p.id);
            let n = p.positions.len();
            vertices += n;
            ensure!(vertices <= MAX_VERTICES, "puppet has too many vertices");
            ensure!((p.texture as usize) < self.textures.len(), "part '{}' texture index", p.id);
            ensure!(n >= 3 && p.uvs.len() == n && (p.colors.is_empty() || p.colors.len() == n), "part '{}' vertex arrays", p.id);
            ensure!(!p.indices.is_empty() && p.indices.len() % 3 == 0 && p.indices.iter().all(|i| (*i as usize) < n),
                "part '{}' indices", p.id);
            ensure!(p.positions.iter().chain(&p.uvs).all(|v| finite(v)) && p.colors.iter().all(|c| finite(c)), "part '{}' values", p.id);
            ensure!(p.opacity.is_finite() && (0.0..=1.).contains(&p.opacity), "part '{}' opacity", p.id);
            if let Some(d) = &p.deformer {
                ensure!(deformers.contains(d.as_str()), "part '{}' has unknown deformer '{d}'", p.id);
            }
            for k in &p.keyforms {
                param(&k.parameter)?;
                keys_sorted(k.keys.iter().map(|k| k.value).collect(), &format!("part '{}'", p.id))?;
                for key in &k.keys {
                    ensure!(key.offsets.is_empty() || key.offsets.len() == n, "part '{}' key offsets must cover every vertex", p.id);
                    ensure!(key.offsets.iter().all(|v| finite(v)) && (0.0..=1.).contains(&key.opacity), "part '{}' key values", p.id);
                }
            }
        }
        for p in &self.parts {
            for m in &p.masks {
                ensure!(parts.contains(m.as_str()) && m != &p.id, "part '{}' has unknown mask '{m}'", p.id);
            }
        }
        let mut motions = BTreeSet::new();
        for m in &self.motions {
            ensure!(motions.insert(m.id.as_str()), "duplicate motion '{}'", m.id);
            ensure!(m.duration_ms.is_finite() && m.duration_ms > 0. && m.fade_in_ms >= 0. && m.fade_out_ms >= 0., "motion '{}' timing", m.id);
            for c in &m.curves {
                param(&c.parameter)?;
                keys_sorted(c.keys.iter().map(|k| k[0]).collect(), &format!("motion '{}'", m.id))?;
                ensure!(c.keys.iter().all(|k| finite(k)), "motion '{}' values", m.id);
            }
        }
        for p in &self.physics {
            param(&p.output)?;
            for i in &p.inputs {
                param(&i.parameter)?;
                ensure!(i.weight.is_finite(), "pendulum '{}' weight", p.id);
            }
            ensure!(finite(&[p.max_angle, p.stiffness, p.damping]) && p.max_angle > 0. && p.stiffness > 0. && p.damping >= 0.,
                "pendulum '{}' constants", p.id);
        }
        Ok(())
    }
}

/// One evaluated part, ready to draw (positions in puppet pixels).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FramePart {
    pub id: String,
    pub texture: u32,
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Per-vertex color including the evaluated opacity.
    pub colors: Vec<[f32; 4]>,
    pub blend: PartBlend,
    /// Indices into `PuppetFrame::parts` of the clipping masks.
    pub masks: Vec<usize>,
    pub invert_mask: bool,
    /// Masks are drawn but not shown when this is false.
    pub visible: bool,
}
/// The output of one evaluation: parts in draw order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PuppetFrame {
    pub size: [f32; 2],
    pub parts: Vec<FramePart>,
    /// Texture paths indexed by `FramePart::texture` (rig-relative from the
    /// evaluator; hosts may rewrite them to their own asset paths).
    #[serde(default)]
    pub textures: Vec<String>,
}

impl PuppetFrame {
    /// Validate a worker-produced frame before owner publication or rendering.
    pub fn validate(&self) -> Result<()> {
        ensure!(self.size.iter().all(|v| v.is_finite() && *v > 0. && *v <= 16384.), "invalid puppet frame size");
        ensure!(self.parts.len() <= MAX_PARTS && self.textures.len() <= MAX_PARTS, "puppet frame part/texture limit");
        ensure!(self.textures.iter().all(|p| !p.is_empty() && p.len() <= 4096), "invalid puppet texture path");
        let mut vertices = 0usize;
        let mut indices = 0usize;
        let mut masks = 0usize;
        for (index, part) in self.parts.iter().enumerate() {
            ensure!(part.id.len() <= 4096, "puppet part identifier limit");
            vertices = vertices.checked_add(part.positions.len()).context("puppet vertex overflow")?;
            indices = indices.checked_add(part.indices.len()).context("puppet index overflow")?;
            masks = masks.checked_add(part.masks.len()).context("puppet mask overflow")?;
            ensure!(indices <= MAX_VERTICES * 6 && masks <= MAX_PARTS * 16, "puppet index/mask budget");
            ensure!(vertices <= MAX_VERTICES, "puppet frame vertex limit");
            ensure!((part.texture as usize) < self.textures.len(), "puppet texture index");
            ensure!(part.positions.len() == part.uvs.len() && part.positions.len() == part.colors.len(), "puppet attribute lengths");
            ensure!(part.indices.len() <= MAX_VERTICES * 6 && part.indices.len() % 3 == 0, "puppet triangle count");
            ensure!(part.indices.iter().all(|i| (*i as usize) < part.positions.len()), "puppet vertex index");
            ensure!(part.positions.iter().flatten().chain(part.uvs.iter().flatten()).chain(part.colors.iter().flatten()).all(|v| v.is_finite()), "nonfinite puppet attribute");
            ensure!(part.masks.len() <= MAX_PARTS && part.masks.iter().all(|m| *m < self.parts.len() && *m != index), "puppet mask index");
        }
        // Masks may reference other masks, but cycles are not valid geometry.
        fn visit(frame: &PuppetFrame, i: usize, states: &mut [u8]) -> Result<()> {
            ensure!(states[i] != 1, "cyclic puppet masks");
            if states[i] == 2 { return Ok(()); }
            states[i] = 1;
            for m in &frame.parts[i].masks { visit(frame, *m, states)?; }
            states[i] = 2; Ok(())
        }
        let mut states = vec![0; self.parts.len()];
        for i in 0..self.parts.len() { visit(self, i, &mut states)?; }
        Ok(())
    }
}

/// Clip a textured mesh against `[left, top, right, bottom]` in its current
/// coordinate space, interpolating UVs and colors at generated vertices.
/// The result remains ordinary engine triangles and needs no native scissor API.
pub fn clip_part_rect(part: &mut FramePart, rect: [f32; 4]) -> Result<()> {
    ensure!(rect.iter().all(|n| n.is_finite()) && rect[0] <= rect[2] && rect[1] <= rect[3], "invalid mesh clip rectangle");
    ensure!(part.positions.len() == part.uvs.len() && part.positions.len() == part.colors.len()
        && part.indices.len() % 3 == 0 && part.indices.iter().all(|i| (*i as usize) < part.positions.len()), "invalid mesh clip input");
    ensure!(part.positions.len() <= MAX_VERTICES && part.indices.len() <= MAX_VERTICES * 6,
        "mesh clip input budget");
    ensure!(part.positions.iter().flatten().chain(part.uvs.iter().flatten()).chain(part.colors.iter().flatten()).all(|v| v.is_finite()),
        "nonfinite mesh clip input");
    #[derive(Clone, Copy)] struct Vertex { p: [f32;2], uv: [f32;2], color: [f32;4] }
    fn mix(a: Vertex, b: Vertex, t: f32) -> Vertex {
        Vertex { p: std::array::from_fn(|i|a.p[i]+(b.p[i]-a.p[i])*t),
            uv: std::array::from_fn(|i|a.uv[i]+(b.uv[i]-a.uv[i])*t),
            color: std::array::from_fn(|i|a.color[i]+(b.color[i]-a.color[i])*t) }
    }
    let mut positions=Vec::new(); let mut uvs=Vec::new(); let mut colors=Vec::new(); let mut indices=Vec::new();
    for triangle in part.indices.chunks_exact(3) {
        let mut polygon=triangle.iter().map(|i|{let i=*i as usize;Vertex{p:part.positions[i],uv:part.uvs[i],color:part.colors[i]}}).collect::<Vec<_>>();
        for (axis, boundary, lower) in [(0,rect[0],true),(0,rect[2],false),(1,rect[1],true),(1,rect[3],false)] {
            if polygon.is_empty() {break}
            let inside=|v:Vertex|if lower {v.p[axis]>=boundary} else {v.p[axis]<=boundary};
            let mut next=Vec::with_capacity(polygon.len()+1);let mut previous=*polygon.last().unwrap();
            for current in polygon {
                let (a,b)=(inside(previous),inside(current));
                if a!=b {let t=(boundary-previous.p[axis])/(current.p[axis]-previous.p[axis]);next.push(mix(previous,current,t));}
                if b {next.push(current)} previous=current;
            }
            polygon=next;
        }
        if polygon.len()<3 {continue}
        ensure!(positions.len()+polygon.len()<=MAX_VERTICES,"clipped mesh vertex limit");
        let base=positions.len() as u32;
        for i in 1..polygon.len()-1 {indices.extend_from_slice(&[base,base+i as u32,base+i as u32+1]);}
        for v in polygon {positions.push(v.p);uvs.push(v.uv);colors.push(v.color);}
    }
    part.positions=positions;part.uvs=uvs;part.colors=colors;part.indices=indices;
    Ok(())
}
