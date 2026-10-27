// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Whole-pixel sine row copying. Parameter semantics checked against
//! wamsoft/layerExRaster a4c9be0c7510cca552e27c0c3e931b2eaee399af.
use crate::ImageSurface;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
/// Whole-image copy profile. Unlike sampling distortion, uncovered destination
/// pixels retain their old values. Displacement truncates toward zero; the phase
/// advances per row from -omega*time/cycle*(image_height/2), with integer height/2.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SineRowCopy {
    pub amplitude: i32,
    pub lines: i32,
    pub cycle: i32,
    pub time: i64,
}
impl ImageSurface {
    pub fn sine_copy_from(
        &self,
        source: &Self,
        spec: &SineRowCopy,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        self.validate()?;
        source.validate()?;
        ensure!(
            u64::from(self.width) * u64::from(self.height) <= crate::effects::MAX_EFFECT_PIXELS,
            "row-copy pixel budget"
        );
        if [self.width, self.height] != [source.width, source.height]
            || spec.lines == 0
            || spec.cycle == 0
        {
            return Ok(self.clone());
        }
        let omega = std::f64::consts::TAU / f64::from(spec.lines);
        let mut phase =
            -omega * spec.time as f64 / f64::from(spec.cycle) * f64::from(self.height / 2);
        let mut out = self.clone();
        let width = self.width as usize;
        for y in 0..self.height as usize {
            check()?;
            let shift = (phase.sin() * f64::from(spec.amplitude)) as i32;
            phase += omega;
            let count = i64::from(self.width) - i64::from(shift).abs();
            if count <= 0 {
                continue;
            }
            let sx = (-i64::from(shift)).max(0) as usize;
            let dx = i64::from(shift).max(0) as usize;
            let from = (y * width + sx) * 4;
            let to = (y * width + dx) * 4;
            let len = count as usize * 4;
            out.rgba[to..to + len].copy_from_slice(&source.rgba[from..from + len]);
        }
        Ok(out)
    }
}
