// Copyright (c) 2026 CrySirror contributors. MIT; see LICENSE/SOURCE.json.
//! Native seed/distribution/capture policy over the shared integer RNG kernel.
//! The algorithm belongs to CrySirror VFX; this module owns no PRNG algorithm.
use anyhow::{ensure, Context, Result};
use crysirror_vfx::falling_atlas::SequenceRng;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeRngSnapshot {
    pub seed: u32,
    pub accumulator: u32,
    pub draws: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct NativeRng {
    seed: u32,
    stream: SequenceRng,
    draws: u64,
}
impl NativeRng {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            stream: SequenceRng::msvc(seed),
            draws: 0,
        }
    }
    pub fn rand_bound(&mut self, bound: i32) -> Result<i32> {
        // The supported native profile requires a nonzero signed divisor.
        // Negative divisors retain C signed remainder semantics; no abs/MIN
        // conversion, floating uniform sampling or random hash substitutes it.
        ensure!(
            bound != 0,
            "Majiro rand(0) requires its native error-dialog policy"
        );
        let draws = self
            .draws
            .checked_add(1)
            .context("Majiro RNG draw counter overflow")?;
        let mut candidate = self.stream.clone();
        let native = candidate.next_u32()? as i32;
        let value = native
            .checked_rem(bound)
            .context("Majiro RNG signed remainder")?;
        self.stream = candidate;
        self.draws = draws;
        Ok(value)
    }
    pub fn snapshot(&self) -> NativeRngSnapshot {
        let SequenceRng::Lcg32 { state, .. } = self.stream;
        NativeRngSnapshot {
            seed: self.seed,
            accumulator: state,
            draws: self.draws,
        }
    }
    pub fn from_snapshot(state: NativeRngSnapshot) -> Self {
        // The snapshot contains logical accumulator data, never mutable profile
        // constants. Restoration always selects the reviewed shared CRT kernel.
        Self {
            seed: state.seed,
            stream: SequenceRng::msvc(state.accumulator),
            draws: state.draws,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_modulo_uses_actual_shared_crt_stream_and_restores_exactly() {
        let mut rng = NativeRng::new(1);
        assert_eq!(
            (0..4)
                .map(|_| rng.rand_bound(100).unwrap())
                .collect::<Vec<_>>(),
            [41, 67, 34, 0]
        );
        let state = rng.snapshot();
        assert_eq!(state.draws, 4);
        let bytes = serde_json::to_vec(&state).unwrap();
        let mut restored = NativeRng::from_snapshot(serde_json::from_slice(&bytes).unwrap());
        for bound in [100, -100, i32::MIN, 1, 32768, i32::MAX] {
            assert_eq!(
                rng.rand_bound(bound).unwrap(),
                restored.rand_bound(bound).unwrap()
            );
        }
        let before = rng.snapshot();
        assert!(rng.rand_bound(0).is_err());
        assert_eq!(rng.snapshot(), before);
        let mut overflow = before;
        overflow.draws = u64::MAX;
        let mut overflow = NativeRng::from_snapshot(overflow);
        let before = overflow.snapshot();
        assert!(overflow.rand_bound(100).is_err());
        assert_eq!(overflow.snapshot(), before);
    }
}
