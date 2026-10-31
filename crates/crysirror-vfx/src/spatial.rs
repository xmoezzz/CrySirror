// SPDX-License-Identifier: MPL-2.0
//! Shared f64 VFX spatial sampling, planar geometry and matrix transforms.
//! Arithmetic order from siglus_rs RealLive DLL translation at
//! 5f358ef1360b0439e92aed2b749e8f25d8721954 (MPL-2.0), dll.rs and
//! pt00/base.rs. Native bank layouts, angle constants and game policy are absent.
//! X-ordered segment clamping is an explicit numeric compatibility profile;
//! it preserves the original degenerate/vertical ordering, not an epsilon rule.

/// `sub_100019A0`: normalises (x, y); zero stays zero.
pub fn normalize(x: &mut f64, y: &mut f64) {
    let length = (*y * *y + *x * *x).sqrt();
    if length == 0.0 {
        *x = 0.0;
        *y = 0.0;
    } else {
        *x /= length;
        *y /= length;
    }
}

/// `sub_100019F0`.
pub fn length3(x: f64, y: f64, z: f64) -> f64 {
    (x * x + y * y + z * z).sqrt()
}

/// `sub_10001A10`: distance between (x1, y1) and (x2, y2).
pub fn distance(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    ((y2 - y1) * (y2 - y1) + (x2 - x1) * (x2 - x1)).sqrt()
}

/// Closest-point result: the point, the vector from the query point to
/// it, and its length.
#[derive(Debug, Clone, Copy, Default)]
pub struct Foot {
    pub x: f64,
    pub y: f64,
    pub dx: f64,
    pub dy: f64,
    pub distance: f64,
}

/// `sub_10001A40`: perpendicular foot of (px, py) on the line through
/// (x1, y1)-(x2, y2).
pub fn line_foot_rotated(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    px: f64,
    py: f64,
    normal_rotation: f64,
) -> Foot {
    let c = normal_rotation.cos();
    let s = normal_rotation.sin();
    let mut nx = (x2 - x1) * c - (y2 - y1) * s;
    let mut ny = s * (x2 - x1) + (y2 - y1) * c;
    normalize(&mut nx, &mut ny);
    let d = (px - x1) * nx + (py - y1) * ny;
    let x = px - d * nx;
    let y = py - d * ny;
    Foot {
        x,
        y,
        dx: x - px,
        dy: y - py,
        distance: d.abs(),
    }
}

/// `sub_10001AF0`: closest point of the segment (x1, y1)-(x2, y2).
pub fn segment_foot_x_ordered(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    px: f64,
    py: f64,
    normal_rotation: f64,
) -> Foot {
    let end = |x: f64, y: f64| {
        let (dx, dy) = (x - px, y - py);
        Foot {
            x,
            y,
            dx,
            dy,
            distance: length3(dx, dy, 0.0),
        }
    };
    // (The degenerate-segment result is overwritten by the general case,
    // exactly as in the DLL.)
    let foot = line_foot_rotated(x1, y1, x2, y2, px, py, normal_rotation);
    let fx = foot.x;
    let first = if x1 < fx {
        x2 <= x1
    } else {
        x1 < x2 || x1 <= fx
    };
    if first {
        return end(x1, y1);
    }
    let second = if fx > x2 {
        if x1 >= x2 {
            return foot;
        }
        true
    } else {
        x2 <= x1 || x2 <= fx
    };
    if second {
        end(x2, y2)
    } else {
        foot
    }
}

/// `sub_10001CD0`: intersection of the lines through (a1, a2)-(a3, a4)
/// and (a5, a6)-(a7, a8), with the unit vector and distance from
/// (a5, a6) to it.
pub fn line_intersection(a: [f64; 8]) -> Option<Foot> {
    let [a1, a2, a3, a4, a5, a6, a7, a8] = a;
    let m1 = if a1 != a3 { (a4 - a2) / (a3 - a1) } else { 0.0 };
    let m2 = if a5 == a7 { 0.0 } else { (a8 - a6) / (a7 - a5) };
    let (x, y) = if a1 != a3 {
        let x = if a5 == a7 {
            a5
        } else {
            if m1 == m2 {
                return None;
            }
            (m1 * a1 - a2 - m2 * a5 + a6) / (m1 - m2)
        };
        (x, (x - a1) * m1 + a2)
    } else if a5 != a7 {
        (a1, (a1 - a5) * m2 + a6)
    } else {
        return None;
    };
    let (dx0, dy0) = (x - a5, y - a6);
    let (mut dx, mut dy) = (dx0, dy0);
    normalize(&mut dx, &mut dy);
    Some(Foot {
        x,
        y,
        dx,
        dy,
        distance: (dy0 * dy0 + dx0 * dx0).sqrt(),
    })
}

/// `sub_10001E40`: segment intersection.
pub fn segment_intersection(a: [f64; 8]) -> Option<Foot> {
    let [a1, a2, a3, a4, a5, a6, a7, a8] = a;
    let foot = line_intersection(a)?;
    let (x, y) = (foot.x, foot.y);
    let outside = |lo: f64, hi: f64, v: f64| (lo > v || v > hi) && (hi > v || v > lo);
    if outside(a1, a3, x) || outside(a2, a4, y) || outside(a5, a7, x) || outside(a6, a8, y) {
        return None;
    }
    Some(foot)
}

pub type Mat3 = [f64; 9];

pub const IDENTITY: Mat3 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// `sub_100023C0`.
pub fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            m[3 * r + c] = a[3 * r] * b[c] + a[3 * r + 1] * b[3 + c] + a[3 * r + 2] * b[6 + c];
        }
    }
    m
}

/// `sub_100024D0` (x), `sub_10002560` (y), `sub_10002600` (z).
pub fn rotate_x(m: &Mat3, a: f64) -> Mat3 {
    let (c, s) = (a.cos(), a.sin());
    mat_mul(m, &[1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c])
}

pub fn rotate_y(m: &Mat3, a: f64) -> Mat3 {
    let (c, s) = (a.cos(), a.sin());
    mat_mul(m, &[c, 0.0, s, 0.0, 1.0, 0.0, -s, 0.0, c])
}

pub fn rotate_z(m: &Mat3, a: f64) -> Mat3 {
    let (c, s) = (a.cos(), a.sin());
    mat_mul(m, &[c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0])
}

/// `sub_10002690`.
pub fn transform(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[2] * v[2] + m[1] * v[1] + m[0] * v[0],
        m[3] * v[0] + m[5] * v[2] + m[4] * v[1],
        m[6] * v[0] + m[8] * v[2] + m[7] * v[1],
    ]
}

/// Uniform cubic B-spline over four successive scalar control points.
/// The parameter is not clamped: callers may deliberately extrapolate.
pub fn uniform_cubic_bspline(points: [f64; 4], t: f64) -> f64 {
    let a = t * t * t / 6.0;
    let b = (-t * t * t + 3.0 * t * t - 3.0 * t + 1.0) / 6.0;
    let c = (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0;
    let d = (-3.0 * t * t * t + 3.0 * t * t + 3.0 * t + 1.0) / 6.0;
    a * points[3] + d * points[2] + c * points[1] + b * points[0]
}

/// Independent per-axis focal lengths over an explicit perspective denominator.
/// Zero/NaN denominators follow IEEE arithmetic; singularity policy belongs to
/// the declaring scene/adapter rather than silently changing the projection.
pub fn perspective_xy(point: [f64; 2], focal: [f64; 2], denominator: f64) -> [f64; 2] {
    [
        point[0] * focal[0] / denominator,
        point[1] * focal[1] / denominator,
    ]
}

/// Height of a parabolic arc: `height` at t=0 and zero at t=span.
pub fn parabolic_arc(t: f64, span: f64, height: f64) -> f64 {
    if span == 0.0 {
        0.0
    } else {
        (1.0 - t * t / (span * span)) * height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }
    #[test]
    fn spline_reproduces_constant_and_linear_paths_with_extrapolation() {
        for t in [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0] {
            close(uniform_cubic_bspline([7.; 4], t), 7.);
            close(uniform_cubic_bspline([0., 1., 2., 3.], t), 1. + t);
        }
    }
    #[test]
    fn axis_focals_and_matrix_composition_are_independent_of_native_banks() {
        assert_eq!(perspective_xy([3., -2.], [800., 700.], 100.), [24., -14.]);
        assert!(perspective_xy([1., 1.], [1., 1.], 0.)[0].is_infinite());
        let rotated = rotate_z(&IDENTITY, std::f64::consts::FRAC_PI_2);
        let v = transform(&rotated, [1., 0., 3.]);
        close(v[0], 0.);
        close(v[1], 1.);
        close(v[2], 3.);
        close(parabolic_arc(5., 10., 100.), 75.);
    }
    #[test]
    fn geometric_queries_keep_exact_parallel_and_endpoint_profiles() {
        let normal = -std::f64::consts::FRAC_PI_2;
        let foot = line_foot_rotated(0., 0., 10., 0., 5., 3., normal);
        close(foot.x, 5.);
        close(foot.y, 0.);
        close(foot.distance, 3.);
        let foot = segment_foot_x_ordered(0., 0., 10., 0., 12., 3., normal);
        assert_eq!([foot.x, foot.y], [10., 0.]);
        let hit = line_intersection([0., 0., 10., 10., 0., 10., 10., 0.]).unwrap();
        assert_eq!([hit.x, hit.y], [5., 5.]);
        assert!(line_intersection([0., 0., 10., 10., 0., 1., 10., 11.]).is_none());
    }
}
