//! ww-core: hold a wearable's accuracy against an attested reference, and say
//! `not_evaluated` whenever the evidence does not reach.
//!
//! Order of operations, by design. Each stage only trusts the one before it:
//! 1. `corpus`   — recordings with receipts. `Corpus::verify` re-hashes every
//!    raw file, so a report is always built from the bytes that were measured.
//! 2. `series`   — beat times or rates derived from those bytes, with the
//!    method written down and the inputs named by hash.
//! 3. `align`    — whether the device series even tracks the reference, and
//!    at what clock offset. Under an independent clock this is a gate.
//! 4. `window`   — fixed epochs where both sides have enough data.
//! 5. `stats`    — Bland-Altman, MAPE, tolerance fraction, Wilson intervals.
//! 6. `claim` / `report` — a stated threshold held against the interval.
//!
//! What this crate must never do: estimate a heart rate. It has no opinion
//! about what a heart rate is from a raw signal; adapters do that outside the
//! crate and record how. It measures a reported rate against a reference
//! whose provenance is already settled.
//!
//! Every ambiguity resolves in the device's favour. A threshold is only
//! `Refuted` when the whole interval clears it, an ambiguous alignment yields
//! no number at all, and a reference without a documented basis never enters
//! a verdict.

pub mod align;
pub mod claim;
pub mod corpus;
pub mod report;
pub mod series;
pub mod stats;
pub mod window;

pub use align::{Alignment, AlignmentPolicy};
pub use claim::{AppliesTo, Claim, Threshold};
pub use corpus::{Clock, Corpus, Defect, FileEntry, Recording, Reference, ReferenceBasis, Source, Strength};
pub use report::{Policy, Row, Verdict};
pub use series::{Role, Samples, Series};
pub use stats::{wilson, BlandAltman, Mape, Tolerance, Z_95, Z_99};
pub use window::{Coverage, HrWindow, HrvWindow, WindowPolicy};
