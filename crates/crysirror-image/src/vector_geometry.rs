// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared vector authoring geometry and exact filled-path bounds.
use crate::vector::{PathCommand, VectorPath};
use anyhow::{ensure, Context, Result};
pub type Affine2 = [f32; 6];
pub const IDENTITY: Affine2 = [1., 0., 0., 1., 0., 0.];
/// Validate finite invertible affine coordinates, shared with VectorPath.
pub fn validate_affine(m: Affine2) -> Result<()> {
    ensure!(
        m.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.),
        "vector transform"
    );
    ensure!(
        (m[0] * m[3] - m[1] * m[2]).abs() > 1e-8,
        "singular vector transform"
    );
    Ok(())
}
/// Apply first, then second. This is GDI+'s row-vector Append order, expressed
/// in the engine's six-value affine convention. No VM policy is involved.
pub fn append_affine(first: Affine2, second: Affine2) -> Result<Affine2> {
    validate_affine(first)?;
    validate_affine(second)?;
    let [a, b, c, d, e, f] = first;
    let [g, h, i, j, k, l] = second;
    let out = [
        g * a + i * b,
        h * a + j * b,
        g * c + i * d,
        h * c + j * d,
        g * e + i * f + k,
        h * e + j * f + l,
    ];
    validate_affine(out)?;
    Ok(out)
}
pub fn translation(x: f32, y: f32) -> Result<Affine2> {
    let m = [1., 0., 0., 1., x, y];
    validate_affine(m)?;
    Ok(m)
}
pub fn scaling(x: f32, y: f32) -> Result<Affine2> {
    let m = [x, 0., 0., y, 0., 0.];
    validate_affine(m)?;
    Ok(m)
}
pub fn rotation(degrees: f32) -> Result<Affine2> {
    ensure!(
        degrees.is_finite() && degrees.abs() <= 1_000_000.,
        "vector angle"
    );
    let (s, c) = degrees.to_radians().sin_cos();
    Ok([c, s, -s, c, 0., 0.])
}
/// Filled rectangle contour. Nonpositive dimensions are an explicit unsupported
/// geometry rather than a fabricated draw/bounds success.
pub fn rectangle([x, y, w, h]: [f32; 4]) -> Result<Vec<PathCommand>> {
    ensure!(
        [x, y, w, h]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1_000_000.)
            && w > 0.
            && h > 0.,
        "vector rectangle"
    );
    polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true)
}
pub fn polygon(points: &[[f32; 2]], closed: bool) -> Result<Vec<PathCommand>> {
    let minimum = if closed { 3 } else { 2 };
    ensure!(
        points.len() >= minimum && points.len() <= 8190,
        "vector point budget"
    );
    ensure!(
        points
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
        "vector point"
    );
    let mut out = vec![PathCommand::Move { to: points[0] }];
    out.extend(points[1..].iter().map(|p| PathCommand::Line { to: *p }));
    if closed {
        out.push(PathCommand::Close)
    }
    Ok(out)
}
/// Append a connected polyline or start a new figure, retaining logical path
/// state separately from raster output. Callers keep the original figure flag.
pub fn append_polyline(
    commands: &mut Vec<PathCommand>,
    points: &[[f32; 2]],
    new_figure: bool,
) -> Result<()> {
    let incoming = polygon(points, false)?;
    ensure!(
        commands.len() + incoming.len() <= 8192,
        "vector command budget"
    );
    if commands.is_empty() || new_figure || matches!(commands.last(), Some(PathCommand::Close)) {
        commands.extend(incoming)
    } else {
        commands.push(PathCommand::Line { to: points[0] });
        commands.extend(incoming.into_iter().skip(1));
    }
    Ok(())
}
pub(crate) fn build_path(commands: &[PathCommand]) -> Result<tiny_skia::Path> {
    let mut b = tiny_skia::PathBuilder::new();
    for c in commands {
        match *c {
            PathCommand::Move { to } => b.move_to(to[0], to[1]),
            PathCommand::Line { to } => b.line_to(to[0], to[1]),
            PathCommand::Quad { control, to } => b.quad_to(control[0], control[1], to[0], to[1]),
            PathCommand::Cubic {
                control1,
                control2,
                to,
            } => b.cubic_to(
                control1[0],
                control1[1],
                control2[0],
                control2[1],
                to[0],
                to[1],
            ),
            PathCommand::Close => b.close(),
        }
    }
    b.finish().context("empty vector geometry")
}
/// Exact axis-aligned filled geometry bounds after the affine transform.
/// Stroke extents are deliberately excluded. Curves use tiny-skia's extrema
/// solver; polygon/rectangle bounds are exact up to f32 arithmetic.
pub fn fill_bounds(commands: &[PathCommand], transform: Affine2) -> Result<[f32; 4]> {
    let p = VectorPath {
        commands: commands.to_vec(),
        fill: Some([0; 4]),
        stroke: None,
        transform,
        even_odd: true,
    };
    p.validate()?;
    let [a, b, c, d, e, f] = transform;
    let path = build_path(commands)?
        .transform(tiny_skia::Transform::from_row(a, b, c, d, e, f))
        .context("transformed vector path")?;
    let r = path.compute_tight_bounds().context("vector bounds")?;
    Ok([r.x(), r.y(), r.width(), r.height()])
}
pub fn union_bounds(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    [
        x,
        y,
        (a[0] + a[2]).max(b[0] + b[2]) - x,
        (a[1] + a[3]).max(b[1] + b[3]) - y,
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authoring_and_append_order_have_independent_geometry_bounds() {
        let p = rectangle([1., 2., 3., 4.]).unwrap();
        let m = append_affine(scaling(2., 3.).unwrap(), translation(10., -4.).unwrap()).unwrap();
        assert_eq!(fill_bounds(&p, m).unwrap(), [12., 2., 6., 12.]);
        assert_eq!(
            union_bounds([1., 2., 3., 4.], [0., 1., 8., 2.]),
            [0., 1., 8., 5.]
        );
        assert!(scaling(0., 1.).is_err());
    }
    #[test]
    fn disconnected_figures_keep_even_odd_fill_and_public_pixels() {
        let mut commands = rectangle([1., 1., 6., 6.]).unwrap();
        commands.extend(rectangle([3., 3., 2., 2.]).unwrap());
        let path = VectorPath {
            commands,
            fill: Some([20, 40, 60, 255]),
            stroke: None,
            transform: IDENTITY,
            even_odd: true,
        };
        let image = crate::ImageSurface::filled(8, 8, [0; 4])
            .unwrap()
            .draw_vector(&path, &|| Ok(()))
            .unwrap();
        assert_eq!(
            &image.rgba[(2 * 8 + 2) * 4..(2 * 8 + 2) * 4 + 4],
            &[20, 40, 60, 255]
        );
        assert_eq!(&image.rgba[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 4], &[0; 4]);
    }

    #[test]
    fn vector_raster_preserves_untouched_straight_pixels_exactly() {
        let original = crate::ImageSurface::filled(4, 4, [101, 37, 19, 128]).unwrap();
        let path = VectorPath {
            commands: rectangle([0., 0., 1., 1.]).unwrap(),
            fill: Some([255, 0, 0, 255]),
            stroke: None,
            transform: IDENTITY,
            even_odd: true,
        };
        let out = original.draw_vector(&path, &|| Ok(())).unwrap();
        assert_eq!(&out.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(
            &out.rgba[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 4],
            &[101, 37, 19, 128]
        );
    }
}
