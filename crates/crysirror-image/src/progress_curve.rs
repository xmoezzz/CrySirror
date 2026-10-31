// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Bounded device-free progress envelopes. Native adapters map their parameter
//! encodings to explicit knots/shapes; every other engine client can reuse the
//! same double-precision evaluation without a VM, host clock or RNG.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
/// One caller-clock position and normalized envelope value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressKnot {
    /// Caller-selected time units, independent of wall-clock time.
    pub at: u64,
    /// Finite normalized value in0..=1.
    pub value: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgressShape {
    Linear,
    QuadraticIn,
    SquareRoot,
    HalfCosine { pi: f64 },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressCurve {
    /// At most64 ordered knots; duplicate times form right-continuous steps.
    pub knots: Vec<ProgressKnot>,
    pub shape: ProgressShape,
}
impl ProgressCurve {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.knots.is_empty() && self.knots.len() <= 64,
            "progress curve knot budget"
        );
        ensure!(
            self.knots
                .iter()
                .all(|p| p.value.is_finite() && (0.0..=1.0).contains(&p.value)),
            "progress curve finite unit values"
        );
        ensure!(
            self.knots.windows(2).all(|p| p[0].at <= p[1].at),
            "progress curve ordered time"
        );
        if let ProgressShape::HalfCosine { pi } = self.shape {
            ensure!(
                pi.is_finite() && (0.0..=16.0).contains(&pi),
                "progress curve cosine parameter"
            );
        }
        Ok(())
    }
    /// Outside the knots, retain the nearest endpoint. Repeated times are
    /// right-continuous steps: the last knot at an exact time wins. Sampling
    /// never advances time, consumes random numbers or changes this object.
    pub fn sample(&self, time: u64) -> Result<f64> {
        self.validate()?;
        let after = self.knots.partition_point(|p| p.at <= time);
        let raw = if after == 0 {
            self.knots[0].value
        } else if after == self.knots.len() {
            self.knots[after - 1].value
        } else {
            let a = self.knots[after - 1];
            let b = self.knots[after];
            a.value + (b.value - a.value) * ((time - a.at) as f64 / (b.at - a.at) as f64)
        }
        .clamp(0.0, 1.0);
        Ok(match self.shape {
            ProgressShape::Linear => raw,
            ProgressShape::QuadraticIn => raw * raw,
            ProgressShape::SquareRoot => raw.sqrt(),
            ProgressShape::HalfCosine { pi } => (1.0 - (raw * pi).cos()) * 0.5,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_envelope_steps_and_f64_shapes_do_not_own_clock_or_rng() {
        let mut c = ProgressCurve {
            knots: vec![
                ProgressKnot { at: 0, value: 0.0 },
                ProgressKnot { at: 60, value: 1.0 },
            ],
            shape: ProgressShape::QuadraticIn,
        };
        assert_eq!(c.sample(15).unwrap(), 0.0625);
        assert_eq!(c.sample(60).unwrap(), 1.0);
        c.shape = ProgressShape::SquareRoot;
        assert_eq!(c.sample(15).unwrap(), 0.5);
        c.shape = ProgressShape::HalfCosine { pi: 3.14159 };
        assert!((c.sample(30).unwrap() - 0.4999993366025517).abs() < 1e-14);
        let s = ProgressCurve {
            knots: vec![
                ProgressKnot { at: 0, value: 1.0 },
                ProgressKnot { at: 50, value: 1.0 },
                ProgressKnot { at: 50, value: 0.0 },
            ],
            shape: ProgressShape::Linear,
        };
        assert_eq!(s.sample(0).unwrap(), 1.0);
        assert_eq!(s.sample(49).unwrap(), 1.0);
        assert_eq!(s.sample(50).unwrap(), 0.0);
        assert_eq!(s.sample(51).unwrap(), 0.0);
        assert_eq!(
            serde_json::from_slice::<ProgressCurve>(&serde_json::to_vec(&s).unwrap()).unwrap(),
            s
        );
    }
    #[test]
    fn invalid_values_order_budget_and_shapes_fail_before_sampling() {
        let mut c = ProgressCurve {
            knots: vec![],
            shape: ProgressShape::Linear,
        };
        assert!(c.sample(0).is_err());
        c.knots = vec![
            ProgressKnot { at: 1, value: 0.0 },
            ProgressKnot { at: 0, value: 1.0 },
        ];
        assert!(c.validate().is_err());
        c.knots = vec![ProgressKnot {
            at: 0,
            value: f64::NAN,
        }];
        assert!(c.validate().is_err());
        c.knots = vec![ProgressKnot { at: 0, value: 0.0 }; 65];
        assert!(c.validate().is_err());
        c.knots.truncate(1);
        c.shape = ProgressShape::HalfCosine { pi: f64::INFINITY };
        assert!(c.validate().is_err());
    }
}
