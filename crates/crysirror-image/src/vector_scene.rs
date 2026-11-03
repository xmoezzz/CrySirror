//! Shared premultiplied CPU vector painter. All scene resources and operations
//! are usable independently of a compatibility VM or a GPU backend.
use crate::ImageSurface;
use anyhow::{Context, Result, ensure};
pub use crysirror_core::vector_scene::*;
use std::collections::BTreeMap;
use tiny_skia::{
    BlendMode, Color, FilterQuality, GradientStop, Mask, MaskType, Paint, Pixmap, PixmapPaint,
    Point, Shader, SpreadMode, Transform,
};
mod filters;
const IDENTITY: [f32; 6] = [1., 0., 0., 1., 0., 0.];
fn transform(m: [f32; 6]) -> Transform {
    Transform::from_row(m[0], m[1], m[2], m[3], m[4], m[5])
}
fn concat(a: [f32; 6], b: [f32; 6]) -> Result<[f32; 6]> {
    let result = [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ];
    ensure!(
        result
            .iter()
            .all(|n| n.is_finite() && n.abs() <= 1_000_000_000.0),
        "vector inherited transform bound"
    );
    Ok(result)
}
fn color(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}
fn spread(mode: VectorSpread) -> SpreadMode {
    match mode {
        VectorSpread::Pad => SpreadMode::Pad,
        VectorSpread::Repeat => SpreadMode::Repeat,
        VectorSpread::Reflect => SpreadMode::Reflect,
    }
}
fn quality(smooth: bool) -> FilterQuality {
    if smooth {
        FilterQuality::Bilinear
    } else {
        FilterQuality::Nearest
    }
}
fn premultiply(
    size: [u32; 2],
    mut rgba: Vec<u8>,
    check: &dyn Fn() -> Result<()>,
) -> Result<Pixmap> {
    ensure!(
        crate::rgba_len(size[0], size[1])? == rgba.len(),
        "vector bitmap byte layout"
    );
    for block in rgba.chunks_mut(262144) {
        check()?;
        for p in block.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = ((u16::from(p[c]) * u16::from(p[3]) + 127) / 255) as u8;
            }
        }
    }
    Pixmap::from_vec(
        rgba,
        tiny_skia::IntSize::from_wh(size[0], size[1]).context("vector bitmap dimensions")?,
    )
    .context("vector pixmap allocation")
}
fn paint<'a>(brush: &VectorBrush, bitmaps: &'a BTreeMap<u32, Pixmap>) -> Result<Paint<'a>> {
    let stops = |stops: &[VectorGradientStop]| {
        stops
            .iter()
            .map(|s| GradientStop::new(s.offset, color(s.color)))
            .collect()
    };
    let shader: Shader<'a> = match brush {
        VectorBrush::Solid { color: c } => Shader::SolidColor(color(*c)),
        VectorBrush::LinearGradient {
            start,
            end,
            stops: s,
            spread: mode,
            transform: m,
        } => tiny_skia::LinearGradient::new(
            Point::from_xy(start[0], start[1]),
            Point::from_xy(end[0], end[1]),
            stops(s),
            spread(*mode),
            transform(*m),
        )
        .context("vector linear gradient geometry")?,
        VectorBrush::RadialGradient {
            center,
            focal,
            radius,
            stops: s,
            spread: mode,
            transform: m,
        } => tiny_skia::RadialGradient::new(
            Point::from_xy(focal[0], focal[1]),
            0.,
            Point::from_xy(center[0], center[1]),
            *radius,
            stops(s),
            spread(*mode),
            transform(*m),
        )
        .context("vector radial gradient geometry")?,
        VectorBrush::Bitmap {
            id,
            transform: m,
            spread: mode,
            smoothing,
        } => tiny_skia::Pattern::new(
            bitmaps
                .get(id)
                .context("vector bitmap brush missing")?
                .as_ref(),
            spread(*mode),
            quality(*smoothing),
            1.,
            transform(*m),
        ),
    };
    Ok(Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    })
}
fn blend_mode(mode: VectorBlend) -> BlendMode {
    match mode {
        VectorBlend::SourceOver => BlendMode::SourceOver,
        VectorBlend::Multiply => BlendMode::Multiply,
        VectorBlend::Screen => BlendMode::Screen,
        VectorBlend::Lighten => BlendMode::Lighten,
        VectorBlend::Darken => BlendMode::Darken,
        VectorBlend::Difference => BlendMode::Difference,
        VectorBlend::Add => BlendMode::Plus,
        VectorBlend::Alpha => BlendMode::DestinationIn,
        VectorBlend::Erase => BlendMode::DestinationOut,
        VectorBlend::Overlay => BlendMode::Overlay,
        VectorBlend::HardLight => BlendMode::HardLight,
        VectorBlend::Subtract | VectorBlend::Invert => unreachable!("custom vector blending"),
    }
}
fn composite(
    destination: &mut Pixmap,
    source: &Pixmap,
    opacity: f32,
    blend: VectorBlend,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    if !matches!(blend, VectorBlend::Subtract | VectorBlend::Invert) {
        destination.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &PixmapPaint {
                opacity,
                blend_mode: blend_mode(blend),
                ..PixmapPaint::default()
            },
            Transform::identity(),
            None,
        );
        return Ok(());
    }
    // Separable straight-colour blend with Porter-Duff source-over coverage.
    for (dst, src) in destination
        .data_mut()
        .chunks_mut(262144)
        .zip(source.data().chunks(262144))
    {
        check()?;
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let sa = f32::from(s[3]) / 255. * opacity;
            let da = f32::from(d[3]) / 255.;
            for c in 0..3 {
                let sp = f32::from(s[c]) / 255. * opacity;
                let dp = f32::from(d[c]) / 255.;
                let cs = if sa > 0. { sp / sa } else { 0. };
                let cd = if da > 0. { dp / da } else { 0. };
                let mixed = match blend {
                    VectorBlend::Subtract => (cd - cs).max(0.),
                    VectorBlend::Invert => 1. - cd,
                    _ => unreachable!(),
                };
                d[c] = (((1. - sa) * dp + (1. - da) * sp + sa * da * mixed) * 255.)
                    .round()
                    .clamp(0., 255.) as u8;
            }
            d[3] = ((sa + da - sa * da) * 255.).round().clamp(0., 255.) as u8;
        }
    }
    Ok(())
}
struct Painter<'a> {
    size: [u32; 2],
    bitmaps: BTreeMap<u32, Pixmap>,
    check: &'a dyn Fn() -> Result<()>,
}
impl Painter<'_> {
    fn empty(&self) -> Result<Pixmap> {
        Pixmap::new(self.size[0], self.size[1]).context("vector group allocation")
    }
    fn draw(&self, target: &mut Pixmap, nodes: &[VectorNode], parent: [f32; 6]) -> Result<()> {
        for node in nodes {
            (self.check)()?;
            match node {
                VectorNode::Path {
                    commands,
                    transform: m,
                    fill,
                    stroke,
                    even_odd,
                } => {
                    let path = crate::vector_geometry::build_path(commands)?;
                    let m = transform(concat(parent, *m)?);
                    if let Some(brush) = fill {
                        target.fill_path(
                            &path,
                            &paint(brush, &self.bitmaps)?,
                            if *even_odd {
                                tiny_skia::FillRule::EvenOdd
                            } else {
                                tiny_skia::FillRule::Winding
                            },
                            m,
                            None,
                        );
                    }
                    if let Some(stroke) = stroke {
                        target.stroke_path(
                            &path,
                            &paint(&stroke.brush, &self.bitmaps)?,
                            &tiny_skia::Stroke {
                                width: stroke.width,
                                miter_limit: stroke.miter_limit,
                                line_cap: match stroke.cap {
                                    VectorLineCap::Butt => tiny_skia::LineCap::Butt,
                                    VectorLineCap::Round => tiny_skia::LineCap::Round,
                                    VectorLineCap::Square => tiny_skia::LineCap::Square,
                                },
                                line_join: match stroke.join {
                                    VectorLineJoin::Miter => tiny_skia::LineJoin::Miter,
                                    VectorLineJoin::Round => tiny_skia::LineJoin::Round,
                                    VectorLineJoin::Bevel => tiny_skia::LineJoin::Bevel,
                                },
                                dash: None,
                            },
                            m,
                            None,
                        );
                    }
                }
                VectorNode::Bitmap {
                    id,
                    transform: m,
                    smoothing,
                    opacity,
                } => {
                    target.draw_pixmap(
                        0,
                        0,
                        self.bitmaps
                            .get(id)
                            .context("vector bitmap missing")?
                            .as_ref(),
                        &PixmapPaint {
                            opacity: *opacity,
                            quality: quality(*smoothing),
                            ..PixmapPaint::default()
                        },
                        transform(concat(parent, *m)?),
                        None,
                    );
                }
                VectorNode::Group {
                    nodes,
                    transform: m,
                    opacity,
                    blend,
                    color_transform,
                    filters: effects,
                    clip,
                    mask,
                } => {
                    let inherited = concat(parent, *m)?;
                    let mut group = self.empty()?;
                    self.draw(&mut group, nodes, inherited)?;
                    if let Some(c) = color_transform {
                        filters::color_transform(&mut group, *c, self.check)?;
                    }
                    for effect in effects {
                        group = filters::apply(group, effect, self.check)?;
                    }
                    if let Some([x, y, w, h]) = clip {
                        let mut coverage = Mask::new(self.size[0], self.size[1])
                            .context("vector clip allocation")?;
                        if let Some(rect) = tiny_skia::Rect::from_xywh(*x, *y, *w, *h) {
                            coverage.fill_path(
                                &tiny_skia::PathBuilder::from_rect(rect),
                                tiny_skia::FillRule::Winding,
                                true,
                                transform(inherited),
                            );
                        }
                        group.apply_mask(&coverage);
                    }
                    if let Some(spec) = mask {
                        let mut surface = self.empty()?;
                        self.draw(&mut surface, &spec.nodes, inherited)?;
                        let mut coverage = Mask::from_pixmap(
                            surface.as_ref(),
                            match spec.kind {
                                VectorMaskKind::Alpha => MaskType::Alpha,
                                VectorMaskKind::Luminance => MaskType::Luminance,
                            },
                        );
                        if spec.invert {
                            for block in coverage.data_mut().chunks_mut(262144) {
                                (self.check)()?;
                                for a in block {
                                    *a = 255 - *a;
                                }
                            }
                        }
                        group.apply_mask(&coverage);
                    }
                    composite(target, &group, *opacity, *blend, self.check)?;
                }
            }
        }
        Ok(())
    }
}
/// Rasterise one validated scene into an immutable-resource-ready straight RGBA
/// image. Scratch/output admission is the caller's task ownership boundary.
pub fn render(spec: &VectorSceneSpec, check: &dyn Fn() -> Result<()>) -> Result<ImageSurface> {
    spec.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
    check()?;
    let mut bitmaps = BTreeMap::new();
    for (&id, resource) in &spec.bitmaps {
        check()?;
        bitmaps.insert(
            id,
            premultiply(resource.size(), resource.rgba().to_vec(), check)?,
        );
    }
    let painter = Painter {
        size: spec.size,
        bitmaps,
        check,
    };
    let mut target = painter.empty()?;
    target.fill(color(spec.clear));
    painter.draw(&mut target, &spec.nodes, IDENTITY)?;
    check()?;
    Ok(ImageSurface {
        width: spec.size[0],
        height: spec.size[1],
        rgba: target.take_demultiplied(),
    })
}
