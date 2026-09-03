#!/usr/bin/env bash
# Fetch the raw PhysioNet bytes the corpus names, verify them against the
# upstream SHA256SUMS, then against corpus/index.json via `ww verify`.
# Idempotent: files already present and matching are not downloaded again.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
base="https://physionet.org/files/wrist/1.0.0"
# PhysioNet is slow to answer from some networks; retry connection failures
# and server errors, never a checksum mismatch (that is verified below).
fetch() { curl -fsS --connect-timeout 30 --retry 6 --retry-delay 10 --retry-all-errors -o "$1" "$2"; }
dir="$root/corpus/records"
mkdir -p "$dir"
cd "$dir"
# sha256sum on Linux, shasum on macOS; both read the "hash  name" format.
if command -v sha256sum >/dev/null 2>&1; then
  sum() { sha256sum "$@"; }
else
  sum() { shasum -a 256 "$@"; }
fi
for f in RECORDS SHA256SUMS.txt; do
  [ -s "$f" ] || fetch "$f" "$base/$f"
done
need=()
for rec in $(cat RECORDS); do
  for ext in hea dat atr; do
    f="$rec.$ext"
    want="$(awk -v f="$f" '$2==f{print $1}' SHA256SUMS.txt)"
    if [ ! -s "$f" ] || [ "$(sum "$f" | cut -d' ' -f1)" != "$want" ]; then
      need+=("$f")
    fi
  done
done
if [ "${#need[@]}" -gt 0 ]; then
  echo "fetching ${#need[@]} file(s) from $base"
  for f in "${need[@]}"; do
    fetch "$f" "$base/$f"
  done
fi
awk '{print $1"  "$2}' SHA256SUMS.txt | grep -E '\.(hea|dat|atr)$' | sum -c --quiet -
echo "records match upstream SHA256SUMS"
cd "$root"
cargo run -q -p ww-cli -- verify
