#!/usr/bin/env python3
"""Adapter for PhysioNet BigIdeasLab_STEP (bigideaslab-step-hr-smartwatch/1.0).

Restricted data under a signed Data Use Agreement. This adapter reads the
CSV from corpus-step/records/, which is never committed, and writes derived
series to series-step/, which is never committed either: a per-second heart
rate of a de-identified participant is the dataset in another form. Only
corpus-step/index.json (hashes and attestations, no values) is tracked.

  corpus   write corpus-step/index.json
  series   write series-step/<recording>.<slug>.json

The CSV has ten columns (README.md of the dataset): ECG, Apple Watch,
Empatica, Garmin, Fitbit, Miband, Biovotion, ID, Skin Tone, Activity.
Timestamps were removed at de-identification; rows are time-synced. The row
index is therefore the only time axis there is, and it is what both sides
share, so pairing is exact whatever a row's duration. `audit` compares block
lengths in rows with the protocol's nominal durations: rows outnumber
seconds by roughly 1.2 to 1.6, so a "10 s" epoch here is 10 rows, about 6 to
8 s of wall time. That is recorded on every recording's clock attestation.

A recording is one contiguous run of rows for one participant with one
activity label. The reference is the ECG column; each device column with at
least two finite values becomes a device series named exactly as the claims
ledger names the device, so the ledger joins.

Nothing here prints a data value. Only counts, hashes and the column header.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import sys

ADAPTER = "tools/adapters/step.py 0.1.0"
SOURCE_ID = "physionet-bigideaslab-step-1.0"
CITATION = (
    "Bent B, Dunn J. BigIdeasLab_STEP: Heart rate measurements captured by smartwatches for "
    "differing skin tones (version 1.0). PhysioNet 2021. doi:10.13026/cqfy-d860. Original study: "
    "Bent B, Goldstein BA, Kibbe WA, Dunn JP. Investigating sources of inaccuracy in wearable "
    "optical heart rate sensors. npj Digit Med 2020;3:18. doi:10.1038/s41746-020-0226-6."
)
URL = "https://physionet.org/content/bigideaslab-step-hr-smartwatch/1.0/"
LICENSE = "PhysioNet Restricted Health Data License 1.5.0 (signed DUA; not redistributable)"
COLUMNS = ["ECG", "Apple Watch", "Empatica", "Garmin", "Fitbit", "Miband", "Biovotion", "ID", "Skin Tone", "Activity"]
REFERENCE_DEVICE = "Bittium Faros 180 ECG patch, heart rate as reported by the device"
DEVICES = {
    "Apple Watch": ("Apple Watch", "Apple Watch 4, software 5.1.3, HR as exported (variable sampling)"),
    "Fitbit": ("Fitbit", "Fitbit Charge 2, software 22.55.2, HR as exported (variable sampling)"),
    "Garmin": ("Garmin", "Garmin Vivosmart 3, software 5.10, HR as exported (variable sampling)"),
    "Miband": ("Xiaomi Miband", "Xiaomi Miband 3, HR as exported (variable sampling)"),
    "Empatica": ("Empatica E4", "Empatica E4, research-grade wrist PPG, HR as exported (~1 Hz)"),
    "Biovotion": ("Biovotion Everion", "Biovotion Everion, research-grade upper-arm PPG, HR as exported (~1 Hz)"),
}
ACTIVITY = {"Rest": "rest", "Activity": "walk", "Breathe": "breathe", "Type": "type"}
DT_S = 1.0


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def upstream_sums(root: str) -> dict[str, str]:
    out = {}
    p = os.path.join(root, "records", "SHA256SUMS.txt")
    if os.path.exists(p):
        for line in open(p, encoding="utf-8"):
            parts = line.split()
            if len(parts) == 2:
                out[parts[1]] = parts[0]
    return out


def fnum(s: str) -> float:
    try:
        v = float(s)
    except ValueError:
        return math.nan
    return v if math.isfinite(v) and v > 0 else math.nan


def segments(csv_path: str):
    """Yield (participant, skin_tone, activity, rows) for each contiguous run.

    rows is a list of dicts column -> float (NaN where missing). Activity-less
    rows (label NaN) are skipped; they are the gaps between protocol blocks.
    """
    with open(csv_path, newline="", encoding="utf-8") as f:
        r = csv.reader(f)
        header = next(r)
        if header != COLUMNS:
            sys.exit(f"unexpected header: {len(header)} columns, expected {COLUMNS}")
        cur = None
        rows = []
        for rec in r:
            if len(rec) != len(COLUMNS):
                continue
            pid, tone, act = rec[7].strip(), rec[8].strip(), rec[9].strip()
            key = (pid, tone, act) if act in ACTIVITY else None
            if key != cur:
                if cur is not None and rows:
                    yield (*cur, rows)
                cur, rows = key, []
            if key is not None:
                rows.append({c: fnum(rec[i]) for i, c in enumerate(COLUMNS[:7])})
        if cur is not None and rows:
            yield (*cur, rows)


def recording_id(pid: str, seq: int, act: str) -> str:
    return f"step-{pid}-{seq:02d}-{ACTIVITY[act]}"


def cmd_corpus(root: str) -> None:
    csv_path = os.path.join(root, "records", "deidentified_data.csv")
    files = []
    sums = upstream_sums(root)
    for name in ("deidentified_data.csv", "protocol.pdf"):
        p = os.path.join(root, "records", name)
        if os.path.exists(p):
            e = {"path": f"records/{name}", "sha256": sha256(p)}
            if name in sums:
                e["upstream_sha256"] = sums[name]
            files.append(e)
    recs = []
    seq_by_pid: dict[str, int] = {}
    for pid, tone, act, rows in segments(csv_path):
        seq_by_pid[pid] = seq_by_pid.get(pid, 0) + 1
        recs.append({
            "id": recording_id(pid, seq_by_pid[pid], act),
            "source": SOURCE_ID,
            "subject": pid,
            "activity": ACTIVITY[act],
            "clock": {"kind": "shared", "basis": "Dataset description: 'The data is completely de-identified and observations are time-synced', each row one point in time. Timestamps were removed at de-identification, so the row index is the time axis for both sides. Against the protocol's nominal block durations one row spans roughly 0.6 to 0.8 s (tools/adapters/step.py audit), so epoch lengths in this corpus are in rows, not seconds."},
            "reference": {"device": REFERENCE_DEVICE, "basis": {"kind": "algorithm", "name": "Bittium Faros 180 firmware heart rate from a ~1000 Hz single-lead ECG, as published in the dataset"}},
            "files": files,
            "tags": {"skin_tone_fitzpatrick": tone, "rows": str(len(rows))},
        })
    index = {
        "sources": [{"id": SOURCE_ID, "citation": CITATION, "url": URL, "license": LICENSE, "retrieved": "2026-09-03"}],
        "recordings": recs,
    }
    with open(os.path.join(root, "index.json"), "w", encoding="utf-8") as f:
        json.dump(index, f, indent=1)
        f.write("\n")
    print(f"corpus-step/index.json: {len(recs)} recordings, {len(seq_by_pid)} participants")


def write_series(out_dir: str, rec: str, slug: str, role: str, device: str, method: str, inputs: list, samples: dict) -> None:
    doc = {"recording": rec, "role": role, "device": device, "method": method, "adapter": ADAPTER, "inputs": inputs, "samples": samples}
    with open(os.path.join(out_dir, f"{rec}.{slug}.json"), "w", encoding="utf-8") as f:
        json.dump(doc, f, separators=(",", ":"))
        f.write("\n")


def cmd_series(root: str, out_dir: str) -> None:
    csv_path = os.path.join(root, "records", "deidentified_data.csv")
    os.makedirs(out_dir, exist_ok=True)
    inputs = [{"path": "records/deidentified_data.csv", "sha256": sha256(csv_path)}]
    seq_by_pid: dict[str, int] = {}
    n_series = 0
    per_device = {k: 0 for k in DEVICES}
    for pid, tone, act, rows in segments(csv_path):
        seq_by_pid[pid] = seq_by_pid.get(pid, 0) + 1
        rec = recording_id(pid, seq_by_pid[pid], act)
        t = [i * DT_S for i in range(len(rows))]
        ref = [(ti, r["ECG"]) for ti, r in zip(t, rows) if not math.isnan(r["ECG"])]
        if len(ref) < 2:
            continue
        write_series(out_dir, rec, "reference", "reference", REFERENCE_DEVICE,
                     "ECG column as published: heart rate reported by the Faros patch; t = row index within the block, NaN rows skipped.",
                     inputs, {"kind": "rate", "t_s": [x[0] for x in ref], "bpm": [round(x[1], 3) for x in ref]})
        n_series += 1
        for col, (name, method) in DEVICES.items():
            dev = [(ti, r[col]) for ti, r in zip(t, rows) if not math.isnan(r[col])]
            if len(dev) < 2:
                continue
            write_series(out_dir, rec, col.lower().replace(" ", "-"), "device", name,
                         f"{method}; column '{col}' as published; t = row index within the block, NaN rows skipped.",
                         inputs, {"kind": "rate", "t_s": [x[0] for x in dev], "bpm": [round(x[1], 3) for x in dev]})
            n_series += 1
            per_device[col] += 1
    print(f"series-step/: {n_series} series; device series per column: {per_device}")


def cmd_audit(root: str) -> None:
    """Row-rate sanity check against the protocol, printing counts only."""
    csv_path = os.path.join(root, "records", "deidentified_data.csv")
    by_act: dict[str, list[int]] = {}
    pids = set()
    n_rows = 0
    for pid, tone, act, rows in segments(csv_path):
        pids.add(pid)
        n_rows += len(rows)
        by_act.setdefault(act, []).append(len(rows))
    print(f"participants: {len(pids)}, labelled rows: {n_rows}")
    for act, lens in sorted(by_act.items()):
        lens.sort()
        med = lens[len(lens) // 2]
        print(f"  {act:9s} segments={len(lens):4d} median_rows={med:5d} min={lens[0]:5d} max={lens[-1]:5d}")
    print("protocol (nominal): Rest 240 s, Breathe 60 s, Activity 300 s, Type 60 s; three rounds per participant")
    nominal = {"Rest": 240, "Breathe": 60, "Activity": 300, "Type": 60}
    ratios = {a: sorted(l)[len(l) // 2] / nominal[a] for a, l in by_act.items() if a in nominal}
    print("median rows per nominal second: " + ", ".join(f"{a} {r:.2f}" for a, r in sorted(ratios.items())) + " (1.00 would mean one row is one second)")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["corpus", "series", "audit"])
    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ap.add_argument("--root", default=os.path.join(repo, "corpus-step"))
    ap.add_argument("--out", default=os.path.join(repo, "series-step"))
    a = ap.parse_args()
    if a.cmd == "corpus":
        cmd_corpus(a.root)
    elif a.cmd == "series":
        cmd_series(a.root, a.out)
    else:
        cmd_audit(a.root)


if __name__ == "__main__":
    sys.exit(main())
