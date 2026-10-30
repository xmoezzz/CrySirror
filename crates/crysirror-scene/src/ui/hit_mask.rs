// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Immutable engine-owned pointer coverage, independent of a VM or renderer.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
/// One byte per logical widget pixel. Transparent samples permit pointer input
/// to reach lower widgets; keyboard focus and activation do not sample this mask.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetHitMask {
    /// Width and height of the local, unscaled coverage grid.
    pub size: [u32; 2],
    /// Row-major immutable coverage bytes, shared across staged scene copies.
    pub coverage: Arc<[u8]>,
    /// A pointer is eligible when its coverage is at least this byte.
    pub threshold: u8,
    /// Exclude the local top and left edges as well as the normal right/bottom edges.
    #[serde(default)]
    pub strict_bounds: bool,
}
impl WidgetHitMask {
    pub(crate) fn validate(&self, rect: [f32; 4]) -> Result<()> {
        let [w, h] = self.size;
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "UI hit mask dimensions"
        );
        let len = u64::from(w) * u64::from(h);
        ensure!(
            len <= 16 * 1024 * 1024 && len == self.coverage.len() as u64,
            "UI hit mask coverage size"
        );
        ensure!(
            rect[2].ceil() == w as f32 && rect[3].ceil() == h as f32,
            "UI hit mask must match logical widget extent"
        );
        Ok(())
    }
    pub(crate) fn hit(&self, x: i32, y: i32, rect: [f32; 4]) -> bool {
        let x = x as f32 - rect[0];
        let y = y as f32 - rect[1];
        if x < 0.
            || y < 0.
            || x >= rect[2]
            || y >= rect[3]
            || (self.strict_bounds && (x <= 0. || y <= 0.))
        {
            return false;
        }
        let x = x.floor() as u32;
        let y = y.floor() as u32;
        if x >= self.size[0] || y >= self.size[1] {
            return false;
        }
        let index = u64::from(y) * u64::from(self.size[0]) + u64::from(x);
        self.coverage
            .get(index as usize)
            .is_some_and(|&v| v >= self.threshold)
    }
}
