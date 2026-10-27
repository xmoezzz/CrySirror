// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Engine vector paths over pinned tiny-skia; no GDI+, DLL, VM or device.
use crate::ImageSurface;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
/// A geometric path command; curves use explicit control points.
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
/// A solid-color stroke with butt caps and miter joins.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathStroke {
    pub color: [u8; 4],
    pub width: f32,
}
/// Solid fills/strokes with antialiasing and a six-value affine transform
/// [scale_x,skew_y,skew_x,scale_y,translate_x,translate_y]. Stroke caps are butt,
/// joins are miter with limit 4. No font drawing, GDI brush or metafile aliases.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorPath {
    pub commands: Vec<PathCommand>,
    pub fill: Option<[u8; 4]>,
    pub stroke: Option<PathStroke>,
    pub transform: [f32; 6],
    pub even_odd: bool,
}
impl VectorPath {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.commands.is_empty() && self.commands.len() <= 8192,
            "vector command budget"
        );
        ensure!(
            matches!(self.commands.first(), Some(PathCommand::Move { .. })),
            "vector path must start with move"
        );
        ensure!(
            self.fill.is_some() || self.stroke.is_some(),
            "vector paint absent"
        );
        ensure!(
            self.transform
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "vector transform"
        );
        ensure!(
            (self.transform[0] * self.transform[3] - self.transform[1] * self.transform[2]).abs()
                > 1e-8,
            "singular vector transform"
        );
        if let Some(s) = &self.stroke {
            ensure!(
                s.width.is_finite() && (0.001..=1024.).contains(&s.width),
                "vector stroke width"
            );
        }
        for c in &self.commands {
            let points: Vec<[f32; 2]> = match c {
                PathCommand::Move { to } | PathCommand::Line { to } => vec![*to],
                PathCommand::Quad { control, to } => vec![*control, *to],
                PathCommand::Cubic {
                    control1,
                    control2,
                    to,
                } => vec![*control1, *control2, *to],
                PathCommand::Close => vec![],
            };
            ensure!(
                points
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                "vector coordinate"
            );
        }
        Ok(())
    }
}
impl ImageSurface {
    pub fn draw_vector(&self, path: &VectorPath, check: &dyn Fn() -> Result<()>) -> Result<Self> {
        self.validate()?;
        path.validate()?;
        ensure!(
            u64::from(self.width) * u64::from(self.height) <= crate::effects::MAX_EFFECT_PIXELS,
            "vector pixel budget"
        );
        check()?;
        let mut pb = tiny_skia::PathBuilder::new();
        for command in &path.commands {
            match command {
                PathCommand::Move { to } => pb.move_to(to[0], to[1]),
                PathCommand::Line { to } => pb.line_to(to[0], to[1]),
                PathCommand::Quad { control, to } => {
                    pb.quad_to(control[0], control[1], to[0], to[1])
                }
                PathCommand::Cubic {
                    control1,
                    control2,
                    to,
                } => pb.cubic_to(
                    control1[0],
                    control1[1],
                    control2[0],
                    control2[1],
                    to[0],
                    to[1],
                ),
                PathCommand::Close => pb.close(),
            }
        }
        let shape = pb.finish().context("empty vector geometry")?;
        let mut pixels = self.rgba.clone();
        for row in pixels.chunks_exact_mut(self.width as usize * 4) {
            check()?;
            for p in row.chunks_exact_mut(4) {
                for c in 0..3 {
                    p[c] = ((u16::from(p[c]) * u16::from(p[3]) + 127) / 255) as u8;
                }
            }
        }
        let size =
            tiny_skia::IntSize::from_wh(self.width, self.height).context("vector dimensions")?;
        let mut pixmap = tiny_skia::Pixmap::from_vec(pixels, size).context("vector allocation")?;
        let m = path.transform;
        let transform = tiny_skia::Transform::from_row(m[0], m[1], m[2], m[3], m[4], m[5]);
        let paint = |c: [u8; 4]| {
            let mut p = tiny_skia::Paint::default();
            p.set_color_rgba8(c[0], c[1], c[2], c[3]);
            p.anti_alias = true;
            p
        };
        if let Some(color) = path.fill {
            pixmap.fill_path(
                &shape,
                &paint(color),
                if path.even_odd {
                    tiny_skia::FillRule::EvenOdd
                } else {
                    tiny_skia::FillRule::Winding
                },
                transform,
                None,
            );
        }
        check()?;
        if let Some(stroke) = &path.stroke {
            pixmap.stroke_path(
                &shape,
                &paint(stroke.color),
                &tiny_skia::Stroke {
                    width: stroke.width,
                    ..Default::default()
                },
                transform,
                None,
            );
        }
        check()?;
        let mut rgba = pixmap.take();
        for row in rgba.chunks_exact_mut(self.width as usize * 4) {
            check()?;
            for p in row.chunks_exact_mut(4) {
                for c in 0..3 {
                    p[c] = if p[3] == 0 {
                        0
                    } else {
                        ((u32::from(p[c]) * 255 + u32::from(p[3]) / 2) / u32::from(p[3])).min(255)
                            as u8
                    };
                }
            }
        }
        Ok(Self {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}
