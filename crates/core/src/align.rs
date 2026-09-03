//! Does the device series track the reference at all, and at what offset?
//!
//! Two sensors on two clocks are the normal case, and a device that is 30 s
//! behind will look inaccurate at every epoch while being right about every
//! beat. So before any agreement number, both series are put on a common grid
//! and cross-correlated over a bounded range of lags. The result is either a
//! lag with the evidence for it, or an admission that the evidence does not
//! pick one, in which case no number follows.

use serde::{Deserialize, Serialize};

/// Every threshold that decides between "aligned" and "not evaluated".
/// Recorded with each report, because a lag is a finding only under the
/// policy that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignmentPolicy {
    /// Lags searched, both directions, in seconds.
    pub max_lag_s: f64,
    /// Grid step for interpolation and lag resolution.
    pub grid_s: f64,
    /// Least overlap, in grid points, for a correlation to count.
    pub min_overlap: usize,
    /// Least correlation at the best lag.
    pub min_corr: f64,
    /// The best lag must beat every lag outside `exclusion_s` of it by this.
    pub min_margin: f64,
    /// Half-width, in seconds, of the neighbourhood not counted as a rival.
    pub exclusion_s: f64,
}

impl Default for AlignmentPolicy {
    fn default() -> Self {
        Self { max_lag_s: 120.0, grid_s: 1.0, min_overlap: 60, min_corr: 0.5, min_margin: 0.05, exclusion_s: 20.0 }
    }
}

/// The outcome of the alignment stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Alignment {
    /// One lag clearly explains the two series. `lag_s` is what to add to
    /// device times to land on the reference clock.
    Conclusive { lag_s: f64, corr: f64, runner_up: f64 },
    /// No lag clearly explains them. `why` names the failing criterion.
    Ambiguous { best_lag_s: f64, corr: f64, runner_up: f64, why: String },
}

impl Alignment {
    #[must_use]
    pub fn lag(&self) -> Option<f64> {
        match self {
            Self::Conclusive { lag_s, .. } => Some(*lag_s),
            Self::Ambiguous { .. } => None,
        }
    }
}

/// Linear interpolation of `(t, v)` samples onto `t_query`; `None` outside the
/// sampled support. Samples must be sorted by `t`.
fn interp(samples: &[(f64, f64)], t_query: f64) -> Option<f64> {
    let (first, last) = (samples.first()?, samples.last()?);
    if t_query < first.0 || t_query > last.0 {
        return None;
    }
    let i = samples.partition_point(|s| s.0 < t_query);
    if i == 0 {
        return Some(first.1);
    }
    let (t0, v0) = samples[i - 1];
    let (t1, v1) = samples[i];
    if t1 <= t0 {
        return Some(v1);
    }
    Some(v0 + (v1 - v0) * (t_query - t0) / (t1 - t0))
}

/// Pearson correlation over pairs where both sides are present.
fn pearson(pairs: &[(f64, f64)]) -> Option<f64> {
    let n = pairs.len();
    if n < 2 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)] // n is a count of grid points
    let n_f = n as f64;
    let (mx, my) = pairs.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (mx, my) = (mx / n_f, my / n_f);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in pairs {
        let (dx, dy) = (x - mx, y - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return None;
    }
    Some(sxy / (sxx * syy).sqrt())
}

/// Estimate the lag between a reference and a device rate series.
///
/// Both inputs are `(t_s, bpm)` sorted by time. The search is exhaustive over
/// `[-max_lag_s, +max_lag_s]` in steps of `grid_s`, so the result is the same
/// on every machine and every run.
#[must_use]
pub fn estimate(reference: &[(f64, f64)], device: &[(f64, f64)], p: &AlignmentPolicy) -> Alignment {
    let ambiguous = |best_lag_s: f64, corr: f64, runner_up: f64, why: &str| Alignment::Ambiguous {
        best_lag_s,
        corr,
        runner_up,
        why: why.to_string(),
    };
    let (Some(r0), Some(r1)) = (reference.first(), reference.last()) else {
        return ambiguous(0.0, f64::NAN, f64::NAN, "empty reference");
    };
    if device.is_empty() {
        return ambiguous(0.0, f64::NAN, f64::NAN, "empty device series");
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // bounded by policy
    let steps = (p.max_lag_s / p.grid_s).round() as i64;
    let mut scores: Vec<(f64, Option<f64>)> = Vec::new();
    let mut t = r0.0;
    let mut grid = Vec::new();
    while t <= r1.0 {
        grid.push(t);
        t += p.grid_s;
    }
    #[allow(clippy::cast_precision_loss)]
    for k in -steps..=steps {
        let lag = k as f64 * p.grid_s;
        // device time + lag = reference time  =>  device value at (t - lag)
        let pairs: Vec<(f64, f64)> = grid
            .iter()
            .filter_map(|&tg| Some((interp(reference, tg)?, interp(device, tg - lag)?)))
            .collect();
        let c = if pairs.len() >= p.min_overlap { pearson(&pairs) } else { None };
        scores.push((lag, c));
    }
    let Some(&(best_lag, Some(best))) = scores
        .iter()
        .filter(|(_, c)| c.is_some())
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    else {
        return ambiguous(0.0, f64::NAN, f64::NAN, "no lag has enough overlap or variance to correlate");
    };
    let runner_up = scores
        .iter()
        .filter(|(lag, c)| c.is_some() && (lag - best_lag).abs() > p.exclusion_s)
        .filter_map(|(_, c)| *c)
        .fold(f64::NEG_INFINITY, f64::max);
    let runner_up = if runner_up.is_finite() { runner_up } else { f64::NEG_INFINITY };
    if best < p.min_corr {
        return ambiguous(best_lag, best, runner_up, "best correlation below policy minimum");
    }
    if best_lag.abs() + 1e-9 >= p.max_lag_s {
        return ambiguous(best_lag, best, runner_up, "best lag sits at the edge of the search range");
    }
    if best - runner_up < p.min_margin {
        return ambiguous(best_lag, best, runner_up, "a rival lag correlates almost as well");
    }
    Alignment::Conclusive { lag_s: best_lag, corr: best, runner_up }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values are the point of these tests
mod tests {
    use super::*;

    /// A slowly varying rate with enough structure to have one best lag.
    fn wavy(start: f64, end: f64, step: f64, offset: f64) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let mut t = start;
        while t <= end {
            let x = t + offset;
            let v = 80.0 + 15.0 * (x / 37.0).sin() + 8.0 * (x / 11.0).cos() + 5.0 * (x / 5.0).sin();
            out.push((t, v));
            t += step;
        }
        out
    }

    #[test]
    fn identical_series_align_at_zero_with_unit_correlation() {
        let r = wavy(0.0, 400.0, 1.0, 0.0);
        let a = estimate(&r, &r, &AlignmentPolicy::default());
        match a {
            Alignment::Conclusive { lag_s, corr, .. } => {
                assert_eq!(lag_s, 0.0);
                assert!((corr - 1.0).abs() < 1e-9, "{corr}");
            }
            Alignment::Ambiguous { .. } => panic!("{a:?}"),
        }
    }

    /// The device clock runs 30 s ahead: a value the reference shows at t,
    /// the device stamps at t + 30. The lag to add to device times is -30.
    #[test]
    fn a_known_shift_is_recovered_with_the_right_sign() {
        let r = wavy(0.0, 400.0, 1.0, 0.0);
        let d: Vec<(f64, f64)> = r.iter().map(|(t, v)| (t + 30.0, *v)).collect();
        let a = estimate(&r, &d, &AlignmentPolicy::default());
        assert_eq!(a.lag(), Some(-30.0), "{a:?}");
    }

    /// A true offset beyond the search range must not be reported as the
    /// nearest searched lag: the edge is where the search stopped, not where
    /// the evidence pointed.
    #[test]
    fn a_best_lag_on_the_search_boundary_is_ambiguous() {
        let r = wavy(0.0, 600.0, 1.0, 0.0);
        let p = AlignmentPolicy::default();
        let d: Vec<(f64, f64)> = r.iter().map(|(t, v)| (t + p.max_lag_s, *v)).collect();
        let a = estimate(&r, &d, &p);
        assert!(matches!(&a, Alignment::Ambiguous { why, .. } if why.contains("edge")), "{a:?}");
    }

    #[test]
    fn a_flat_reference_cannot_be_aligned() {
        let r: Vec<(f64, f64)> = (0..300).map(|i| (f64::from(i), 70.0)).collect();
        let d = wavy(0.0, 300.0, 1.0, 0.0);
        assert!(matches!(estimate(&r, &d, &AlignmentPolicy::default()), Alignment::Ambiguous { .. }));
    }

    #[test]
    fn noise_that_does_not_track_is_ambiguous() {
        let r = wavy(0.0, 400.0, 1.0, 0.0);
        // A deterministic pseudo-random series with no relation to r.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let d: Vec<(f64, f64)> = (0..400)
            .map(|i| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                #[allow(clippy::cast_precision_loss)]
                let v = 60.0 + (x % 60) as f64;
                (f64::from(i), v)
            })
            .collect();
        assert!(matches!(estimate(&r, &d, &AlignmentPolicy::default()), Alignment::Ambiguous { .. }));
    }

    #[test]
    fn interpolation_is_none_outside_support() {
        let s = vec![(1.0, 10.0), (3.0, 30.0)];
        assert_eq!(interp(&s, 2.0), Some(20.0));
        assert_eq!(interp(&s, 0.5), None);
        assert_eq!(interp(&s, 3.5), None);
    }
}
