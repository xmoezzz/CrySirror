//! Device-free vector scenes. Brushes, groups, masks and filters are engine
//! operations; compatibility VMs only translate their own drawing commands.
use crate::{Error, Result, image_processing::ImageResource};
use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PathCommand {
    Move {
        to: [f32; 2],
    },
    Line {
        to: [f32; 2],
    },
    Quad {
        control: [f32; 2],
        to: [f32; 2],
    },
    Cubic {
        control1: [f32; 2],
        control2: [f32; 2],
        to: [f32; 2],
    },
    Close,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorSpread {
    Pad,
    Repeat,
    Reflect,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorGradientStop {
    pub offset: f32,
    pub color: [u8; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VectorBrush {
    Solid {
        color: [u8; 4],
    },
    LinearGradient {
        start: [f32; 2],
        end: [f32; 2],
        stops: Vec<VectorGradientStop>,
        spread: VectorSpread,
        transform: [f32; 6],
    },
    RadialGradient {
        center: [f32; 2],
        focal: [f32; 2],
        radius: f32,
        stops: Vec<VectorGradientStop>,
        spread: VectorSpread,
        transform: [f32; 6],
    },
    Bitmap {
        id: u32,
        transform: [f32; 6],
        spread: VectorSpread,
        smoothing: bool,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorLineCap {
    Butt,
    Round,
    Square,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorLineJoin {
    Miter,
    Round,
    Bevel,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorStroke {
    pub brush: VectorBrush,
    pub width: f32,
    pub cap: VectorLineCap,
    pub join: VectorLineJoin,
    pub miter_limit: f32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorBlend {
    SourceOver,
    Multiply,
    Screen,
    Lighten,
    Darken,
    Difference,
    Add,
    Subtract,
    Invert,
    Alpha,
    Erase,
    Overlay,
    HardLight,
}
/// Straight, normalized RGBA. Additions are normalized units, not byte values.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorColorTransform {
    pub multiply: [f32; 4],
    pub add: [f32; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VectorFilter {
    /// Separable, transparent-border box blur over premultiplied channels.
    BoxBlur { radius: [u16; 2], passes: u8 },
    /// Row-major normalized RGBA 4x5 matrix, applied to straight channels.
    ColorMatrix { matrix: [f32; 20] },
    DropShadow {
        offset: [i32; 2],
        color: [u8; 4],
        blur: [u16; 2],
        passes: u8,
        strength: f32,
        inner: bool,
        knockout: bool,
        hide_object: bool,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorMaskKind {
    Alpha,
    Luminance,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorMask {
    pub nodes: Vec<VectorNode>,
    pub kind: VectorMaskKind,
    pub invert: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VectorNode {
    Path {
        commands: Vec<PathCommand>,
        transform: [f32; 6],
        fill: Option<VectorBrush>,
        stroke: Option<VectorStroke>,
        even_odd: bool,
    },
    Bitmap {
        id: u32,
        transform: [f32; 6],
        smoothing: bool,
        opacity: f32,
    },
    /// Isolated group. The order is child drawing, color transform, filters,
    /// clip/mask, opacity and blend into its parent. Coordinates are inherited.
    Group {
        nodes: Vec<VectorNode>,
        transform: [f32; 6],
        opacity: f32,
        blend: VectorBlend,
        color_transform: Option<VectorColorTransform>,
        filters: Vec<VectorFilter>,
        clip: Option<[f32; 4]>,
        mask: Option<VectorMask>,
    },
}
#[derive(Clone, Debug)]
pub struct VectorSceneSpec {
    pub size: [u32; 2],
    pub clear: [u8; 4],
    pub bitmaps: BTreeMap<u32, Arc<dyn ImageResource>>,
    pub nodes: Vec<VectorNode>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct VectorSceneBudget {
    pub nodes: usize,
    pub path_commands: usize,
    pub depth: usize,
    pub bitmap_bytes: usize,
}
fn finite(values: &[f32]) -> bool {
    values
        .iter()
        .all(|n| n.is_finite() && n.abs() <= 1_000_000.0)
}
fn invertible(m: &[f32; 6]) -> bool {
    finite(m) && (m[0] * m[3] - m[1] * m[2]).abs() > 1e-12
}
fn opacity(n: f32) -> bool {
    n.is_finite() && (0.0..=1.0).contains(&n)
}
fn bad() -> Error {
    Error::Invalid("vector scene geometry or paint")
}
impl VectorSceneSpec {
    pub fn validate(&self) -> Result<VectorSceneBudget> {
        if self.size.iter().any(|n| *n == 0 || *n > 16384)
            || u64::from(self.size[0]) * u64::from(self.size[1]) > 16 * 1024 * 1024
            || self.bitmaps.len() > 1024
        {
            return Err(Error::Limit("vector scene resource budget"));
        }
        let mut budget = VectorSceneBudget::default();
        for image in self.bitmaps.values() {
            let [w, h] = image.size();
            let bytes = u64::from(w) * u64::from(h) * 4;
            if w == 0 || h == 0 || bytes > 64 * 1024 * 1024 {
                return Err(Error::Limit("vector bitmap bytes"));
            }
            budget.bitmap_bytes = budget
                .bitmap_bytes
                .checked_add(bytes as usize)
                .ok_or(Error::Limit("vector bitmap budget"))?;
        }
        fn brush(b: &VectorBrush, scene: &VectorSceneSpec) -> Result<()> {
            let gradient = |stops: &[VectorGradientStop]| -> Result<()> {
                if !(2..=64).contains(&stops.len())
                    || stops.iter().any(|s| !opacity(s.offset))
                    || stops.windows(2).any(|v| v[0].offset > v[1].offset)
                {
                    return Err(bad());
                }
                Ok(())
            };
            match b {
                VectorBrush::Solid { .. } => {}
                VectorBrush::Bitmap { id, transform, .. } => {
                    if !scene.bitmaps.contains_key(id) || !invertible(transform) {
                        return Err(bad());
                    }
                }
                VectorBrush::LinearGradient {
                    start,
                    end,
                    stops,
                    transform,
                    ..
                } => {
                    if !finite(start) || !finite(end) || !invertible(transform) || start == end {
                        return Err(bad());
                    }
                    gradient(stops)?;
                }
                VectorBrush::RadialGradient {
                    center,
                    focal,
                    radius,
                    stops,
                    transform,
                    ..
                } => {
                    if !finite(center)
                        || !finite(focal)
                        || !invertible(transform)
                        || !radius.is_finite()
                        || !(0.0001..=1_000_000.0).contains(radius)
                    {
                        return Err(bad());
                    }
                    gradient(stops)?;
                }
            }
            Ok(())
        }
        fn visit(
            nodes: &[VectorNode],
            depth: usize,
            scene: &VectorSceneSpec,
            budget: &mut VectorSceneBudget,
        ) -> Result<()> {
            if depth > 32 {
                return Err(Error::Limit("vector scene group depth"));
            }
            budget.depth = budget.depth.max(depth);
            for node in nodes {
                budget.nodes += 1;
                if budget.nodes > 4096 {
                    return Err(Error::Limit("vector scene nodes"));
                }
                match node {
                    VectorNode::Path {
                        commands,
                        transform,
                        fill,
                        stroke,
                        ..
                    } => {
                        if commands.is_empty()
                            || commands.len() > 8192
                            || !matches!(commands.first(), Some(PathCommand::Move { .. }))
                            || !finite(transform)
                            || fill.is_none() && stroke.is_none()
                        {
                            return Err(bad());
                        }
                        budget.path_commands += commands.len();
                        if budget.path_commands > 262144 {
                            return Err(Error::Limit("vector path commands"));
                        }
                        for command in commands {
                            let valid = match command {
                                PathCommand::Move { to } | PathCommand::Line { to } => finite(to),
                                PathCommand::Quad { control, to } => finite(control) && finite(to),
                                PathCommand::Cubic {
                                    control1,
                                    control2,
                                    to,
                                } => finite(control1) && finite(control2) && finite(to),
                                PathCommand::Close => true,
                            };
                            if !valid {
                                return Err(bad());
                            }
                        }
                        if let Some(fill) = fill {
                            brush(fill, scene)?;
                        }
                        if let Some(s) = stroke {
                            if !s.width.is_finite()
                                || !(0.0..=1024.0).contains(&s.width)
                                || !s.miter_limit.is_finite()
                                || !(1.0..=1024.0).contains(&s.miter_limit)
                            {
                                return Err(bad());
                            }
                            brush(&s.brush, scene)?;
                        }
                    }
                    VectorNode::Bitmap {
                        id,
                        transform,
                        opacity: a,
                        ..
                    } => {
                        if !scene.bitmaps.contains_key(id) || !finite(transform) || !opacity(*a) {
                            return Err(bad());
                        }
                    }
                    VectorNode::Group {
                        nodes,
                        transform,
                        opacity: a,
                        color_transform,
                        filters,
                        clip,
                        mask,
                        ..
                    } => {
                        if !finite(transform) || !opacity(*a) || filters.len() > 16 {
                            return Err(bad());
                        }
                        if let Some(c) = color_transform {
                            if !finite(&c.multiply) || !finite(&c.add) {
                                return Err(bad());
                            }
                        }
                        if let Some(rect) = clip {
                            if !finite(rect) || rect[2] < 0.0 || rect[3] < 0.0 {
                                return Err(bad());
                            }
                        }
                        for f in filters {
                            match f {
                                VectorFilter::BoxBlur { radius, passes }
                                | VectorFilter::DropShadow {
                                    blur: radius,
                                    passes,
                                    ..
                                } => {
                                    if radius.iter().any(|r| *r > 128) || !(1..=4).contains(passes)
                                    {
                                        return Err(bad());
                                    }
                                }
                                VectorFilter::ColorMatrix { matrix } => {
                                    if !finite(matrix) {
                                        return Err(bad());
                                    }
                                }
                            }
                            if let VectorFilter::DropShadow {
                                offset, strength, ..
                            } = f
                            {
                                if offset.iter().any(|n| n.unsigned_abs() > 65536)
                                    || !strength.is_finite()
                                    || !(0.0..=16.0).contains(strength)
                                {
                                    return Err(bad());
                                }
                            }
                        }
                        visit(nodes, depth + 1, scene, budget)?;
                        if let Some(mask) = mask {
                            visit(&mask.nodes, depth + 1, scene, budget)?;
                        }
                    }
                }
            }
            Ok(())
        }
        visit(&self.nodes, 0, self, &mut budget)?;
        Ok(budget)
    }
}
