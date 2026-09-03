//! What is claimed about a device's accuracy, recorded so it can be held
//! still: a verbatim quote, where it was read, when, and the sha256 of the
//! page as retrieved, so a later edit is provable rather than deniable.
//!
//! Two kinds of claim live in one ledger. A vendor's own words about its own
//! device, and a threshold from a standard or from the validation literature
//! that applies to any device. Both are quotes; only the ones that pin a
//! number and show the derivation are testable. "May not be reliable every
//! time" pins nothing, and the report says so rather than inventing a bound.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppliesTo {
    /// A threshold that any device can be held to.
    Any,
    /// Joined to `Series::device` by exact string match.
    Device { name: String },
}

/// The quote reduced to a bound, where it states one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Threshold {
    /// MAPE must not exceed `max` (a fraction: 0.10 is 10%).
    MapeAtMost { max: f64 },
    /// At least `fraction` of epochs must fall within `bpm` or `percent`,
    /// whichever is greater.
    WithinToleranceAtLeast { bpm: f64, percent: f64, fraction: f64 },
}

impl Threshold {
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        match *self {
            Self::MapeAtMost { max } => max.is_finite() && (0.0..=1.0).contains(&max),
            Self::WithinToleranceAtLeast { bpm, percent, fraction } => {
                bpm.is_finite() && bpm >= 0.0 && percent.is_finite() && percent >= 0.0 && (0.0..=1.0).contains(&fraction)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub applies_to: AppliesTo,
    /// The words, verbatim, not paraphrased.
    pub quote: String,
    pub url: String,
    /// When the quote was read.
    pub retrieved: String,
    /// sha256 of the retrieved page.
    pub page_sha256: String,
    /// How `threshold` was read out of `quote`. Required whenever a
    /// threshold is given, because that step is the only interpretive move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<Threshold>,
    /// Anything a reader needs in order not to over-read the claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Claim {
    /// Usable for a verdict only if it pins a bound and shows its working.
    #[must_use]
    pub fn is_testable(&self) -> bool {
        self.threshold.is_some_and(|t| t.is_well_formed()) && self.derivation.is_some()
    }

    #[must_use]
    pub fn applies(&self, device: &str) -> bool {
        match &self.applies_to {
            AppliesTo::Any => true,
            AppliesTo::Device { name } => name == device,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(threshold: Option<Threshold>, derivation: Option<&str>) -> Claim {
        Claim {
            id: "c".into(),
            applies_to: AppliesTo::Any,
            quote: "acceptable error rate ... ±10%".into(),
            url: "https://example.invalid".into(),
            retrieved: "2026-09-02".into(),
            page_sha256: "0".repeat(64),
            derivation: derivation.map(Into::into),
            threshold,
            note: None,
        }
    }

    #[test]
    fn a_bound_with_its_working_is_testable() {
        assert!(claim(Some(Threshold::MapeAtMost { max: 0.10 }), Some("±10% read as MAPE 0.10")).is_testable());
    }

    #[test]
    fn words_without_a_number_are_not_testable() {
        assert!(!claim(None, None).is_testable());
    }

    #[test]
    fn a_bound_without_derivation_is_not_testable() {
        assert!(!claim(Some(Threshold::MapeAtMost { max: 0.10 }), None).is_testable());
    }

    #[test]
    fn out_of_range_bounds_are_rejected() {
        assert!(!claim(Some(Threshold::MapeAtMost { max: 1.5 }), Some("x")).is_testable());
        assert!(!claim(Some(Threshold::WithinToleranceAtLeast { bpm: 5.0, percent: 10.0, fraction: 1.2 }), Some("x")).is_testable());
    }

    #[test]
    fn device_claims_join_by_exact_name() {
        let mut c = claim(None, None);
        c.applies_to = AppliesTo::Device { name: "Apple Watch 4".into() };
        assert!(c.applies("Apple Watch 4"));
        assert!(!c.applies("Apple Watch"));
        assert!(claim(None, None).applies("anything"));
    }
}
