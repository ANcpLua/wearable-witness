# CLAUDE.md

Read this before touching anything. It says what the repository is, what state
it is in, which rules are not negotiable, and which paths are open. Every path
ends in a condition you can run. Do not start a path that is not listed here
without asking the owner first.

## What this is

`wearable-witness` holds a wearable's reported heart rate against a reference
recording and reports agreement with confidence intervals, or `not_evaluated`
when the evidence is insufficient. It never estimates a heart rate from a raw
signal; adapters in `tools/adapters/` do that and record how. `README.md` is
the overview, `docs/design.md` the module-by-module reasoning,
`docs/results.md` the generated tables.

Rust workspace: `crates/core` (library `ww-core`: corpus, series, align,
window, stats, claim, report) and `crates/cli` (binary `ww`). Python adapters:
`tools/adapters/physionet_wrist.py` (numpy, scipy, wfdb) and
`tools/adapters/step.py` (stdlib).

## Context: why this exists

Solo showcase for Winter School 2026 "Healthy Ageing", Track 6 (Richard Pasteka,
consumer-wearable validation, HRV). Application deadline 13 Sep 2026, on-site 23 to
27 Nov 2026, contact lisa.hartl@technikum-wien.at. Also intended for the agent hackathon
on 17 Sep 2026. The owner wants one human domain expert (Richard) plus Claude, not a
beginner group, so the instrument must be presentable. Personal motivation is heart
health; that belongs in the cover letter, not in the code.

STEP result (2026-09-03): Apple Watch meets the 10 % MAPE bound in every activity, Garmin
overall, Fitbit and Miband fail while walking and typing, Empatica E4 and Biovotion
Everion fail everywhere. Vendor ledger entries join but pin no number. STEP DUA was signed
by the owner on 2026-09-03 as PhysioNet user `ancplua`.

Shapes were carried over from counter-snake-oil, bytewitness and grounded-agents; those
reference clones are gone.

## State, and how to verify it

Run these first. If any of them disagrees with what is written here, stop and
tell the owner; do not "fix" the state.

```bash
git log --oneline            # exactly one commit: "wearable-witness 0.1.0", tag v0.1.0
git for-each-ref             # refs/heads/main, refs/remotes/origin/main, refs/tags/v0.1.0, nothing else
cargo test --workspace       # 42 tests pass
cargo clippy --workspace --all-targets   # zero warnings
tools/fetch.sh               # corpus 1 present and matching upstream hashes
tools/measure.sh --check     # "generated tables match a fresh run"
ls corpus-step/records/      # deidentified_data.csv, protocol.pdf, SHA256SUMS.txt (restricted; may be absent on other machines)
```

Remote: `github.com/ANcpLua/wearable-witness`, public, CI green on the single
commit.

## Rules that are not negotiable

1. **No backups.** No backup branches, no `.bak` files, no test clones in
   scratch directories, no "just in case" copies. If the owner asks for
   something to be removed, remove it and then search for stray copies
   (`git for-each-ref`, `git stash list`, `git worktree list`, scratch
   directories) before saying it is gone. A backup means nothing changed.
2. **No history changes without an explicit request in the current
   conversation**, and before pushing, show the resulting commit list as text
   and wait for approval. No `filter-branch`, no `rebase`, no `--amend`, no
   force-push on your own judgement.
3. **Commit messages are one line.** No body. No trailer unless the owner
   asks for one.
4. **Restricted data stays local.** `corpus-step/` and `series-step/` are
   gitignored and must stay so. Never print a value from
   `deidentified_data.csv`; the adapter prints counts, hashes and the header
   only. Never read the CSV into the conversation.
5. **Nothing in `ww-core` reads a raw signal or estimates a rate.**
6. **Generated numbers are never typed.** Every number inside
   `<!-- BEGIN/END GENERATED -->` blocks in `README.md` and `docs/results.md`
   comes from `tools/measure.sh --write`. A control that misbehaves is a bug in
   the instrument, never a finding.
7. **Passwords and accounts are the owner's.** PhysioNet credentials live in
   the owner's `~/.netrc`; `wget` reads it. Do not read that file, do not ask
   for the password.

## Data gotchas already paid for

- PhysioNet serves restricted files only to a `Wget/` user agent with HTTP
  Basic auth after a 401 challenge; `curl` gets 403 regardless of credentials.
  Use `wget`, which reads `~/.netrc`.
- Chrome cannot save the 12 MB CSV via "Save page" (truncates at 256 KB) and
  blocks script-initiated downloads. The only browser path is right-click,
  "Save link as".
- STEP rows carry no timestamps; the row index is the time axis on both sides.
  `python3 tools/adapters/step.py audit` shows 1.2 to 1.6 rows per nominal
  second. Epochs in that corpus are in rows.
- Corpus 1 (PhysioNet wrist) does not document how ECG and wrist unit were
  synchronised; every recording is `Clock::Independent`, so most rows are
  `not_evaluated`. That is correct behaviour, not a bug.

## Open paths

Each path: goal, where, steps, done-condition. Work one path per session
unless told otherwise. Keep the controls passing at every step.

### Path A: pool by skin tone (STEP)

Goal: the question the STEP study asked, answered by this instrument.
Where: `crates/core/src/report.rs` (`pool`), `corpus.rs` (`Recording.tags`,
key `skin_tone_fitzpatrick`), `crates/cli/src/main.rs` (tables),
`tools/measure.sh` (a new `step-skin-tone` block), `docs/results.md`.
Steps: add a pooling key over a tag; one pooled row per device × tag value and
per device × tag value × activity; render as its own table; add a block.
Done when: a unit test in `report.rs` pools two synthetic recordings with
different tag values into separate rows; `cargo test` 43+; `tools/measure.sh
--write` then `--check` green; `docs/results.md` shows the table with the
provenance line; no value from the CSV appears anywhere but the pooled table.

### Path B: block bootstrap for the intervals

Goal: intervals that respect autocorrelation between consecutive epochs.
Where: `crates/core/src/stats.rs` (`Mape::of`, `Tolerance::of`,
`BlandAltman::of`), `report::Policy` (block length, resample count, seed).
Steps: moving-block bootstrap over epoch sequences per recording; seeded
xorshift so results are deterministic; policy fields serialised with the
report; keep the closed-form interval as the default until the owner switches.
Done when: a test shows the bootstrap interval equals the closed form within
tolerance on independent synthetic epochs and is wider on autocorrelated
ones; controls unchanged; `README.md` limitation paragraph updated;
`tools/measure.sh --check` green.

### Path C: adapters for device exports

Goal: a lab plugs in its own recordings.
Where: new files under `tools/adapters/`; formats in order of value: Polar
CSV/RR export, Garmin FIT, Apple Health XML.
Steps: one adapter per format producing `series/*.json` with role, device,
method, inputs by hash, samples (`beats` where RR is available, else `rate`);
a `corpus` subcommand that writes recording entries with clock and reference
attestations the user fills in.
Done when: a sample export (owner-provided or synthetic, never restricted
data) passes `Series::check` through `ww verify`; the adapter prints counts,
hashes and the header only; documented in `README.md` under Data.

### Path D: HRV on a device that reports beats

Goal: RMSSD agreement for a real device, which STEP cannot give (rates only).
Where: needs a dataset or lab recording with device RR intervals plus a
reference; then Path C's adapter.
Done when: `docs/results.md` HRV table has at least one non-control device row
and the README states the source and its attestation.

### Path E: clock attestation for corpus 1

Goal: replace `Clock::Independent` with `Clock::Shared { basis }` if the
authors document synchronisation.
Where: Jarchi and Casson 2017, Data 2(1):1, doi:10.3390/data2010001
(fetch was blocked; the owner can get it via the university library);
`tools/adapters/physionet_wrist.py` (`clock` field), `corpus/index.json`.
Done when: the basis quotes the paper verbatim with page or section; the
counterfactual table becomes the primary one in `README.md` and
`docs/results.md`; `tools/measure.sh --check` green.

### Path F: calibrate alignment thresholds

Goal: thresholds justified by data, not only by the controls.
Where: `crates/core/src/align.rs`, `AlignmentPolicy`.
Steps: a test that sweeps synthetic offsets from 0 to 150 s at several noise
levels over the corpus 1 references and records recovery rate; choose
`min_corr`, `min_margin`, `exclusion_s` from that; document in
`docs/design.md`.
Done when: the sweep is a test (deterministic), the chosen values are in
`Policy` with a comment naming the sweep, controls still pass.

### Path G: activity recognition (separate module)

Goal: activity recognition usually accompanies wearable validation; the instrument has nothing for it.
Where: a new crate or adapter, not `ww-core`; corpus 1 has accelerometer and
gyroscope channels and activity labels per recording.
Done when: a confusion matrix over walk/run/bike_low/bike_high with a
leave-one-subject-out split is generated by a script and its numbers land in
a generated block; nothing of it touches the agreement pipeline.

### Path H: curated git history (only on explicit request)

Goal: a commit list that reads like the module walkthrough, one commit per
module with a factual subject.
Steps: from the single commit, build a branch with one commit per unit
(corpus, series, align, window, stats, claim, report, cli, adapters,
controls, docs); show the eleven subjects as text; wait for approval; only
then replace `main` and push; then search for stray refs and remove them.
Done when: `git log --oneline` shows the approved list and nothing else,
`git for-each-ref` shows only `main`, `origin/main` and the tag, CI green.

## Commands

```bash
tools/fetch.sh                                   # corpus 1: download, verify, ww verify
.venv/bin/python tools/adapters/physionet_wrist.py corpus | series
python3 tools/adapters/step.py audit | corpus | series      # restricted corpus, local only
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p ww-cli -- verify
cargo run -p ww-cli -- report [--json] [--strict] [--assume-shared-clock]
cargo run -p ww-cli -- --corpus corpus-step --series series-step report
cargo run -p ww-cli -- refute                    # exit 1 if a testable claim fails on a measured row
tools/measure.sh [--write|--check]
```

## Architecture in one line

```
corpus/index.json ─► Corpus::verify ─► Series::check ─► align::estimate ─► window::hr_windows ─► stats ─► Verdict
                       (hashes)          (inputs by hash)   (gate if clock     (coverage)       (intervals)  (claims)
                                                             is independent)
```

Every threshold that shapes a number is in `report::Policy` and is serialised
with the output. A verdict is `fails` only when the whole interval clears the
bound. Controls are device series whose name starts with `control:`.
