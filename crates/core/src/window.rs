//! Fixed epochs where both sides have enough data to state a rate.
//!
//! An epoch is dropped, and counted as dropped, whenever either side is thin.
//! Coverage is reported next to every number so a high agreement over a
//! quarter of the recording reads as exactly that.

use crate::series::Samples;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowPolicy {
    /// Epoch length for rate agreement.
    pub hr_window_s: f64,
    /// Epoch length for RMSSD agreement (beats only).
    pub hrv_window_s: f64,
    /// Least beats in an epoch to state a rate from beats.
    pub min_beats: usize,
    /// Least reported samples in an epoch to state a rate from a rate series.
    pub min_rate_samples: usize,
    /// Least RR intervals in an epoch to state an RMSSD.
    pub min_rr: usize,
}

impl Default for WindowPolicy {
    fn default() -> Self {
        Self { hr_window_s: 10.0, hrv_window_s: 60.0, min_beats: 3, min_rate_samples: 2, min_rr: 10 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HrWindow {
    pub start_s: f64,
    pub reference_bpm: f64,
    pub device_bpm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HrvWindow {
    pub start_s: f64,
    pub reference_rmssd_ms: f64,
    pub device_rmssd_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Coverage {
    /// Epochs in the overlap of the two series.
    pub windows: usize,
    /// Epochs where both sides stated a value.
    pub evaluable: usize,
    pub dropped_reference: usize,
    pub dropped_device: usize,
}

fn in_window(t: &[f64], start: f64, end: f64) -> &[f64] {
    let lo = t.partition_point(|&x| x < start);
    let hi = t.partition_point(|&x| x < end);
    &t[lo..hi]
}

/// Mean rate over an epoch, or `None` when the side is too thin.
fn epoch_rate(s: &Samples, start: f64, end: f64, p: &WindowPolicy) -> Option<f64> {
    match s {
        Samples::Beats { t_s } => {
            let b = in_window(t_s, start, end);
            if b.len() < p.min_beats {
                return None;
            }
            let span = b[b.len() - 1] - b[0];
            #[allow(clippy::cast_precision_loss)]
            let intervals = (b.len() - 1) as f64;
            (span > 0.0).then(|| 60.0 * intervals / span)
        }
        Samples::Rate { t_s, bpm } => {
            let lo = t_s.partition_point(|&x| x < start);
            let hi = t_s.partition_point(|&x| x < end);
            let v = &bpm[lo..hi];
            if v.len() < p.min_rate_samples {
                return None;
            }
            #[allow(clippy::cast_precision_loss)]
            let n = v.len() as f64;
            Some(v.iter().sum::<f64>() / n)
        }
    }
}

/// Root mean square of successive RR differences, in ms.
fn epoch_rmssd(s: &Samples, start: f64, end: f64, p: &WindowPolicy) -> Option<f64> {
    let Samples::Beats { t_s } = s else { return None };
    let b = in_window(t_s, start, end);
    let rr: Vec<f64> = b.windows(2).map(|w| (w[1] - w[0]) * 1000.0).collect();
    if rr.len() < p.min_rr {
        return None;
    }
    let sq: f64 = rr.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    #[allow(clippy::cast_precision_loss)]
    let n = (rr.len() - 1) as f64;
    Some((sq / n).sqrt())
}

/// The overlap of two series, on a common clock, cut into epochs of `len_s`.
fn epochs(reference: &Samples, device: &Samples, len_s: f64) -> Vec<f64> {
    let (rt, dt) = (reference.times(), device.times());
    let (Some(&r0), Some(&r1), Some(&d0), Some(&d1)) = (rt.first(), rt.last(), dt.first(), dt.last()) else {
        return Vec::new();
    };
    let start = r0.max(d0);
    let end = r1.min(d1);
    let mut out = Vec::new();
    let mut t = start;
    while t + len_s <= end {
        out.push(t);
        t += len_s;
    }
    out
}

/// Rate agreement epochs. `device` must already be on the reference clock.
#[must_use]
pub fn hr_windows(reference: &Samples, device: &Samples, p: &WindowPolicy) -> (Vec<HrWindow>, Coverage) {
    let mut cov = Coverage::default();
    let mut out = Vec::new();
    for start in epochs(reference, device, p.hr_window_s) {
        cov.windows += 1;
        let end = start + p.hr_window_s;
        match (epoch_rate(reference, start, end, p), epoch_rate(device, start, end, p)) {
            (Some(r), Some(d)) => {
                cov.evaluable += 1;
                out.push(HrWindow { start_s: start, reference_bpm: r, device_bpm: d });
            }
            (None, _) => cov.dropped_reference += 1,
            (_, None) => cov.dropped_device += 1,
        }
    }
    (out, cov)
}

/// RMSSD agreement epochs; empty unless both sides carry beats.
#[must_use]
pub fn hrv_windows(reference: &Samples, device: &Samples, p: &WindowPolicy) -> (Vec<HrvWindow>, Coverage) {
    let mut cov = Coverage::default();
    let mut out = Vec::new();
    if !matches!((reference, device), (Samples::Beats { .. }, Samples::Beats { .. })) {
        return (out, cov);
    }
    for start in epochs(reference, device, p.hrv_window_s) {
        cov.windows += 1;
        let end = start + p.hrv_window_s;
        match (epoch_rmssd(reference, start, end, p), epoch_rmssd(device, start, end, p)) {
            (Some(r), Some(d)) => {
                cov.evaluable += 1;
                out.push(HrvWindow { start_s: start, reference_rmssd_ms: r, device_rmssd_ms: d });
            }
            (None, _) => cov.dropped_reference += 1,
            (_, None) => cov.dropped_device += 1,
        }
    }
    (out, cov)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values are the point of these tests
mod tests {
    use super::*;

    fn beats(bpm: f64, until: f64) -> Samples {
        let rr = 60.0 / bpm;
        let mut t = 0.0;
        let mut v = Vec::new();
        while t <= until {
            v.push(t);
            t += rr;
        }
        Samples::Beats { t_s: v }
    }

    #[test]
    fn a_steady_60_bpm_reads_as_60_in_every_epoch() {
        let r = beats(60.0, 100.0);
        let (w, cov) = hr_windows(&r, &r, &WindowPolicy::default());
        assert_eq!(cov.windows, 10);
        assert_eq!(cov.evaluable, 10);
        for x in &w {
            assert!((x.reference_bpm - 60.0).abs() < 1e-9 && (x.device_bpm - 60.0).abs() < 1e-9, "{x:?}");
        }
    }

    #[test]
    fn a_thin_device_epoch_is_dropped_and_counted() {
        let r = beats(60.0, 100.0);
        // Device reports twice inside [10, 20) and once or never elsewhere: one evaluable epoch.
        let d = Samples::Rate { t_s: vec![0.0, 12.0, 13.0, 100.0], bpm: vec![60.0, 60.0, 60.0, 60.0] };
        let (w, cov) = hr_windows(&r, &d, &WindowPolicy::default());
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].start_s, 10.0);
        assert_eq!(cov.windows, 10);
        assert_eq!(cov.dropped_device, 9);
    }

    #[test]
    fn rmssd_of_a_perfectly_regular_rhythm_is_zero_and_needs_beats_on_both_sides() {
        let r = beats(60.0, 130.0);
        let (w, cov) = hrv_windows(&r, &r, &WindowPolicy::default());
        assert_eq!(cov.evaluable, 2);
        assert!(w.iter().all(|x| x.reference_rmssd_ms.abs() < 1e-9));
        let d = Samples::Rate { t_s: vec![0.0, 130.0], bpm: vec![60.0, 60.0] };
        assert!(hrv_windows(&r, &d, &WindowPolicy::default()).0.is_empty());
    }

    #[test]
    fn rmssd_matches_a_hand_computation() {
        // RR (ms): 1000, 1100, 900, 1000, ... differences 100, -200, 100, ...
        let t_s = vec![0.0, 1.0, 2.1, 3.0, 4.0, 5.1, 6.0, 7.0, 8.1, 9.0, 10.0, 11.1, 11.9, 12.5];
        let s = Samples::Beats { t_s };
        let p = WindowPolicy { hrv_window_s: 12.0, min_rr: 5, ..WindowPolicy::default() };
        let (w, _) = hrv_windows(&s, &s, &p);
        assert_eq!(w.len(), 1);
        // The epoch [0, 12) holds the first 13 beats; 12.5 lies outside it.
        let rr: Vec<f64> = s.times()[..13].windows(2).map(|x| (x[1] - x[0]) * 1000.0).collect();
        let sq: f64 = rr.windows(2).map(|x| (x[1] - x[0]).powi(2)).sum();
        #[allow(clippy::cast_precision_loss)]
        let expect = (sq / (rr.len() - 1) as f64).sqrt();
        assert!((w[0].reference_rmssd_ms - expect).abs() < 1e-9);
    }
}
