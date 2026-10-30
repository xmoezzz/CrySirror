// SPDX-License-Identifier: MPL-2.0
//! Device-free, absolute engine-clock sprite sheet sequences for layers and UI.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetSequence {
    /// Number of equally sized horizontal cells, 1 through 1024.
    pub cells: u16,
    /// One cell duration in engine milliseconds (1 through 600000 unless still).
    pub frame_ms: u32,
    /// Repeat, stop at the final cell, ping-pong, or hold the first cell.
    pub mode: SheetLoop,
    /// Absolute start time in the owning engine clock; earlier queries show cell zero.
    pub started_ms: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetLoop {
    Repeat,
    Once,
    PingPong,
    Still,
}
impl SheetSequence {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=1024).contains(&self.cells)
                && (matches!(self.mode, SheetLoop::Still) || (1..=600000).contains(&self.frame_ms)),
            "sprite sequence bounds"
        );
        Ok(())
    }
    pub fn frame(&self, now_ms: u64) -> Result<u16> {
        self.validate()?;
        let cells = u64::from(self.cells);
        let tick = now_ms.saturating_sub(self.started_ms) / u64::from(self.frame_ms.max(1));
        Ok(match self.mode {
            SheetLoop::Repeat => tick % cells,
            SheetLoop::Once => tick.min(cells - 1),
            SheetLoop::Still => 0,
            SheetLoop::PingPong if cells > 1 => {
                let phase = tick % (2 * (cells - 1));
                if phase < cells {
                    phase
                } else {
                    2 * (cells - 1) - phase
                }
            }
            SheetLoop::PingPong => 0,
        } as u16)
    }
    /// Normalized [u0,v0,u1,v1] of the current horizontal cell.
    pub fn uv(&self, now_ms: u64) -> Result<[f32; 4]> {
        let cell = f32::from(self.frame(now_ms)?);
        let count = f32::from(self.cells);
        Ok([cell / count, 0., (cell + 1.) / count, 1.])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_clock_repeat_once_pingpong_and_single_cell() {
        let mut s = SheetSequence {
            cells: 3,
            frame_ms: 10,
            mode: SheetLoop::PingPong,
            started_ms: 100,
        };
        assert_eq!(
            (0..8)
                .map(|n| s.frame(100 + n * 10).unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 1, 0, 1, 2, 1]
        );
        assert_eq!(s.frame(90).unwrap(), 0);
        s.mode = SheetLoop::Once;
        assert_eq!(s.frame(u64::MAX).unwrap(), 2);
        s.mode = SheetLoop::Repeat;
        assert_eq!(s.frame(140).unwrap(), 1);
        s.cells = 1;
        s.mode = SheetLoop::PingPong;
        assert_eq!(s.frame(999).unwrap(), 0);
    }
}
