//! Beat times or reported rates derived from corpus bytes, with the method
//! written down and the inputs named by hash.
//!
//! The crate never derives a series itself. An adapter outside the crate reads
//! the raw signal, does whatever it does, and records how; this module checks
//! that what it produced is coherent and that it was produced from bytes the
//! corpus knows.

use crate::corpus::Corpus;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Reference,
    Device,
}

/// What the series carries. Beats allow heart-rate variability; a reported
/// rate allows only rate agreement, which is what most consumer devices
/// export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Samples {
    Beats {
        /// Beat times in seconds on the recording's clock, non-decreasing.
        t_s: Vec<f64>,
    },
    Rate {
        /// Sample times in seconds, non-decreasing.
        t_s: Vec<f64>,
        /// Reported rate at each time, beats per minute, finite and positive.
        bpm: Vec<f64>,
    },
}

impl Samples {
    #[must_use]
    pub fn times(&self) -> &[f64] {
        match self {
            Self::Beats { t_s } | Self::Rate { t_s, .. } => t_s,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.times().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.times().is_empty()
    }

    /// The series as an instantaneous rate: one `(t, bpm)` per sample. For
    /// beats, the rate `60 / RR` is placed at the second beat of each pair.
    #[must_use]
    pub fn as_rate(&self) -> Vec<(f64, f64)> {
        match self {
            Self::Beats { t_s } => t_s
                .windows(2)
                .filter(|w| w[1] > w[0])
                .map(|w| (w[1], 60.0 / (w[1] - w[0])))
                .collect(),
            Self::Rate { t_s, bpm } => t_s.iter().copied().zip(bpm.iter().copied()).collect(),
        }
    }

    /// Same samples with `lag_s` added to every time, so a device series can
    /// be moved onto the reference clock without touching the original.
    #[must_use]
    pub fn shifted(&self, lag_s: f64) -> Self {
        match self {
            Self::Beats { t_s } => Self::Beats { t_s: t_s.iter().map(|t| t + lag_s).collect() },
            Self::Rate { t_s, bpm } => {
                Self::Rate { t_s: t_s.iter().map(|t| t + lag_s).collect(), bpm: bpm.clone() }
            }
        }
    }
}

/// A corpus file this series was derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRef {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// Must match a `Recording::id`.
    pub recording: String,
    pub role: Role,
    /// The thing under test (or the reference), named so claims can join.
    pub device: String,
    /// How the numbers were obtained, in enough detail to repeat.
    pub method: String,
    /// The program that produced this file, with its version.
    pub adapter: String,
    pub inputs: Vec<InputRef>,
    pub samples: Samples,
}

/// One way a series can fail to be usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flaw {
    UnknownRecording { recording: String },
    /// Derived from bytes the corpus does not name, or names with a
    /// different hash. Such a series could have been made from anything.
    UnknownInput { recording: String, path: PathBuf },
    NoInputs { recording: String },
    TooShort { recording: String, len: usize },
    NotMonotonic { recording: String, index: usize },
    NotFinite { recording: String, index: usize },
    LengthMismatch { recording: String, t: usize, bpm: usize },
    EmptyMethod { recording: String },
}

impl std::fmt::Display for Flaw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownRecording { recording } => write!(f, "{recording}: not in corpus"),
            Self::UnknownInput { recording, path } => {
                write!(f, "{recording}: input {} is not a corpus file with that hash", path.display())
            }
            Self::NoInputs { recording } => write!(f, "{recording}: names no inputs"),
            Self::TooShort { recording, len } => write!(f, "{recording}: only {len} sample(s)"),
            Self::NotMonotonic { recording, index } => write!(f, "{recording}: time goes backwards at {index}"),
            Self::NotFinite { recording, index } => write!(f, "{recording}: non-finite value at {index}"),
            Self::LengthMismatch { recording, t, bpm } => write!(f, "{recording}: {t} times vs {bpm} rates"),
            Self::EmptyMethod { recording } => write!(f, "{recording}: method not recorded"),
        }
    }
}

impl Series {
    /// Coherence and provenance. `None` means usable.
    #[must_use]
    pub fn check(&self, corpus: &Corpus) -> Option<Flaw> {
        let recording = self.recording.clone();
        if corpus.recording(&self.recording).is_none() {
            return Some(Flaw::UnknownRecording { recording });
        }
        if self.method.trim().is_empty() {
            return Some(Flaw::EmptyMethod { recording });
        }
        if self.inputs.is_empty() {
            return Some(Flaw::NoInputs { recording });
        }
        let files = corpus.files();
        for i in &self.inputs {
            match files.get(i.path.as_path()) {
                Some(f) if f.sha256 == i.sha256 => {}
                _ => return Some(Flaw::UnknownInput { recording, path: i.path.clone() }),
            }
        }
        let t = self.samples.times();
        if t.len() < 2 {
            return Some(Flaw::TooShort { recording, len: t.len() });
        }
        if let Some(i) = t.iter().position(|v| !v.is_finite()) {
            return Some(Flaw::NotFinite { recording, index: i });
        }
        if let Some(i) = t.windows(2).position(|w| w[1] < w[0]) {
            return Some(Flaw::NotMonotonic { recording, index: i + 1 });
        }
        if let Samples::Rate { t_s, bpm } = &self.samples {
            if t_s.len() != bpm.len() {
                return Some(Flaw::LengthMismatch { recording, t: t_s.len(), bpm: bpm.len() });
            }
            if let Some(i) = bpm.iter().position(|v| !v.is_finite() || *v <= 0.0) {
                return Some(Flaw::NotFinite { recording, index: i });
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values are the point of these tests
mod tests {
    use super::*;
    use crate::corpus::{Clock, FileEntry, Recording, Reference, ReferenceBasis, Source};

    fn corpus() -> Corpus {
        Corpus {
            sources: vec![Source { id: "s".into(), citation: "c".into(), url: "u".into(), license: "l".into(), retrieved: "d".into() }],
            recordings: vec![Recording {
                id: "r".into(),
                source: "s".into(),
                subject: "1".into(),
                activity: "walk".into(),
                clock: Clock::Independent,
                reference: Reference { device: "ecg".into(), basis: ReferenceBasis::ManualAnnotation { annotator: "a".into() } },
                files: vec![FileEntry { path: "r.dat".into(), sha256: "abc".into(), upstream_sha256: None }],
                tags: std::collections::BTreeMap::default(),
            }],
        }
    }

    fn series(samples: Samples) -> Series {
        Series {
            recording: "r".into(),
            role: Role::Device,
            device: "d".into(),
            method: "m".into(),
            adapter: "a".into(),
            inputs: vec![InputRef { path: "r.dat".into(), sha256: "abc".into() }],
            samples,
        }
    }

    #[test]
    fn a_coherent_series_passes() {
        assert_eq!(series(Samples::Beats { t_s: vec![0.0, 1.0, 2.0] }).check(&corpus()), None);
    }

    #[test]
    fn an_input_with_the_wrong_hash_is_refused() {
        let mut s = series(Samples::Beats { t_s: vec![0.0, 1.0] });
        s.inputs[0].sha256 = "zzz".into();
        assert!(matches!(s.check(&corpus()), Some(Flaw::UnknownInput { .. })));
    }

    #[test]
    fn time_must_not_go_backwards() {
        let s = series(Samples::Beats { t_s: vec![0.0, 2.0, 1.0] });
        assert_eq!(s.check(&corpus()), Some(Flaw::NotMonotonic { recording: "r".into(), index: 2 }));
    }

    #[test]
    fn a_zero_or_nan_rate_is_refused() {
        let s = series(Samples::Rate { t_s: vec![0.0, 1.0], bpm: vec![60.0, 0.0] });
        assert!(matches!(s.check(&corpus()), Some(Flaw::NotFinite { index: 1, .. })));
        let s = series(Samples::Rate { t_s: vec![0.0, 1.0], bpm: vec![60.0, f64::NAN] });
        assert!(matches!(s.check(&corpus()), Some(Flaw::NotFinite { index: 1, .. })));
    }

    #[test]
    fn beats_become_rates_at_the_second_beat() {
        let r = Samples::Beats { t_s: vec![0.0, 1.0, 1.5] }.as_rate();
        assert_eq!(r, vec![(1.0, 60.0), (1.5, 120.0)]);
    }

    #[test]
    fn shifting_moves_time_only() {
        let s = Samples::Rate { t_s: vec![0.0, 1.0], bpm: vec![60.0, 61.0] }.shifted(2.5);
        assert_eq!(s, Samples::Rate { t_s: vec![2.5, 3.5], bpm: vec![60.0, 61.0] });
    }
}
