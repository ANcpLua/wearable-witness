# Design

The instrument separates three kinds of statement and never lets one masquerade as another.

- **Fact**: what was in the bytes. File hashes, beat times, reported rates, the correlation at a given lag.
- **Policy**: what was chosen. Epoch length, minimum epochs, alignment thresholds, tolerance rule, reference tiers, clock attestations. Every policy value lives in `report::Policy` and is serialised with the output.
- **Verdict**: what follows from fact and policy. Never from a point estimate alone, never when the evidence is missing.

## Rules

1. A bound is `fails` only when the whole confidence interval lies on the wrong side of it, `meets` only when the whole interval lies on the right side; otherwise `not settled` (`stats::against_maximum`, `stats::against_minimum`).
2. Under `Clock::Independent`, no agreement number is produced unless one lag clearly explains both series: correlation at least `min_corr`, margin at least `min_margin` over every rival lag beyond `exclusion_s`, and the best lag not on the boundary of the search (`align::estimate`).
3. A reference whose basis is `Reported` has strength `Asserted` and never produces a verdict. `ManualAnnotation` is `Proof`, `Algorithm` on a clinical signal is `Documented`; both produce verdicts (`corpus::Reference::strength`).
4. A row with fewer evaluable epochs than `min_windows` receives no verdict of its own; its epochs still enter pooled rows. A row that failed alignment has no epochs (`report::pool`).
5. A claim is testable only if it states a numeric bound and records how the bound was read from the quote (`claim::Claim::is_testable`). A device claim joins by exact device name.
6. A series is refused if any declared input is not a corpus file with the declared hash (`series::Series::check`).
7. Nothing in `ww-core` reads a raw signal or estimates a heart rate.

## Modules

### `corpus`
Recordings with their raw files, each file with a local sha256 and, where the upstream publishes one, the upstream sha256. `verify` re-hashes every file and returns a list of defects; a report is built only over a corpus with none. The clock attestation (`Shared { basis }` or `Independent`) and the reference basis are recorded per recording.

### `series`
`Samples::Beats` (beat times) or `Samples::Rate` (times and beats per minute), plus the method in prose, the adapter version and the input files by hash. `as_rate` turns beats into an instantaneous rate (60 / RR at the second beat of each pair); `shifted` moves a series onto another clock without touching the original.

### `align`
Both series are interpolated onto a grid and Pearson-correlated at every lag in `[-max_lag_s, +max_lag_s]` in steps of `grid_s`. The search is exhaustive and deterministic. The result is `Conclusive { lag_s, corr, runner_up }` or `Ambiguous { .., why }` with the failing criterion named. `lag_s` is what to add to device times to land on the reference clock.

### `window`
The overlap of the two series is cut into epochs. A rate is stated for an epoch only from at least `min_beats` beats (60 × intervals / span) or `min_rate_samples` reported values (mean). RMSSD is computed only from beats and only with at least `min_rr` intervals. `Coverage` counts epochs, evaluable epochs and which side dropped the rest.

### `stats`
Bland-Altman (bias, SD, limits of agreement at ±z·SD), MAPE with a normal-approximation interval clamped at zero, and the fraction of epochs inside a tolerance rule (`|d| ≤ max(bpm, percent·reference)`) with a Wilson score interval. Wilson is used for proportions because the normal approximation produces intervals that leave [0, 1] near the edges, which would let a clean run claim perfection.

### `claim`
A verbatim quote with URL, retrieval date and the sha256 of the page as read, so a later edit is provable. `Threshold::MapeAtMost` or `Threshold::WithinToleranceAtLeast`; `AppliesTo::Any` for a literature or standards threshold, `AppliesTo::Device { name }` for a vendor's words about one device.

### `report`
`build` produces one row per device series: alignment, lag applied, coverage, and either `Evaluated { hr, hrv, verdicts }` or `NotEvaluated { why }`. `pool` concatenates epochs per device and activity, then per device overall. Controls are device series whose name starts with `control:`; they are printed separately and ignored by `refute`.

## Known statistical limitation

Consecutive epochs of one recording are autocorrelated; every interval here treats them as independent and is therefore narrower than it should be. That makes `fails` easier than it should be, which is the wrong direction for a tool whose ambiguities are meant to favour the device. The tests pin the present arithmetic so that changing the method is a visible decision. A block bootstrap or a per-recording random effect is the intended correction.
