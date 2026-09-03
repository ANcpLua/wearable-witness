//! The corpus: recordings with receipts, plus the integrity check over them.
//!
//! A recording is one subject doing one activity while wearing a reference
//! sensor and at least one device under test. The raw files are named here
//! with the sha256 of the bytes that were measured; `verify` is a
//! precondition of `report`, not a courtesy command.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a batch of recordings came from, in a form a reader can follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub citation: String,
    pub url: String,
    pub license: String,
    /// When the bytes were retrieved.
    pub retrieved: String,
}

/// One raw file, relative to the corpus root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    /// sha256 of the bytes on disk.
    pub sha256: String,
    /// sha256 as published by the upstream (a `SHA256SUMS` file, say), when it
    /// publishes one. Equal to `sha256` when the download was clean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_sha256: Option<String>,
}

/// Whether the reference and the device share a time base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Clock {
    /// The dataset authors attest that rows are time-synchronised. The
    /// alignment estimate is reported as a check but does not gate.
    Shared {
        /// Where the attestation can be read.
        basis: String,
    },
    /// Nothing attests a common clock. Alignment must be conclusive before
    /// any agreement number is produced.
    Independent,
}

/// How the reference heart rate was obtained. This is the ground truth the
/// tool accepts, tiered so that a weak reference can never produce a verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReferenceBasis {
    /// Beats marked by a human on an ECG trace.
    ManualAnnotation { annotator: String },
    /// A named algorithm over a clinical-grade ECG.
    Algorithm { name: String },
    /// Reported by a device whose method is not documented.
    Reported,
}

/// Strength of a reference, ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    /// Not evidence. Reported, never judged.
    Asserted,
    /// A documented method over a clinical signal.
    Documented,
    /// Human-marked beats on an ECG.
    Proof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub device: String,
    pub basis: ReferenceBasis,
}

impl Reference {
    #[must_use]
    pub fn strength(&self) -> Strength {
        match self.basis {
            ReferenceBasis::ManualAnnotation { .. } => Strength::Proof,
            ReferenceBasis::Algorithm { .. } => Strength::Documented,
            ReferenceBasis::Reported => Strength::Asserted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recording {
    /// Stable identifier; series refer to recordings by this.
    pub id: String,
    pub source: String,
    pub subject: String,
    pub activity: String,
    pub clock: Clock,
    pub reference: Reference,
    pub files: Vec<FileEntry>,
    /// Free-form attributes a source publishes per recording (a skin-tone
    /// scale, a protocol round). Carried through to the report, never used
    /// by it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Corpus {
    pub sources: Vec<Source>,
    pub recordings: Vec<Recording>,
}

/// One way the corpus can fail to be usable as evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Defect {
    Missing { id: String, path: PathBuf },
    Unreadable { id: String, path: PathBuf, why: String },
    HashMismatch { id: String, path: PathBuf, declared: String, actual: String },
    /// The bytes on disk match the index but not what upstream published.
    UpstreamMismatch { id: String, path: PathBuf },
    DuplicateId { id: String },
    UnknownSource { id: String, source: String },
}

impl std::fmt::Display for Defect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { id, path } => write!(f, "{id}: file not found: {}", path.display()),
            Self::Unreadable { id, path, why } => write!(f, "{id}: unreadable {}: {why}", path.display()),
            Self::HashMismatch { id, path, declared, actual } => write!(
                f,
                "{id}: sha256 mismatch on {}: declared {}…, actual {}…",
                path.display(),
                &declared[..declared.len().min(16)],
                &actual[..actual.len().min(16)]
            ),
            Self::UpstreamMismatch { id, path } => {
                write!(f, "{id}: {} differs from the upstream-published hash", path.display())
            }
            Self::DuplicateId { id } => write!(f, "{id}: id used more than once"),
            Self::UnknownSource { id, source } => write!(f, "{id}: source {source} is not declared"),
        }
    }
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().fold(String::with_capacity(64), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

impl Corpus {
    #[must_use]
    pub fn recording(&self, id: &str) -> Option<&Recording> {
        self.recordings.iter().find(|r| r.id == id)
    }

    /// Every declared file, keyed by path, so a series can be checked against
    /// the bytes it claims to derive from.
    #[must_use]
    pub fn files(&self) -> BTreeMap<&Path, &FileEntry> {
        self.recordings
            .iter()
            .flat_map(|r| r.files.iter())
            .map(|f| (f.path.as_path(), f))
            .collect()
    }

    /// Read every file and check it against its declared hash.
    ///
    /// An empty result means the corpus on disk is the corpus that was
    /// measured. Anything else invalidates a report built from it.
    #[must_use]
    pub fn verify(&self, root: &Path) -> Vec<Defect> {
        let mut defects = Vec::new();
        let mut seen_ids: Vec<&str> = Vec::new();
        let mut hashed: BTreeMap<&Path, Result<String, String>> = BTreeMap::new();

        for r in &self.recordings {
            if seen_ids.contains(&r.id.as_str()) {
                defects.push(Defect::DuplicateId { id: r.id.clone() });
            }
            seen_ids.push(&r.id);
            if !self.sources.iter().any(|s| s.id == r.source) {
                defects.push(Defect::UnknownSource { id: r.id.clone(), source: r.source.clone() });
            }
            for file in &r.files {
                let actual = hashed
                    .entry(file.path.as_path())
                    .or_insert_with(|| match std::fs::read(root.join(&file.path)) {
                        Ok(b) => Ok(sha256_hex(&b)),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(String::new()),
                        Err(e) => Err(e.to_string()),
                    })
                    .clone();
                match actual {
                    Err(why) if why.is_empty() => {
                        defects.push(Defect::Missing { id: r.id.clone(), path: file.path.clone() });
                    }
                    Err(why) => {
                        defects.push(Defect::Unreadable { id: r.id.clone(), path: file.path.clone(), why });
                    }
                    Ok(actual) if actual != file.sha256 => defects.push(Defect::HashMismatch {
                        id: r.id.clone(),
                        path: file.path.clone(),
                        declared: file.sha256.clone(),
                        actual,
                    }),
                    Ok(_) => {
                        if file.upstream_sha256.as_ref().is_some_and(|u| *u != file.sha256) {
                            defects.push(Defect::UpstreamMismatch { id: r.id.clone(), path: file.path.clone() });
                        }
                    }
                }
            }
        }
        defects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording(id: &str, path: &str, sha: &str) -> Recording {
        Recording {
            id: id.into(),
            source: "src".into(),
            subject: "s1".into(),
            activity: "walk".into(),
            clock: Clock::Independent,
            reference: Reference {
                device: "ecg".into(),
                basis: ReferenceBasis::ManualAnnotation { annotator: "atr".into() },
            },
            files: vec![FileEntry { path: path.into(), sha256: sha.into(), upstream_sha256: None }],
            tags: BTreeMap::new(),
        }
    }

    fn corpus(recs: Vec<Recording>) -> Corpus {
        Corpus {
            sources: vec![Source {
                id: "src".into(),
                citation: "c".into(),
                url: "u".into(),
                license: "l".into(),
                retrieved: "2026-09-02".into(),
            }],
            recordings: recs,
        }
    }

    #[test]
    fn a_matching_file_has_no_defects() {
        let dir = std::env::temp_dir().join(format!("ww-corpus-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"hello").unwrap();
        let c = corpus(vec![recording("r1", "a.bin", &sha256_hex(b"hello"))]);
        assert!(c.verify(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_changed_byte_is_a_defect() {
        let dir = std::env::temp_dir().join(format!("ww-corpus-mut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"hellp").unwrap();
        let c = corpus(vec![recording("r1", "a.bin", &sha256_hex(b"hello"))]);
        let d = c.verify(&dir);
        assert!(matches!(d.as_slice(), [Defect::HashMismatch { .. }]), "{d:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_duplicate_id_and_unknown_source_are_each_named() {
        let dir = std::env::temp_dir().join(format!("ww-corpus-miss-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut other = recording("r1", "b.bin", "0");
        other.source = "nope".into();
        let c = corpus(vec![recording("r1", "a.bin", "0"), other]);
        let d = c.verify(&dir);
        assert!(d.iter().any(|x| matches!(x, Defect::Missing { .. })));
        assert!(d.iter().any(|x| matches!(x, Defect::DuplicateId { .. })));
        assert!(d.iter().any(|x| matches!(x, Defect::UnknownSource { .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upstream_disagreement_is_a_defect_even_when_the_index_matches() {
        let dir = std::env::temp_dir().join(format!("ww-corpus-up-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"hello").unwrap();
        let mut r = recording("r1", "a.bin", &sha256_hex(b"hello"));
        r.files[0].upstream_sha256 = Some("f".repeat(64));
        let d = corpus(vec![r]).verify(&dir);
        assert!(matches!(d.as_slice(), [Defect::UpstreamMismatch { .. }]), "{d:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reference_strength_tiers() {
        let manual = Reference { device: "e".into(), basis: ReferenceBasis::ManualAnnotation { annotator: "a".into() } };
        let algo = Reference { device: "e".into(), basis: ReferenceBasis::Algorithm { name: "pan-tompkins".into() } };
        let reported = Reference { device: "watch".into(), basis: ReferenceBasis::Reported };
        assert_eq!(manual.strength(), Strength::Proof);
        assert_eq!(algo.strength(), Strength::Documented);
        assert_eq!(reported.strength(), Strength::Asserted);
        assert!(Strength::Proof > Strength::Documented && Strength::Documented > Strength::Asserted);
    }
}
