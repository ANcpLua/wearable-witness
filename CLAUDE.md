# CLAUDE.md

Guidance for working in this repository. Read `README.md` for the overview;
this file is about the code.

## What this is

`wearable-witness` holds a wearable's heart-rate accuracy against an attested
reference recording and reports the gap with an interval. It never estimates a
heart rate from a raw signal; adapters outside the crate do that and record
how. `not_evaluated` is a real verdict and is preferred to a manufactured
number.

Rust workspace, two crates:

- `crates/core` — library `ww-core`: corpus, series, alignment, windows,
  stats, claims, report. No I/O beyond hashing corpus files.
- `crates/cli` — binary `ww`: loads JSON, prints tables, owns exit codes.

Python adapters in `tools/adapters/`: `physionet_wrist.py` (numpy, scipy,
wfdb; `uv venv .venv && uv pip install --python .venv/bin/python wfdb numpy
scipy`) and `step.py` (stdlib only).

Two corpora. `corpus/` + `series/` is open PhysioNet data and is what CI
runs. `corpus-step/` + `series-step/` is BigIdeasLab_STEP, restricted data
under a signed Data Use Agreement: `corpus-step/` and `series-step/` are
gitignored entirely and must stay so (the per-participant index is data too).
`tools/adapters/step.py corpus` and `series` regenerate them from the CSV.
Never print a value from that CSV; the adapter prints counts, hashes and the
header only.

## Commands

```bash
tools/fetch.sh                                   # download + verify PhysioNet bytes, then ww verify
.venv/bin/python tools/adapters/physionet_wrist.py corpus   # regenerate corpus/index.json
.venv/bin/python tools/adapters/physionet_wrist.py series   # regenerate series/*.json
cargo test --workspace
cargo clippy --workspace --all-targets           # pedantic at warn
cargo run -p ww-cli -- verify
cargo run -p ww-cli -- report [--json] [--strict]
cargo run -p ww-cli -- refute                    # exit 1 if any testable claim fails on a measured row
tools/measure.sh [--write|--check]               # README tables; --check is the reproducibility gate
python3 tools/adapters/step.py audit             # STEP row-rate sanity check (counts only)
python3 tools/adapters/step.py corpus            # corpus-step/index.json
python3 tools/adapters/step.py series            # series-step/*.json (never committed)
cargo run -p ww-cli -- --corpus corpus-step --series series-step report
```

`measure.sh` regenerates the `step-*` README blocks only when the STEP bytes
are present; in CI they are left as committed and reported as unchecked.

## Architecture: how a number gets made

```
corpus/index.json ─► Corpus::verify ─► Series::check ─► align::estimate ─► window::hr_windows ─► stats ─► Verdict
                       (hashes)          (inputs by hash)   (gate if clock     (coverage)       (intervals)  (claims)
                                                             is independent)
```

- **Clock is the one policy input per recording.** `Clock::Independent` makes
  a conclusive alignment a precondition of any number. `Clock::Shared` records
  who attests the synchronisation and reports the estimate as a check.
- **Reference strength gates verdicts.** `ReferenceBasis::Reported` is
  `Strength::Asserted` and never produces a verdict.
- **Controls are device series whose name starts with `control:`.** The CLI
  prints them in their own table and `refute` ignores them. They are
  known-answer tests of the instrument (identity, +60 s shift, ×1.15 rate).
- **Every threshold that shapes a number is in `report::Policy`** and is
  serialised with the JSON output.
- **Refuted only when the whole interval clears the bound.** See
  `stats::against_maximum` / `against_minimum`.

## Rules for this repo

- Nothing in `ww-core` may read a raw signal or estimate a rate.
- Numbers inside `<!-- BEGIN/END GENERATED -->` blocks in `README.md` and `docs/results.md` come
  from `tools/measure.sh --write`. Never type one.
- `corpus/records/` is fetched, verified against upstream hashes, and never
  committed.
- A control that fails is a bug in the instrument, not a finding.
- Pooling includes epochs from rows too short for a verdict of their own;
  rows that failed alignment contribute nothing (see `report::pool`).
