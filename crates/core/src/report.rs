//! Joining corpus, series and claims into the rows the repository exists to
//! produce. Every number here is derived; none is written down anywhere.

use crate::align::{self, Alignment, AlignmentPolicy};
use crate::claim::{Claim, Threshold};
use crate::corpus::{Clock, Corpus, Strength};
use crate::series::{Role, Series};
use crate::stats::{against_maximum, against_minimum, BlandAltman, Mape, Standing, Tolerance, ToleranceRule};
use crate::window::{hr_windows, hrv_windows, Coverage, WindowPolicy};
use serde::{Deserialize, Serialize};

/// Every threshold that shapes a number, recorded with the numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub alignment: AlignmentPolicy,
    pub window: WindowPolicy,
    /// The tolerance rule reported descriptively on every row.
    pub tolerance: ToleranceRule,
    /// Fewer evaluable epochs than this and the row is `not_evaluated`.
    pub min_windows: usize,
    /// z for every interval in the report.
    pub z: f64,
    /// The confidence level `z` stands for, for the reader.
    pub level: String,
    /// Counterfactual switch: treat every recording's clock as shared, so
    /// alignment never gates. Off by default; when on, every output says so.
    #[serde(default)]
    pub assume_shared_clock: bool,
}

impl Policy {
    #[must_use]
    pub fn new(z: f64, level: &str) -> Self {
        Self {
            alignment: AlignmentPolicy::default(),
            window: WindowPolicy::default(),
            tolerance: ToleranceRule { bpm: 5.0, percent: 10.0 },
            min_windows: 10,
            z,
            level: level.to_string(),
            assume_shared_clock: false,
        }
    }
}

/// Rate agreement over the evaluable epochs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agreement {
    pub n: usize,
    pub bland_altman: BlandAltman,
    pub mape: Mape,
    pub tolerance: Tolerance,
}

impl Agreement {
    fn of(pairs: &[(f64, f64)], p: &Policy) -> Option<Self> {
        Some(Self {
            n: pairs.len(),
            bland_altman: BlandAltman::of(pairs, p.z)?,
            mape: Mape::of(pairs, p.z)?,
            tolerance: Tolerance::of(pairs, p.tolerance, p.z)?,
        })
    }
}

/// RMSSD agreement, descriptive only: no claim in the ledger bounds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HrvAgreement {
    pub n: usize,
    pub bland_altman: BlandAltman,
    pub mape: Mape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimVerdict {
    pub claim: String,
    pub standing: Standing,
}

/// What the row concluded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // one per row; the size is the agreement summary itself
pub enum Verdict {
    Evaluated {
        hr: Agreement,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hrv: Option<HrvAgreement>,
        verdicts: Vec<ClaimVerdict>,
    },
    /// The evidence did not reach. No number is manufactured.
    NotEvaluated { why: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub recording: String,
    pub subject: String,
    pub activity: String,
    pub device: String,
    pub method: String,
    pub reference: String,
    pub reference_strength: Strength,
    pub clock: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag_applied_s: Option<f64>,
    pub coverage: Coverage,
    pub verdict: Verdict,
    /// Epoch pairs kept for pooling; not part of the published row.
    #[serde(skip)]
    pub pairs: Vec<(f64, f64)>,
    #[serde(skip)]
    pub hrv_pairs: Vec<(f64, f64)>,
}

impl Row {
    #[must_use]
    pub fn is_control(&self) -> bool {
        self.device.starts_with("control:")
    }
}

/// Rows pooled over recordings for one device, optionally within one
/// activity. Pooling concatenates epochs; only evaluated rows contribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pooled {
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    pub recordings: usize,
    pub verdict: Verdict,
}

fn verdicts_for(pairs: &[(f64, f64)], device: &str, claims: &[Claim], hr: &Agreement, p: &Policy) -> Vec<ClaimVerdict> {
    claims
        .iter()
        .filter(|c| c.is_testable() && c.applies(device))
        .map(|c| {
            let standing = match c.threshold.expect("testable implies threshold") {
                Threshold::MapeAtMost { max } => against_maximum(max, hr.mape.interval),
                Threshold::WithinToleranceAtLeast { bpm, percent, fraction } => {
                    // Counted under the claim's own rule, which need not be the report's.
                    Tolerance::of(pairs, ToleranceRule { bpm, percent }, p.z)
                        .map_or(Standing::Consistent, |t| against_minimum(fraction, t.interval))
                }
            };
            ClaimVerdict { claim: c.id.clone(), standing }
        })
        .collect()
}

fn evaluate(pairs: &[(f64, f64)], hrv_pairs: &[(f64, f64)], device: &str, claims: &[Claim], p: &Policy) -> Verdict {
    if pairs.len() < p.min_windows {
        return Verdict::NotEvaluated {
            why: format!("{} evaluable epoch(s); policy needs {}", pairs.len(), p.min_windows),
        };
    }
    let Some(hr) = Agreement::of(pairs, p) else {
        return Verdict::NotEvaluated { why: "agreement statistics undefined over these epochs".into() };
    };
    let hrv = (hrv_pairs.len() >= 2)
        .then(|| Some(HrvAgreement { n: hrv_pairs.len(), bland_altman: BlandAltman::of(hrv_pairs, p.z)?, mape: Mape::of(hrv_pairs, p.z)? }))
        .flatten();
    let verdicts = verdicts_for(pairs, device, claims, &hr, p);
    Verdict::Evaluated { hr, hrv, verdicts }
}

/// One row per device series. `series` must already have passed
/// `Series::check`; the corpus must already have passed `verify`.
#[must_use]
pub fn build(corpus: &Corpus, series: &[Series], claims: &[Claim], p: &Policy) -> Vec<Row> {
    let mut rows: Vec<Row> = series
        .iter()
        .filter(|s| s.role == Role::Device)
        .filter_map(|dev| {
            let rec = corpus.recording(&dev.recording)?;
            let clock = match (&rec.clock, p.assume_shared_clock) {
                (_, true) => "assumed shared",
                (Clock::Shared { .. }, false) => "shared",
                (Clock::Independent, false) => "independent",
            }
            .to_string();
            let mut row = Row {
                recording: rec.id.clone(),
                subject: rec.subject.clone(),
                activity: rec.activity.clone(),
                device: dev.device.clone(),
                method: dev.method.clone(),
                reference: rec.reference.device.clone(),
                reference_strength: rec.reference.strength(),
                clock,
                alignment: None,
                lag_applied_s: None,
                coverage: Coverage::default(),
                verdict: Verdict::NotEvaluated { why: String::new() },
                pairs: Vec::new(),
                hrv_pairs: Vec::new(),
            };
            let Some(reference) = series.iter().find(|s| s.role == Role::Reference && s.recording == rec.id) else {
                row.verdict = Verdict::NotEvaluated { why: "no reference series for this recording".into() };
                return Some(row);
            };
            if rec.reference.strength() == Strength::Asserted {
                row.verdict = Verdict::NotEvaluated { why: "reference basis is only asserted".into() };
                return Some(row);
            }
            let alignment = align::estimate(&reference.samples.as_rate(), &dev.samples.as_rate(), &p.alignment);
            let clock = if p.assume_shared_clock { &Clock::Shared { basis: String::new() } } else { &rec.clock };
            let lag = match (clock, &alignment) {
                (Clock::Shared { .. }, _) => 0.0,
                (Clock::Independent, Alignment::Conclusive { lag_s, .. }) => *lag_s,
                (Clock::Independent, Alignment::Ambiguous { why, .. }) => {
                    row.verdict = Verdict::NotEvaluated { why: format!("alignment ambiguous: {why}") };
                    row.alignment = Some(alignment);
                    return Some(row);
                }
            };
            row.alignment = Some(alignment);
            row.lag_applied_s = Some(lag);
            let shifted = dev.samples.shifted(lag);
            let (hr, cov) = hr_windows(&reference.samples, &shifted, &p.window);
            let (hrv, _) = hrv_windows(&reference.samples, &shifted, &p.window);
            row.coverage = cov;
            row.pairs = hr.iter().map(|w| (w.reference_bpm, w.device_bpm)).collect();
            row.hrv_pairs = hrv.iter().map(|w| (w.reference_rmssd_ms, w.device_rmssd_ms)).collect();
            row.verdict = evaluate(&row.pairs, &row.hrv_pairs, &dev.device, claims, p);
            Some(row)
        })
        .collect();
    rows.sort_by(|a, b| (&a.recording, &a.device).cmp(&(&b.recording, &b.device)));
    rows
}

/// Pool rows per device and activity, then per device overall.
///
/// A row contributes its epochs whenever it got past alignment, including a
/// row too short for a verdict of its own: the per-row minimum guards against
/// a verdict on thin evidence, not against the epochs being real. A row that
/// failed alignment has no epochs and contributes nothing.
#[must_use]
pub fn pool(rows: &[Row], claims: &[Claim], p: &Policy) -> Vec<Pooled> {
    let evaluated: Vec<&Row> = rows.iter().filter(|r| !r.pairs.is_empty()).collect();
    let mut devices: Vec<&str> = evaluated.iter().map(|r| r.device.as_str()).collect();
    devices.sort_unstable();
    devices.dedup();
    let mut out = Vec::new();
    for device in devices {
        let mine: Vec<&Row> = evaluated.iter().copied().filter(|r| r.device == device).collect();
        let mut activities: Vec<&str> = mine.iter().map(|r| r.activity.as_str()).collect();
        activities.sort_unstable();
        activities.dedup();
        let groups = activities.iter().map(|a| Some(*a)).chain(std::iter::once(None));
        for activity in groups {
            let members: Vec<&Row> = mine.iter().copied().filter(|r| activity.is_none_or(|a| r.activity == a)).collect();
            let pairs: Vec<(f64, f64)> = members.iter().flat_map(|r| r.pairs.iter().copied()).collect();
            let hrv: Vec<(f64, f64)> = members.iter().flat_map(|r| r.hrv_pairs.iter().copied()).collect();
            out.push(Pooled {
                device: device.to_string(),
                activity: activity.map(str::to_string),
                recordings: members.len(),
                verdict: evaluate(&pairs, &hrv, device, claims, p),
            });
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values are the point of these tests
mod tests {
    use super::*;
    use crate::claim::AppliesTo;
    use crate::corpus::{FileEntry, Recording, Reference, ReferenceBasis, Source};
    use crate::series::{InputRef, Samples};
    use crate::stats::Z_95;

    fn corpus(clock: Clock) -> Corpus {
        Corpus {
            sources: vec![Source { id: "s".into(), citation: "c".into(), url: "u".into(), license: "l".into(), retrieved: "d".into() }],
            recordings: vec![Recording {
                id: "r".into(),
                source: "s".into(),
                subject: "1".into(),
                activity: "walk".into(),
                clock,
                reference: Reference { device: "ecg".into(), basis: ReferenceBasis::ManualAnnotation { annotator: "a".into() } },
                files: vec![FileEntry { path: "r.dat".into(), sha256: "abc".into(), upstream_sha256: None }],
                tags: std::collections::BTreeMap::default(),
            }],
        }
    }

    fn wavy_rate(offset_s: f64, scale: f64) -> Vec<(f64, f64)> {
        (0..400)
            .map(|i| {
                let t = f64::from(i);
                let x = t + offset_s;
                (t, scale * (80.0 + 15.0 * (x / 37.0).sin() + 8.0 * (x / 11.0).cos() + 5.0 * (x / 5.0).sin()))
            })
            .collect()
    }

    fn series(role: Role, device: &str, samples: Samples) -> Series {
        Series {
            recording: "r".into(),
            role,
            device: device.into(),
            method: "m".into(),
            adapter: "a".into(),
            inputs: vec![InputRef { path: "r.dat".into(), sha256: "abc".into() }],
            samples,
        }
    }

    fn rate(v: &[(f64, f64)]) -> Samples {
        Samples::Rate { t_s: v.iter().map(|x| x.0).collect(), bpm: v.iter().map(|x| x.1).collect() }
    }

    fn mape_claim(max: f64) -> Claim {
        Claim {
            id: "mape".into(),
            applies_to: AppliesTo::Any,
            quote: "q".into(),
            url: "u".into(),
            retrieved: "d".into(),
            page_sha256: "0".repeat(64),
            derivation: Some("x".into()),
            threshold: Some(Threshold::MapeAtMost { max }),
            note: None,
        }
    }

    #[test]
    fn an_identical_device_beats_the_threshold_at_lag_zero() {
        let r = wavy_rate(0.0, 1.0);
        let s = vec![series(Role::Reference, "ecg", rate(&r)), series(Role::Device, "d", rate(&r))];
        let rows = build(&corpus(Clock::Independent), &s, &[mape_claim(0.10)], &Policy::new(Z_95, "95%"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].lag_applied_s, Some(0.0));
        match &rows[0].verdict {
            Verdict::Evaluated { hr, verdicts, .. } => {
                assert!(hr.mape.mape.abs() < 1e-12);
                assert_eq!(verdicts, &[ClaimVerdict { claim: "mape".into(), standing: Standing::Better }]);
            }
            Verdict::NotEvaluated { why } => panic!("{why}"),
        }
    }

    #[test]
    fn a_device_running_fifteen_percent_fast_is_refuted() {
        let r = wavy_rate(0.0, 1.0);
        let d = wavy_rate(0.0, 1.15);
        let s = vec![series(Role::Reference, "ecg", rate(&r)), series(Role::Device, "d", rate(&d))];
        let rows = build(&corpus(Clock::Shared { basis: "test".into() }), &s, &[mape_claim(0.10)], &Policy::new(Z_95, "95%"));
        match &rows[0].verdict {
            Verdict::Evaluated { verdicts, .. } => assert_eq!(verdicts[0].standing, Standing::Refuted),
            Verdict::NotEvaluated { why } => panic!("{why}"),
        }
    }

    #[test]
    fn an_ambiguous_alignment_under_an_independent_clock_yields_no_number() {
        let r = wavy_rate(0.0, 1.0);
        let flat: Vec<(f64, f64)> = (0..400).map(|i| (f64::from(i), 70.0)).collect();
        let s = vec![series(Role::Reference, "ecg", rate(&r)), series(Role::Device, "d", rate(&flat))];
        let rows = build(&corpus(Clock::Independent), &s, &[], &Policy::new(Z_95, "95%"));
        assert!(matches!(&rows[0].verdict, Verdict::NotEvaluated { why } if why.starts_with("alignment ambiguous")));
        let rows = build(&corpus(Clock::Shared { basis: "test".into() }), &s, &[], &Policy::new(Z_95, "95%"));
        assert!(matches!(&rows[0].verdict, Verdict::Evaluated { .. }), "a shared clock does not gate");
    }

    #[test]
    fn a_shifted_device_is_brought_back_before_windows_are_cut() {
        let r = wavy_rate(0.0, 1.0);
        let d: Vec<(f64, f64)> = r.iter().map(|(t, v)| (t + 30.0, *v)).collect();
        let s = vec![series(Role::Reference, "ecg", rate(&r)), series(Role::Device, "d", rate(&d))];
        let rows = build(&corpus(Clock::Independent), &s, &[], &Policy::new(Z_95, "95%"));
        assert_eq!(rows[0].lag_applied_s, Some(-30.0));
        match &rows[0].verdict {
            Verdict::Evaluated { hr, .. } => assert!(hr.mape.mape < 1e-9, "{}", hr.mape.mape),
            Verdict::NotEvaluated { why } => panic!("{why}"),
        }
    }

    #[test]
    fn a_reported_reference_never_produces_a_verdict() {
        let mut c = corpus(Clock::Shared { basis: "t".into() });
        c.recordings[0].reference.basis = ReferenceBasis::Reported;
        let r = wavy_rate(0.0, 1.0);
        let s = vec![series(Role::Reference, "watch", rate(&r)), series(Role::Device, "d", rate(&r))];
        let rows = build(&c, &s, &[mape_claim(0.10)], &Policy::new(Z_95, "95%"));
        assert!(matches!(&rows[0].verdict, Verdict::NotEvaluated { why } if why.contains("asserted")));
    }

    /// Two 60 s segments are each too short for a verdict (6 epochs < 10) but
    /// together carry 12 epochs, so the pooled row is evaluated.
    #[test]
    fn rows_too_short_for_their_own_verdict_still_pool() {
        let r: Vec<(f64, f64)> = wavy_rate(0.0, 1.0).into_iter().take(61).collect();
        let mut c = corpus(Clock::Shared { basis: "t".into() });
        let mut second = c.recordings[0].clone();
        second.id = "r2".into();
        c.recordings.push(second);
        let mut s2 = series(Role::Reference, "ecg", rate(&r));
        s2.recording = "r2".into();
        let mut d2 = series(Role::Device, "d", rate(&r));
        d2.recording = "r2".into();
        let s = vec![series(Role::Reference, "ecg", rate(&r)), series(Role::Device, "d", rate(&r)), s2, d2];
        let p = Policy::new(Z_95, "95%");
        let rows = build(&c, &s, &[mape_claim(0.10)], &p);
        assert!(rows.iter().all(|r| matches!(&r.verdict, Verdict::NotEvaluated { why } if why.contains("epoch"))), "{rows:?}");
        let pooled = pool(&rows, &[mape_claim(0.10)], &p);
        let overall = pooled.iter().find(|x| x.activity.is_none()).unwrap();
        assert_eq!(overall.recordings, 2);
        assert!(matches!(&overall.verdict, Verdict::Evaluated { hr, .. } if hr.n == 12), "{overall:?}");
    }

    #[test]
    fn pooling_concatenates_epochs_and_marks_the_overall_group() {
        let r = wavy_rate(0.0, 1.0);
        let s = vec![series(Role::Reference, "ecg", rate(&r)), series(Role::Device, "d", rate(&r))];
        let p = Policy::new(Z_95, "95%");
        let rows = build(&corpus(Clock::Shared { basis: "t".into() }), &s, &[], &p);
        let pooled = pool(&rows, &[], &p);
        assert_eq!(pooled.len(), 2);
        assert_eq!(pooled[0].activity.as_deref(), Some("walk"));
        assert_eq!(pooled[1].activity, None);
        assert_eq!(pooled[1].recordings, 1);
    }
}
