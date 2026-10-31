// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Stateful packed block patterns. RNG state is supplied and returned by the
//! caller so several resources can share one native/global integer stream.
//! These CPU kernels are for engine workers; publication/handles stay host-owned.
use crate::{rgba_len, ImageSurface};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternRng {
    pub state: u32,
    pub multiplier: u32,
    pub increment: u32,
    pub output_shift: u8,
    pub output_mask: u32,
    pub draws: u64,
}
impl PatternRng {
    pub fn msvc(state: u32) -> Self {
        Self {
            state,
            multiplier: 214013,
            increment: 2531011,
            output_shift: 16,
            output_mask: 0x7fff,
            draws: 0,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.output_shift < 32 && self.output_mask > 0 && self.output_mask <= 0x7fff,
            "packed pattern RNG output domain"
        );
        Ok(())
    }
    /// Consume one checked draw from the shared stream, independently of any
    /// pattern resource. Native VM policies may use this same state for rnd.
    pub fn next_draw(&mut self) -> Result<u32> {
        self.validate()?;
        self.next()
    }
    fn next(&mut self) -> Result<u32> {
        let draws = self
            .draws
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("pattern RNG draw count exhausted"))?;
        self.state = self
            .state
            .wrapping_mul(self.multiplier)
            .wrapping_add(self.increment);
        self.draws = draws;
        Ok((self.state >> self.output_shift) & self.output_mask)
    }
    fn pair(&mut self) -> Result<u32> {
        Ok(self.next()? | (self.next()? << 16))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalPatternSpec {
    pub size: [u32; 2],
    /// One packed word describes four horizontally adjacent grey cells.
    pub cell: [u32; 2],
}
impl TemporalPatternSpec {
    pub fn validate(&self) -> Result<()> {
        self.word_count().map(|_| ())
    }
    /// Declared packed word bytes, before any worker allocation.
    pub fn state_bytes(&self) -> Result<usize> {
        Ok(self.word_count()? * 4)
    }
    fn word_count(&self) -> Result<usize> {
        rgba_len(self.size[0], self.size[1])?;
        ensure!(
            self.cell.iter().all(|v| (1..=8).contains(v)),
            "pattern cell dimensions"
        );
        ensure!(
            self.size[0] % (4 * self.cell[0]) == 0 && self.size[1] % self.cell[1] == 0,
            "pattern aligned extent"
        );
        Ok((self.size[0] / self.cell[0] / 4) as usize * (self.size[1] / self.cell[1]) as usize)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalPatternSnapshot {
    pub schema: u32,
    pub spec: TemporalPatternSpec,
    pub words: Vec<u32>,
    pub phase: u8,
    pub completed_steps: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporalPattern {
    snapshot: TemporalPatternSnapshot,
}
impl TemporalPattern {
    pub fn new(
        spec: TemporalPatternSpec,
        rng: &mut PatternRng,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        let n = spec.word_count()?;
        rng.validate()?;
        check()?;
        let mut candidate = rng.clone();
        let mut words = Vec::with_capacity(n);
        for i in 0..n {
            if i % 256 == 0 {
                check()?;
            }
            words.push(candidate.pair()?);
        }
        check()?;
        *rng = candidate;
        Ok(Self {
            snapshot: TemporalPatternSnapshot {
                schema: 1,
                spec,
                words,
                phase: 0,
                completed_steps: 0,
            },
        })
    }
    pub fn snapshot(&self) -> TemporalPatternSnapshot {
        self.snapshot.clone()
    }
    pub fn spec(&self) -> &TemporalPatternSpec {
        &self.snapshot.spec
    }
    pub fn resident_bytes(&self) -> usize {
        self.snapshot.words.len() * 4
    }
    pub fn completed_steps(&self) -> u64 {
        self.snapshot.completed_steps
    }
    pub fn from_snapshot(
        snapshot: TemporalPatternSnapshot,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        let n = snapshot.spec.word_count()?;
        ensure!(
            snapshot.schema == 1
                && snapshot.words.len() == n
                && snapshot.phase <= 1
                && u64::from(snapshot.phase) == snapshot.completed_steps % 2,
            "pattern snapshot layout"
        );
        for words in snapshot.words.chunks(256) {
            check()?;
            ensure!(
                words.iter().all(|v| v & 0x8000_8000 == 0),
                "pattern packed word domain"
            );
        }
        check()?;
        Ok(Self { snapshot })
    }
    pub fn restore(
        &mut self,
        snapshot: TemporalPatternSnapshot,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        let candidate = Self::from_snapshot(snapshot, check)?;
        *self = candidate;
        Ok(())
    }
    /// Toggle the updated half-bank, XORing one new pair into each selected
    /// word. Pattern and external global RNG commit together only after success.
    pub fn advance(&mut self, rng: &mut PatternRng, check: &dyn Fn() -> Result<()>) -> Result<()> {
        self.advance_prefix(self.snapshot.words.len(), rng, check)
    }
    /// Update only a bounded prefix of the allocated bank. Zero still toggles
    /// the phase, preserving an empty render's state without consuming RNG.
    pub fn advance_prefix(
        &mut self,
        prefix_words: usize,
        rng: &mut PatternRng,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        ensure!(
            prefix_words <= self.snapshot.words.len(),
            "pattern update prefix bounds"
        );
        rng.validate()?;
        check()?;
        let steps = self
            .snapshot
            .completed_steps
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("pattern step count exhausted"))?;
        let phase = self.snapshot.phase ^ 1;
        let mut words = self.snapshot.words.clone();
        let mut candidate = rng.clone();
        // A trailing odd word is retained, matching a bank of floor(n/2) pairs.
        for i in 0..prefix_words / 2 {
            if i % 256 == 0 {
                check()?;
            }
            words[2 * i + usize::from(phase)] ^= candidate.pair()?;
        }
        check()?;
        self.snapshot.words = words;
        self.snapshot.phase = phase;
        self.snapshot.completed_steps = steps;
        *rng = candidate;
        Ok(())
    }
    /// Composite block greys into a checked region, sampling packed words from
    /// region origin. RGB uses floor(src*opacity/256)+floor(dst*(256-opacity)/256).
    /// Alpha is retained exactly. No decode, device, clock or mutable VM state.
    pub fn composite(
        &self,
        destination: &ImageSurface,
        rect: [u32; 4],
        opacity256: u16,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        destination.validate()?;
        ensure!(
            [destination.width, destination.height] == self.snapshot.spec.size,
            "pattern destination dimensions"
        );
        let [x, y, w, h] = rect;
        let [cx, cy] = self.snapshot.spec.cell;
        ensure!(
            opacity256 <= 256
                && w > 0
                && h > 0
                && w % (4 * cx) == 0
                && h % cy == 0
                && x.checked_add(w).is_some_and(|v| v <= destination.width)
                && y.checked_add(h).is_some_and(|v| v <= destination.height),
            "pattern composite bounds"
        );
        check()?;
        let mut out = destination.clone();
        let opacity = u32::from(opacity256);
        for row in 0..h {
            if row % 32 == 0 {
                check()?;
            }
            for column in 0..w {
                let cell_x = column / cx;
                let index = (row / cy) * (w / (4 * cx)) + cell_x / 4;
                let word = self.snapshot.words[index as usize].wrapping_shl(1);
                let gray = (word >> (8 * (cell_x % 4))) & 255;
                let at = (((y + row) * destination.width + x + column) * 4) as usize;
                for channel in 0..3 {
                    out.rgba[at + channel] = ((gray * opacity >> 8)
                        + (u32::from(destination.rgba[at + channel]) * (256 - opacity) >> 8))
                        as u8;
                }
            }
        }
        check()?;
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefix_updates_keep_other_words_and_empty_render_only_toggles_phase() {
        let mut rng = PatternRng::msvc(1);
        let mut pattern = TemporalPattern::new(
            TemporalPatternSpec {
                size: [16, 8],
                cell: [2, 2],
            },
            &mut rng,
            &|| Ok(()),
        )
        .unwrap();
        let before = pattern.snapshot();
        let draws = rng.draws;
        pattern.advance_prefix(3, &mut rng, &|| Ok(())).unwrap();
        let after = pattern.snapshot();
        assert_eq!(rng.draws, draws + 2);
        for index in 0..before.words.len() {
            if index != 1 {
                assert_eq!(before.words[index], after.words[index]);
            }
        }
        pattern.advance_prefix(0, &mut rng, &|| Ok(())).unwrap();
        assert_eq!(rng.draws, draws + 2);
        assert_eq!(pattern.snapshot().words, after.words);
        assert_eq!(pattern.snapshot().phase, 0);
        let snapshot = pattern.snapshot();
        let random = rng.clone();
        assert!(pattern
            .advance_prefix(snapshot.words.len() + 1, &mut rng, &|| Ok(()))
            .is_err());
        assert_eq!(pattern.snapshot(), snapshot);
        assert_eq!(rng, random);
    }
    use std::cell::Cell;
    #[test]
    fn packed_state_uses_external_stream_alternating_banks_and_exact_blend() {
        let mut rng = PatternRng::msvc(1);
        let mut p = TemporalPattern::new(
            TemporalPatternSpec {
                size: [8, 2],
                cell: [2, 2],
            },
            &mut rng,
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(rng.draws, 2);
        assert_eq!(p.snapshot.words, [41 | (18467 << 16)]);
        let old = p.clone();
        p.advance(&mut rng, &|| Ok(())).unwrap();
        assert_eq!(
            p.snapshot.words, old.snapshot.words,
            "odd tail has no second bank"
        );
        assert_eq!(rng.draws, 2);
        let image = ImageSurface::filled(8, 2, [101, 151, 201, 17]).unwrap();
        let out = p.composite(&image, [0, 0, 8, 2], 76, &|| Ok(())).unwrap();
        let shifted = (41u32 | (18467 << 16)).wrapping_shl(1);
        for x in 0..8 {
            let g = (shifted >> (8 * (x / 2))) & 255;
            let at = x as usize * 4;
            assert_eq!(out.rgba[at], ((g * 76 >> 8) + (101 * 180 >> 8)) as u8);
            assert_eq!(out.rgba[at + 3], 17);
        }
    }
    #[test]
    fn cancel_and_invalid_restore_leave_rng_and_pattern_unchanged() {
        let mut rng = PatternRng::msvc(9);
        let mut p = TemporalPattern::new(
            TemporalPatternSpec {
                size: [16, 2],
                cell: [2, 2],
            },
            &mut rng,
            &|| Ok(()),
        )
        .unwrap();
        let initial = p.clone();
        let initial_rng = rng.clone();
        let checks = Cell::new(0);
        assert!(p
            .advance(&mut rng, &|| {
                let n = checks.get() + 1;
                checks.set(n);
                ensure!(n < 3, "cancelled");
                Ok(())
            })
            .is_err());
        assert_eq!(p, initial);
        assert_eq!(rng, initial_rng);
        p.advance(&mut rng, &|| Ok(())).unwrap();
        assert_ne!(p.snapshot.words[1], initial.snapshot.words[1]);
        assert_eq!(p.snapshot.words[0], initial.snapshot.words[0]);
        let before = p.clone();
        let mut bad = p.snapshot();
        bad.words[0] |= 0x8000;
        assert!(p.restore(bad, &|| Ok(())).is_err());
        assert_eq!(p, before);
        let mut restored = initial.clone();
        restored.restore(p.snapshot(), &|| Ok(())).unwrap();
        assert_eq!(restored, p);
    }
    #[test]
    fn multiple_resources_share_one_global_stream_without_reseeding() {
        let mut rng = PatternRng::msvc(1);
        let a = TemporalPattern::new(
            TemporalPatternSpec {
                size: [8, 2],
                cell: [2, 2],
            },
            &mut rng,
            &|| Ok(()),
        )
        .unwrap();
        let mut b = TemporalPattern::new(
            TemporalPatternSpec {
                size: [16, 2],
                cell: [2, 2],
            },
            &mut rng,
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(rng.draws, 6);
        assert_ne!(a.snapshot.words[0], b.snapshot.words[0]);
        let after_init = rng.clone();
        b.advance(&mut rng, &|| Ok(())).unwrap();
        assert_eq!(rng.draws, 8);
        let before = b.clone();
        let mut invalid = b.snapshot();
        invalid.phase ^= 1;
        assert!(b.restore(invalid, &|| Ok(())).is_err());
        assert_eq!(b, before);
        let restored = TemporalPattern::from_snapshot(b.snapshot(), &|| Ok(())).unwrap();
        assert_eq!(restored, b);
        assert_eq!(after_init.draws, 6);
    }
}
