# Contributing

Issues and pull requests are welcome.

- Run `cargo test --workspace`, `cargo clippy --workspace --all-targets` and `tools/measure.sh --check` before opening a pull request. CI runs the same three.
- A change to any policy value must keep the three controls passing and must regenerate the tables with `tools/measure.sh --write`. Never edit a number inside a generated block by hand.
- Nothing in `crates/core` may read a raw signal or estimate a heart rate. Signal processing belongs in `tools/adapters/`.
- Data from the restricted corpus (raw bytes, derived series, per-participant index) must never be committed. The `.gitignore` entries for `corpus-step/` and `series-step/` stay.
- Commit messages: one subject line.

To add a corpus, write an adapter that produces `corpus/index.json` entries (files with hashes, clock attestation, reference basis) and `series/*.json` files (role, device, method, inputs by hash, samples). The struct doc comments in `crates/core/src/corpus.rs` and `series.rs` are the schema.
