# wearable-witness

[![ci](https://github.com/ANcpLua/wearable-witness/actions/workflows/ci.yml/badge.svg)](https://github.com/ANcpLua/wearable-witness/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Holds a wearable's reported heart rate against a reference recording and reports agreement with confidence intervals, or `not_evaluated` when the evidence is insufficient.**

If you use this software, please cite it (see [Citation](#citation)). The consumer-device results in [docs/results.md](docs/results.md) are computed on the BigIdeasLab_STEP dataset (Bent and Dunn 2021) and should be cited together with the original study (Bent et al. 2020).

## Statement of need

Validation studies of wearable heart-rate sensors typically report a mean absolute percentage error and a Bland-Altman plot per device and activity. Three things are usually left implicit: which bytes the numbers came from, which analysis choices shaped them, and what happens when the data cannot support a number at all. `wearable-witness` makes each of these explicit. Every input file is hashed, every derived series names its inputs and method, every threshold is a policy value that is printed with the results, and a verdict against a claim is issued only when the whole confidence interval lies on one side of the bound. Three known-answer controls run with every report.

The tool is aimed at researchers validating consumer wearables against laboratory-grade references, and at students who need a reproducible analysis rather than a notebook. It does not estimate heart rate from raw signals; libraries such as NeuroKit2 or HeartPy do that, and their output can be fed in as a device series.

## Quick example

```bash
tools/fetch.sh                        # 52 MB of PhysioNet data, verified against upstream hashes
cargo run --release -p ww-cli -- report
```

The report opens with the controls, which must pass before any measured row is worth reading:

<!-- BEGIN GENERATED tools/measure.sh controls -->
3 controls × 19 recordings = 57 rows; all 57 behave as specified (lag recovered, verdict as expected). Pooled over every recording:

| Device | Activity | Recordings | Epochs | Bias ± SD (bpm) | LoA (bpm) | MAPE [95%] | Within ±5 bpm / ±10% [95%] | Verdict |
|---|---|---:|---:|---:|---|---|---|---|
| control: reference beats shifted +60 s | **all** | 19 | 667 | +0.0 ± 0.1 | -0.1 to +0.1 | 0.0% [0.0–0.0%] | 667/667 = 100.0% [99.4–100.0%] | literature-mape-10: meets |
| control: reference beats, identity | **all** | 19 | 667 | +0.0 ± 0.0 | +0.0 to +0.0 | 0.0% [0.0–0.0%] | 667/667 = 100.0% [99.4–100.0%] | literature-mape-10: meets |
| control: reference rate ×1.15 | **all** | 19 | 665 | +15.7 ± 4.3 | +7.2 to +24.2 | 15.6% [15.5–15.7%] | 0/665 = 0.0% [0.0–0.6%] | literature-mape-10: **FAILS** |
<!-- END GENERATED tools/measure.sh controls -->

Consumer devices against an ECG patch, pooled over 53 participants (restricted data, computed locally; full tables and provenance in [docs/results.md](docs/results.md)):

<!-- BEGIN GENERATED tools/measure.sh step-headline -->
| Device | Activity | Recordings | Epochs | Bias ± SD (bpm) | LoA (bpm) | MAPE [95%] | Within ±5 bpm / ±10% [95%] | Verdict |
|---|---|---:|---:|---:|---|---|---|---|
| Apple Watch | all | 204 | 3137 | -0.3 ± 6.9 | -13.9 to +13.3 | 4.5% [4.3–4.7%] | 2832/3137 = 90.3% [89.2–91.3%] | literature-mape-10: meets |
| Biovotion Everion | all | 452 | 10378 | -7.0 ± 20.7 | -47.5 to +33.6 | 20.1% [19.9–20.3%] | 2111/10378 = 20.3% [19.6–21.1%] | literature-mape-10: **FAILS** |
| Empatica E4 | all | 411 | 9575 | -2.3 ± 18.1 | -37.8 to +33.3 | 13.6% [13.3–14.0%] | 5223/9575 = 54.5% [53.5–55.5%] | literature-mape-10: **FAILS** |
| Fitbit | all | 252 | 2511 | -5.2 ± 14.5 | -33.6 to +23.3 | 11.5% [11.0–11.9%] | 1460/2511 = 58.1% [56.2–60.1%] | literature-mape-10: **FAILS** |
| Garmin | all | 207 | 4415 | -3.7 ± 13.3 | -29.7 to +22.3 | 8.8% [8.5–9.1%] | 3125/4415 = 70.8% [69.4–72.1%] | literature-mape-10: meets |
| Xiaomi Miband | all | 191 | 3311 | -4.1 ± 17.2 | -37.7 to +29.5 | 14.3% [13.8–14.7%] | 1615/3311 = 48.8% [47.1–50.5%] | literature-mape-10: **FAILS** |
<!-- END GENERATED tools/measure.sh step-headline -->

`meets` and `fails` refer to the validation literature's 10% MAPE bound (Nelson and Allen 2019); `not settled` means the interval straddles it.

## Installation

Rust 1.85 or newer via [rustup](https://rustup.rs); Python 3.12 or newer for the adapters.

```bash
git clone https://github.com/ANcpLua/wearable-witness && cd wearable-witness
cargo build --release                                            # binary at target/release/ww
uv venv .venv && uv pip install --python .venv/bin/python wfdb numpy scipy   # adapters only
```

Dependencies are pinned in `Cargo.lock`. The core has no runtime dependencies beyond the standard library, `serde`, `sha2` and `clap`.

## How it works

```
recording (raw files, sha256 local and upstream)
  -> series (beats or rate; method and input hashes recorded)
  -> alignment (lag search; gates the result when the clocks are independent)
  -> epochs (fixed length; coverage counted)
  -> agreement (Bland-Altman, MAPE, tolerance fraction; Wilson intervals)
  -> verdict (claim threshold held against the interval)
```

Facts (hashes, beat times, reported rates) are kept apart from policy (epoch length, minimum epochs, alignment thresholds, tolerance rule, reference tiers, clock attestations). All policy lives in one struct that is serialised with every report. The rules that decide a verdict, and the reasoning behind each module, are in [docs/design.md](docs/design.md).

| Module | Responsibility |
|---|---|
| `corpus` | Recordings, file hashes, clock and reference attestations; `verify` re-hashes every file. |
| `series` | Beat or rate series with method and input hashes; refused if inputs are not corpus files. |
| `align` | Lag search by cross-correlation; conclusive or ambiguous with a reason. |
| `window` | Fixed epochs with coverage counts; RMSSD where both sides carry beats. |
| `stats` | Bland-Altman, MAPE, tolerance fraction, Wilson intervals, standing against a bound. |
| `claim` | Verbatim quotes with page hashes; testable only with a numeric bound and derivation. |
| `report` | Rows and pooled rows; every number derived, none typed. |
| `ww` (CLI) | `verify`, `report [--json] [--strict] [--assume-shared-clock]`, `refute` (exit 1 on a failed claim). |

## Data

| Corpus | Source | Access | Role |
|---|---|---|---|
| Wrist PPG during exercise | Jarchi and Casson 2017, [PhysioNet wrist/1.0.0](https://physionet.org/content/wrist/1.0.0/) | open (ODC-By 1.0) | fetched and verified by `tools/fetch.sh`; runs in CI |
| BigIdeasLab_STEP | Bent and Dunn 2021, [PhysioNet](https://physionet.org/content/bigideaslab-step-hr-smartwatch/1.0/) | restricted (Data Use Agreement) | regenerated locally by `tools/adapters/step.py`; never committed |

Raw bytes, derived series and the per-participant index of the restricted corpus are not part of this repository. Results computed from it carry the input hash and are marked as not reproducible in CI.

## Tests and reproducibility

```bash
cargo test --workspace          # 42 unit tests, including hand-computed statistics and control behaviour
cargo clippy --workspace --all-targets
tools/measure.sh --check        # every generated table in README.md and docs/results.md matches a fresh run
```

CI runs all three on every push. A control that misbehaves fails the check.

## Limitations

- Consecutive epochs of one recording are treated as independent, so every interval is narrower than it should be. A block bootstrap or a per-recording random effect would correct this.
- Alignment thresholds are validated on the controls, not calibrated against a dataset with a known clock offset.
- The synchronisation of the two recorders in the open corpus is undocumented; its recordings are therefore marked as independent clocks and most rows are `not_evaluated`. A counterfactual report under a shared clock is provided and labelled.
- The restricted corpus has no timestamps; epochs there are counted in rows.
- Skin tone is carried as a tag but is not yet a pooling key.

## Contributing

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). Changes to any threshold must keep the controls passing and regenerate the tables.

## AI usage disclosure

The Rust and Python code, the tests and the documentation were written by Claude (Anthropic, model Claude Fable 5.1) under the direction of the author, who set the architecture and the fail-closed rules, selected the datasets and signed the data agreement, defined what may leave the machine, reviewed every result and decided what is claimed. Commits carry a `Co-Authored-By` trailer for the model.

## Citation

See [CITATION.cff](CITATION.cff).

- Software: Nachtmann A. *wearable-witness* (version 0.1.0), 2026. https://github.com/ANcpLua/wearable-witness
- Open corpus: Jarchi D, Casson AJ. Description of a Database Containing Wrist PPG Signals Recorded during Physical Exercise with Both Accelerometer and Gyroscope Measures of Motion. *Data* 2017;2(1):1. doi:10.3390/data2010001
- Restricted corpus: Bent B, Dunn J. BigIdeasLab_STEP: Heart rate measurements captured by smartwatches for differing skin tones (version 1.0). *PhysioNet* 2021. doi:10.13026/cqfy-d860. Study: Bent B, Goldstein BA, Kibbe WA, Dunn JP. Investigating sources of inaccuracy in wearable optical heart rate sensors. *npj Digital Medicine* 2020;3:18. doi:10.1038/s41746-020-0226-6
- Threshold: Nelson BW, Allen NB. Accuracy of Consumer Wearable Heart Rate Measurement During an Ecologically Valid 24-Hour Period. *JMIR mHealth and uHealth* 2019;7(3):e10828.

## License

MIT. Copyright (c) 2026 Alexander Nachtmann.
