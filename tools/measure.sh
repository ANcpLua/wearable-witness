#!/usr/bin/env bash
# Regenerate (or check) the README tables from the release binary, nothing
# hand-typed.
#   tools/measure.sh            print the blocks to stdout
#   tools/measure.sh --write    replace the generated blocks in README.md
#   tools/measure.sh --check    exit 1 if a block differs from a fresh run
# Blocks are delimited by BEGIN/END GENERATED comments; prose outside them is
# never touched. --check is what answers "you typed those numbers".
set -euo pipefail
mode="print"
case "${1-}" in
  "") ;;
  --write) mode="write" ;;
  --check) mode="check" ;;
  *) echo "usage: $(basename "$0") [--write|--check]" >&2; exit 2 ;;
esac
root="$(cd "$(dirname "$0")/.." && pwd)"
( cd "$root" && cargo build --release -p ww-cli -q )
python3 - "$root" "$mode" <<'PY'
import difflib, json, os, subprocess, sys

root, mode = sys.argv[1], sys.argv[2]
BIN = os.path.join(root, "target", "release", "ww")
README = os.path.join(root, "README.md")
RESULTS = os.path.join(root, "docs", "results.md")
MARK = "tools/measure.sh"

def ww(*args):
    r = subprocess.run([BIN, *args], cwd=root, capture_output=True, text=True)
    if r.returncode not in (0, 1):
        sys.exit(f"ww {' '.join(args)} failed:\n{r.stderr}")
    return r.stdout

def section(md, title):
    body = md.split(f"## {title}\n\n", 1)[1]
    return body.split("\n## ", 1)[0].rstrip()

md = ww("report")
data = json.loads(ww("report", "--json"))
md_cf = ww("report", "--assume-shared-clock")
data_cf = json.loads(ww("report", "--json", "--assume-shared-clock"))
policy_line = md.split("\n", 1)[0]

def counts(data):
    rows = [r for r in data["rows"] if not r["device"].startswith("control:")]
    recs = len({r["recording"] for r in rows})
    ev = sum(1 for r in rows if r["verdict"]["kind"] == "evaluated")
    ne = len(rows) - ev
    reasons = {}
    for r in rows:
        if r["verdict"]["kind"] == "not_evaluated":
            why = r["verdict"]["why"]
            why = "too few epochs for a verdict of its own" if "evaluable epoch" in why else why.split(":")[0]
            reasons[why] = reasons.get(why, 0) + 1
    why = ", ".join(f"{v} × {k}" for k, v in sorted(reasons.items(), key=lambda kv: -kv[1]))
    tail = f" ({why})" if why else ""
    standings = {}
    for r in rows:
        if r["verdict"]["kind"] == "evaluated":
            for v in r["verdict"]["verdicts"]:
                standings[v["standing"]] = standings.get(v["standing"], 0) + 1
    st = ", ".join(f"{v} {k}" for k, v in sorted(standings.items()))
    st = f" Of the evaluated rows, `literature-mape-10` stands: {st}." if st else ""
    return f"{len(rows)} device rows over {recs} recordings: {ev} evaluated, {ne} not evaluated{tail}.{st}"

EXPECT = {
    "control: reference beats, identity": (0.0, "better"),
    "control: reference beats shifted +60 s": (-60.0, "better"),
    "control: reference rate ×1.15": (0.0, "refuted"),
}

def controls(data):
    rows = [r for r in data["rows"] if r["device"].startswith("control:")]
    bad = []
    for r in rows:
        lag, standing = EXPECT[r["device"]]
        ok = (r["verdict"]["kind"] == "evaluated"
              and r.get("lag_applied_s") == lag
              and r["alignment"]["kind"] == "conclusive"
              and any(v["claim"] == "literature-mape-10" and v["standing"] == standing
                      for v in r["verdict"]["verdicts"]))
        if not ok:
            bad.append(f'{r["recording"]} / {r["device"]}')
    if bad:
        sys.exit("controls misbehave, which is a bug in the instrument:\n  " + "\n  ".join(bad))
    head = f"{len(EXPECT)} controls × {len({r['recording'] for r in rows})} recordings = {len(rows)} rows; all {len(rows)} behave as specified (lag recovered, verdict as expected). Pooled over every recording:"
    pooled = [ln for ln in section(md, "Pooled").splitlines() if ln.startswith("| Device") or ln.startswith("|---") or ("| control:" in ln and "**all**" in ln)]
    return head + "\n\n" + "\n".join(pooled)

def measured_pooled(md_text):
    return "\n".join(ln for ln in section(md_text, "Pooled").splitlines() if "| control:" not in ln)

def headline(md_text):
    """Pooled rows over all activities only, one per device."""
    lines = section(md_text, "Pooled").splitlines()
    keep = [ln for ln in lines if ln.startswith("| Device") or ln.startswith("|---") or ("**all**" in ln and "| control:" not in ln)]
    return "\n".join(ln.replace("| **all** ", "| all ") for ln in keep)

# BigIdeasLab_STEP is restricted data under a signed DUA: raw bytes and derived
# series live only on a machine whose owner signed it. When they are present
# the blocks are regenerated; when absent (CI), the committed blocks are kept
# and reported as unchecked rather than failed.
STEP_CSV = os.path.join(root, "corpus-step", "records", "deidentified_data.csv")
STEP_PRESENT = os.path.exists(STEP_CSV) and os.path.isdir(os.path.join(root, "series-step")) \
    and any(f.endswith(".json") for f in os.listdir(os.path.join(root, "series-step")))

def step_blocks():
    args = ["--corpus", "corpus-step", "--series", "series-step"]
    md_s = ww(*args, "report")
    data_s = json.loads(ww(*args, "report", "--json"))
    import hashlib
    h = hashlib.sha256(open(STEP_CSV, "rb").read()).hexdigest()
    prov = (f"Computed locally over `deidentified_data.csv` (sha256 `{h[:16]}…`, PhysioNet upstream hash identical) "
            f"by `tools/adapters/step.py`; {ww(*args, 'verify').strip()}. CI cannot reproduce this block without the "
            f"signed agreement and leaves it unchecked.")
    audit = subprocess.run([sys.executable, os.path.join(root, "tools", "adapters", "step.py"), "audit"],
                           cwd=root, capture_output=True, text=True, check=True).stdout.rstrip()
    return {
        "step-provenance": prov,
        "step-audit": "```\n" + audit + "\n```",
        "step-counts": counts(data_s),
        "step-pooled": measured_pooled(md_s),
        "step-headline": headline(md_s),
    }

BLOCKS = {
    "corpus": ww("verify").strip(),
    "policy": policy_line,
    "controls": controls(data),
    "counts": counts(data),
    "pooled": measured_pooled(md),
    "counts-assumed": counts(data_cf),
    "pooled-assumed": measured_pooled(md_cf),
    "hrv-assumed": section(md_cf, "Heart-rate variability (descriptive)"),
    "ledger": section(md, "Claims ledger"),
}
if STEP_PRESENT:
    BLOCKS.update(step_blocks())
else:
    print("note: BigIdeasLab_STEP data absent; step-* blocks left as committed, not checked", file=sys.stderr)

def splice(text, name, body):
    """Replace the block if its markers are present; a file need not carry every block."""
    b, e = f"<!-- BEGIN GENERATED {MARK} {name} -->", f"<!-- END GENERATED {MARK} {name} -->"
    if b not in text:
        return text
    if e not in text:
        sys.exit(f"missing END marker for {name}")
    head, rest = text.split(b, 1)
    _, tail = rest.split(e, 1)
    return f"{head}{b}\n{body}\n{e}{tail}"

if mode == "print":
    for n, body in BLOCKS.items():
        print(f"--- {n} ---\n{body}\n")
    sys.exit(0)

stale = False
for path in (README, RESULTS):
    cur = open(path, encoding="utf-8").read()
    new = cur
    for n, body in BLOCKS.items():
        new = splice(new, n, body)
    rel = os.path.relpath(path, root)
    if mode == "write":
        if new != cur:
            open(path, "w", encoding="utf-8").write(new)
            print(f"updated {rel}")
        else:
            print(f"{rel}: no change")
    elif new != cur:
        stale = True
        sys.stdout.writelines(difflib.unified_diff(
            cur.splitlines(True), new.splitlines(True),
            fromfile=f"{rel} (committed)", tofile=f"{rel} (fresh run)"))
if mode == "check":
    if stale:
        sys.exit("\ngenerated tables are stale; run tools/measure.sh --write")
    print("generated tables match a fresh run")
PY
