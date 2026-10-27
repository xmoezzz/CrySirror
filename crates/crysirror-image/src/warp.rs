// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Convex projective image mapping. Inverse homography, center sampling and
//! premultiplied bilinear filtering; outside the destination quad is transparent.
use crate::ImageSurface;
use anyhow::{ensure, Result};
fn solve<const N: usize>(mut a: Vec<Vec<f64>>) -> Result<Vec<f64>> {
    for c in 0..N {
        let pivot = (c..N)
            .max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))
            .unwrap();
        ensure!(
            a[pivot][c].abs() > 1e-12,
            "degenerate perspective transform"
        );
        a.swap(pivot, c);
        let d = a[c][c];
        for k in c..a[c].len() {
            a[c][k] /= d;
        }
        for i in 0..N {
            if i != c {
                let d = a[i][c];
                for k in c..a[i].len() {
                    a[i][k] -= d * a[c][k];
                }
            }
        }
    }
    Ok(a.into_iter().flat_map(|r| r.into_iter().skip(N)).collect())
}
pub fn validate_quad(quad: &[[f32; 2]; 4]) -> Result<()> {
    ensure!(
        quad.iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= 65536.),
        "perspective coordinate"
    );
    let mut signs = vec![];
    for i in 0..4 {
        let a = quad[i];
        let b = quad[(i + 1) % 4];
        let c = quad[(i + 2) % 4];
        let cross = f64::from(b[0] - a[0]) * f64::from(c[1] - b[1])
            - f64::from(b[1] - a[1]) * f64::from(c[0] - b[0]);
        ensure!(cross.abs() > 1e-6, "degenerate perspective quad");
        signs.push(cross > 0.);
    }
    ensure!(
        signs.iter().all(|v| *v == signs[0]),
        "perspective quad must be convex and ordered"
    );
    Ok(())
}
impl ImageSurface {
    pub fn perspective(&self, quad: [[f32; 2]; 4], check: &dyn Fn() -> Result<()>) -> Result<Self> {
        self.validate()?;
        validate_quad(&quad)?;
        ensure!(
            u64::from(self.width) * u64::from(self.height) <= crate::effects::MAX_EFFECT_PIXELS,
            "perspective pixel budget"
        );
        let mut rows = vec![];
        for ([s, t], [x, y]) in [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            .into_iter()
            .zip(quad.map(|p| p.map(f64::from)))
        {
            rows.push(vec![s, t, 1., 0., 0., 0., -x * s, -x * t, x]);
            rows.push(vec![0., 0., 0., s, t, 1., -y * s, -y * t, y]);
        }
        let h = solve::<8>(rows)?;
        let m = solve::<3>(vec![
            vec![h[0], h[1], h[2], 1., 0., 0.],
            vec![h[3], h[4], h[5], 0., 1., 0.],
            vec![h[6], h[7], 1., 0., 0., 1.],
        ])?;
        let mut out = Self::filled(self.width, self.height, [0; 4])?;
        for y in 0..self.height {
            check()?;
            for x in 0..self.width {
                let px = f64::from(x) + 0.5;
                let py = f64::from(y) + 0.5;
                let d = m[6] * px + m[7] * py + m[8];
                if d.abs() < 1e-12 {
                    continue;
                }
                let u = (m[0] * px + m[1] * py + m[2]) / d;
                let v = (m[3] * px + m[4] * py + m[5]) / d;
                if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                    continue;
                }
                let sx = u * f64::from(self.width) - 0.5;
                let sy = v * f64::from(self.height) - 0.5;
                let x0 = sx.floor();
                let y0 = sy.floor();
                let tx = sx - x0;
                let ty = sy - y0;
                let mut sum = [0.; 4];
                for (dy, wy) in [(0, 1. - ty), (1, ty)] {
                    for (dx, wx) in [(0, 1. - tx), (1, tx)] {
                        let xx = (x0 as i64 + dx).clamp(0, i64::from(self.width) - 1) as u32;
                        let yy = (y0 as i64 + dy).clamp(0, i64::from(self.height) - 1) as u32;
                        let at = ((yy * self.width + xx) * 4) as usize;
                        let p = &self.rgba[at..at + 4];
                        let a = f64::from(p[3]) / 255.;
                        for c in 0..3 {
                            sum[c] += f64::from(p[c]) * a * wx * wy;
                        }
                        sum[3] += f64::from(p[3]) * wx * wy;
                    }
                }
                let at = ((y * self.width + x) * 4) as usize;
                out.rgba[at + 3] = sum[3].round().clamp(0., 255.) as u8;
                for c in 0..3 {
                    out.rgba[at + c] = if sum[3] > 0. {
                        (sum[c] * 255. / sum[3]).round().clamp(0., 255.) as u8
                    } else {
                        0
                    };
                }
            }
        }
        Ok(out)
    }
}
