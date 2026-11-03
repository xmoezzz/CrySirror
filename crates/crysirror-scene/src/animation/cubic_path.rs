//! Immutable natural cubic paths, reusable by 2D/3D animation clients.
use anyhow::{Result, ensure};
#[derive(Clone, Debug)]
pub struct NaturalCubicPath {
    axes: [Vec<f64>; 3],
}
impl NaturalCubicPath {
    pub fn new(points: &[[i32; 3]]) -> Result<Self> {
        ensure!((2..=100).contains(&points.len()), "cubic path point budget");
        Ok(Self {
            axes: std::array::from_fn(|axis| points.iter().map(|p| f64::from(p[axis])).collect()),
        })
    }
    pub fn sample(&self, normalized: f64) -> Result<[i32; 3]> {
        ensure!(normalized.is_finite(), "cubic path sample time");
        Ok(self
            .axes
            .each_ref()
            .map(|values| sample_natural_cubic(values, normalized) as i32))
    }
}
fn sample_natural_cubic(values: &[f64], normalized: f64) -> f64 {
    match values {
        [] => 0.0,
        [value] => *value,
        [first, second] => first + (second - first) * normalized.clamp(0.0, 1.0),
        _ => {
            let segments = values.len() - 1;
            let scaled = normalized.clamp(0.0, 1.0) * segments as f64;
            let segment = (scaled.floor() as usize).min(segments - 1);
            let local = scaled - segment as f64;

            // The target CSpline builds a natural cubic spline independently
            // for x/y/z. Equal knot spacing lets us solve the tridiagonal
            // second-derivative system with this compact Thomas pass.
            let n = values.len();
            let mut lower = vec![0.0; n];
            let mut diagonal = vec![1.0; n];
            let mut upper = vec![0.0; n];
            let mut rhs = vec![0.0; n];
            for index in 1..n - 1 {
                lower[index] = 1.0;
                diagonal[index] = 4.0;
                upper[index] = 1.0;
                rhs[index] = 6.0 * (values[index + 1] - 2.0 * values[index] + values[index - 1]);
            }
            for index in 1..n {
                let factor = lower[index] / diagonal[index - 1];
                diagonal[index] -= factor * upper[index - 1];
                rhs[index] -= factor * rhs[index - 1];
            }
            let mut second = vec![0.0; n];
            for index in (0..n).rev() {
                second[index] = if index + 1 < n {
                    (rhs[index] - upper[index] * second[index + 1]) / diagonal[index]
                } else {
                    rhs[index] / diagonal[index]
                };
            }
            let a = 1.0 - local;
            let b = local;
            a * values[segment]
                + b * values[segment + 1]
                + ((a * a * a - a) * second[segment] + (b * b * b - b) * second[segment + 1]) / 6.0
        }
    }
}
