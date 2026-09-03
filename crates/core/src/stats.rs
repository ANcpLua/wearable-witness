//! Agreement statistics with intervals, because a point estimate over thirty
//! epochs is not a finding.
//!
//! Three views of the same differences, each answering a different question:
//! Bland-Altman (is there a bias, and how wide is the scatter), MAPE (the
//! figure the validation literature reports), and the tolerance fraction (how
//! often the device was within a stated bound), which is a proportion and so
//! gets a Wilson interval rather than the normal approximation that produces
//! intervals crossing zero.
//!
//! Epochs from one recording are not independent; the intervals here treat
//! them as if they were, which makes every interval narrower than it should
//! be and every refutation easier than it should be. That is the wrong
//! direction for a tool whose ambiguities are supposed to favour the device,
//! and it is pinned as an open item in the README rather than hidden.

use serde::{Deserialize, Serialize};

/// z for a two-sided 95% interval.
pub const Z_95: f64 = 1.959_964;
/// z for a two-sided 99% interval.
pub const Z_99: f64 = 2.575_829;

/// Wilson score interval for `successes` out of `n`, clamped to `[0, 1]`.
/// `n == 0` yields `(0.0, 1.0)`: no observations, no information.
#[must_use]
pub fn wilson(successes: u64, n: u64, z: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    #[allow(clippy::cast_precision_loss)] // counts of epochs, far below 2^53
    let (n_f, p) = (n as f64, successes as f64 / n as f64);
    let z2 = z * z;
    let denom = 1.0 + z2 / n_f;
    let center = (p + z2 / (2.0 * n_f)) / denom;
    let margin = (z / denom) * (p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)).sqrt();
    ((center - margin).max(0.0), (center + margin).min(1.0))
}

fn mean(v: &[f64]) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = v.len() as f64;
    v.iter().sum::<f64>() / n
}

/// Sample standard deviation (n - 1).
fn sd(v: &[f64], m: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = (v.len() - 1) as f64;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n).sqrt()
}

/// Bland-Altman summary of `device - reference` differences.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlandAltman {
    pub n: usize,
    /// Mean difference, device minus reference.
    pub bias: f64,
    pub sd: f64,
    /// `bias ± z·sd`.
    pub loa: (f64, f64),
}

impl BlandAltman {
    /// `None` below two pairs: no scatter can be stated from one.
    #[must_use]
    pub fn of(pairs: &[(f64, f64)], z: f64) -> Option<Self> {
        if pairs.len() < 2 {
            return None;
        }
        let d: Vec<f64> = pairs.iter().map(|(r, dv)| dv - r).collect();
        let bias = mean(&d);
        let sd = sd(&d, bias);
        Some(Self { n: d.len(), bias, sd, loa: (bias - z * sd, bias + z * sd) })
    }
}

/// Mean absolute percentage error with a normal-approximation interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mape {
    pub n: usize,
    /// As a fraction, not a percentage: 0.10 is 10%.
    pub mape: f64,
    /// `mape ± z·sd/√n`, lower bound clamped at zero.
    pub interval: (f64, f64),
}

impl Mape {
    #[must_use]
    pub fn of(pairs: &[(f64, f64)], z: f64) -> Option<Self> {
        if pairs.len() < 2 || pairs.iter().any(|(r, _)| *r <= 0.0) {
            return None;
        }
        let e: Vec<f64> = pairs.iter().map(|(r, d)| (d - r).abs() / r).collect();
        let m = mean(&e);
        #[allow(clippy::cast_precision_loss)]
        let half = z * sd(&e, m) / (e.len() as f64).sqrt();
        Some(Self { n: e.len(), mape: m, interval: ((m - half).max(0.0), m + half) })
    }
}

/// A tolerance rule of the "±10% or ±5 bpm, whichever is greater" shape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToleranceRule {
    pub bpm: f64,
    pub percent: f64,
}

impl ToleranceRule {
    #[must_use]
    pub fn admits(&self, reference: f64, device: f64) -> bool {
        (device - reference).abs() <= self.bpm.max(self.percent / 100.0 * reference)
    }
}

/// Fraction of epochs inside a tolerance rule, with a Wilson interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    pub rule: ToleranceRule,
    pub within: u64,
    pub n: u64,
    pub fraction: f64,
    pub interval: (f64, f64),
}

impl Tolerance {
    #[must_use]
    pub fn of(pairs: &[(f64, f64)], rule: ToleranceRule, z: f64) -> Option<Self> {
        if pairs.is_empty() {
            return None;
        }
        let within = pairs.iter().filter(|(r, d)| rule.admits(*r, *d)).count() as u64;
        let n = pairs.len() as u64;
        #[allow(clippy::cast_precision_loss)]
        let fraction = within as f64 / n as f64;
        Some(Self { rule, within, n, fraction, interval: wilson(within, n, z) })
    }
}

/// Where an interval sits relative to a stated bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// The whole interval is on the wrong side of the bound.
    Refuted,
    /// The bound falls inside the interval: not settled either way.
    Consistent,
    /// The whole interval is on the right side of the bound.
    Better,
}

/// Hold "at most `max`" against an interval. `Refuted` only when the *lower*
/// bound already exceeds the maximum.
#[must_use]
pub fn against_maximum(max: f64, interval: (f64, f64)) -> Standing {
    if interval.0 > max {
        Standing::Refuted
    } else if interval.1 < max {
        Standing::Better
    } else {
        Standing::Consistent
    }
}

/// Hold "at least `min`" against an interval. `Refuted` only when the *upper*
/// bound is already below the minimum.
#[must_use]
pub fn against_minimum(min: f64, interval: (f64, f64)) -> Standing {
    if interval.1 < min {
        Standing::Refuted
    } else if interval.0 > min {
        Standing::Better
    } else {
        Standing::Consistent
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values are the point of these tests
mod tests {
    use super::*;

    #[test]
    fn zero_observations_carry_no_information() {
        assert_eq!(wilson(0, 0, Z_95), (0.0, 1.0));
    }

    #[test]
    fn a_clean_run_does_not_prove_perfection() {
        let (lo, hi) = wilson(80, 80, Z_95);
        assert!(hi >= 1.0 - 1e-12 && lo < 1.0 && lo > 0.94, "{lo} {hi}");
    }

    #[test]
    fn wilson_stays_ordered_and_in_range() {
        for n in [1u64, 7, 30, 500] {
            for k in 0..=n {
                let (lo, hi) = wilson(k, n, Z_99);
                assert!((0.0..=1.0).contains(&lo) && (0.0..=1.0).contains(&hi) && lo <= hi);
            }
        }
    }

    #[test]
    fn bland_altman_matches_hand_arithmetic() {
        // differences: +2, -2, +4, 0 → bias 1, sd sqrt(((1)^2+(-3)^2+(3)^2+(-1)^2)/3)=sqrt(20/3)
        let pairs = [(100.0, 102.0), (100.0, 98.0), (100.0, 104.0), (100.0, 100.0)];
        let ba = BlandAltman::of(&pairs, Z_95).unwrap();
        assert!((ba.bias - 1.0).abs() < 1e-12);
        assert!((ba.sd - (20.0_f64 / 3.0).sqrt()).abs() < 1e-12);
        assert!((ba.loa.1 - (1.0 + Z_95 * ba.sd)).abs() < 1e-12);
        assert!(BlandAltman::of(&pairs[..1], Z_95).is_none());
    }

    #[test]
    fn mape_is_a_fraction_and_refuses_a_zero_reference() {
        let pairs = [(100.0, 110.0), (100.0, 90.0), (200.0, 200.0)];
        let m = Mape::of(&pairs, Z_95).unwrap();
        assert!((m.mape - (0.1 + 0.1 + 0.0) / 3.0).abs() < 1e-12);
        assert!(m.interval.0 >= 0.0 && m.interval.0 <= m.mape && m.mape <= m.interval.1);
        assert!(Mape::of(&[(0.0, 1.0), (1.0, 1.0)], Z_95).is_none());
    }

    #[test]
    fn the_tolerance_rule_takes_the_larger_of_the_two_bounds() {
        let rule = ToleranceRule { bpm: 5.0, percent: 10.0 };
        assert!(rule.admits(40.0, 45.0)); // 10% of 40 is 4, so 5 bpm governs
        assert!(!rule.admits(40.0, 45.1));
        assert!(rule.admits(150.0, 165.0)); // 10% of 150 is 15, so percent governs
        assert!(!rule.admits(150.0, 165.1));
    }

    #[test]
    fn tolerance_fraction_carries_a_wilson_interval() {
        let rule = ToleranceRule { bpm: 5.0, percent: 10.0 };
        let pairs: Vec<(f64, f64)> = (0..20).map(|i| (100.0, if i < 15 { 102.0 } else { 130.0 })).collect();
        let t = Tolerance::of(&pairs, rule, Z_95).unwrap();
        assert_eq!((t.within, t.n), (15, 20));
        assert_eq!(t.interval, wilson(15, 20, Z_95));
    }

    #[test]
    fn a_maximum_is_refuted_only_when_the_lower_bound_clears_it() {
        assert_eq!(against_maximum(0.10, (0.12, 0.20)), Standing::Refuted);
        assert_eq!(against_maximum(0.10, (0.08, 0.12)), Standing::Consistent);
        assert_eq!(against_maximum(0.10, (0.02, 0.06)), Standing::Better);
        assert_eq!(against_maximum(0.10, (0.10, 0.20)), Standing::Consistent);
    }

    #[test]
    fn a_minimum_is_refuted_only_when_the_upper_bound_misses_it() {
        assert_eq!(against_minimum(0.95, (0.60, 0.90)), Standing::Refuted);
        assert_eq!(against_minimum(0.95, (0.90, 0.97)), Standing::Consistent);
        assert_eq!(against_minimum(0.95, (0.96, 1.00)), Standing::Better);
    }
}
