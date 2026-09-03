#!/usr/bin/env python3
"""Adapter for PhysioNet "Wrist PPG During Exercise" (wrist/1.0.0).

Two subcommands, both deterministic:

  corpus   write corpus/index.json from corpus/records/{RECORDS,SHA256SUMS.txt,*.hea}
  series   write series/<record>.<slug>.json for every record:
             reference        beats from the .atr annotations (human-marked R peaks)
             ppg-peaks        beats from a band-pass + peak-picking detector on wrist PPG
             ppg-spectral     rate from the dominant PPG spectral line per 8 s frame,
                              with accelerometer lines suppressed
             ppg-tracked      rate from a Viterbi path through the same spectrogram
             control-*        known-answer series derived from the reference

The three PPG estimators are baselines written for this repository, not a
vendor's firmware. They stand in for "the device's reported rate" because the
dataset ships raw PPG, not a watch's output. Their quality is what the
instrument reports; it is not the point of the instrument.

ww-core never reads a raw signal. Everything physiological happens here, and
this file records how.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys

import numpy as np

ADAPTER = "tools/adapters/physionet_wrist.py 0.1.0"
SOURCE_ID = "physionet-wrist-1.0.0"
CITATION = (
    "Jarchi D, Casson AJ. Description of a Database Containing Wrist PPG Signals Recorded "
    "during Physical Exercise with Both Accelerometer and Gyroscope Measures of Motion. "
    "Data 2017;2(1):1. doi:10.3390/data2010001. Hosted on PhysioNet (wrist/1.0.0)."
)
URL = "https://physionet.org/content/wrist/1.0.0/"
LICENSE = "Open Data Commons Attribution License v1.0"
ACTIVITY = {"walk": "walk", "run": "run", "low_resistance_bike": "bike_low", "high_resistance_bike": "bike_high"}
REFERENCE_DEVICE = "Actiwave chest ECG, R peaks marked by hand (.atr)"
PPG_DEVICE = "Shimmer3 GSR+ wrist PPG"


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def records(root: str) -> list[str]:
    with open(os.path.join(root, "records", "RECORDS"), encoding="utf-8") as f:
        return [line.strip() for line in f if line.strip()]


def upstream_sums(root: str) -> dict[str, str]:
    out = {}
    with open(os.path.join(root, "records", "SHA256SUMS.txt"), encoding="utf-8") as f:
        for line in f:
            parts = line.split()
            if len(parts) == 2:
                out[parts[1]] = parts[0]
    return out


def activity_of(rec: str) -> str:
    key = rec.split("_", 1)[1]
    return ACTIVITY[key]


def cmd_corpus(root: str) -> None:
    sums = upstream_sums(root)
    recs = []
    for rec in records(root):
        files = []
        for ext in ("hea", "dat", "atr"):
            name = f"{rec}.{ext}"
            files.append({
                "path": f"records/{name}",
                "sha256": sha256(os.path.join(root, "records", name)),
                "upstream_sha256": sums[name],
            })
        recs.append({
            "id": rec,
            "source": SOURCE_ID,
            "subject": rec.split("_", 1)[0],
            "activity": activity_of(rec),
            "clock": {"kind": "independent"},
            "reference": {
                "device": REFERENCE_DEVICE,
                "basis": {"kind": "manual_annotation", "annotator": "atr: 'Reference ECG peaks' (PhysioNet ANNOTATORS), identified by hand per the dataset description"},
            },
            "files": files,
        })
    index = {
        "sources": [{
            "id": SOURCE_ID,
            "citation": CITATION,
            "url": URL,
            "license": LICENSE,
            "retrieved": "2026-09-02",
        }],
        "recordings": recs,
    }
    with open(os.path.join(root, "index.json"), "w", encoding="utf-8") as f:
        json.dump(index, f, indent=2)
        f.write("\n")
    print(f"corpus/index.json: {len(recs)} recordings")


# ---------------------------------------------------------------- estimators

def finite_runs(mask: np.ndarray):
    edges = np.flatnonzero(np.diff(np.r_[0, mask.astype(int), 0]))
    return list(zip(edges[::2], edges[1::2]))


def ppg_peaks(ppg: np.ndarray, fs: float) -> np.ndarray:
    from scipy.signal import butter, filtfilt, find_peaks
    b, a = butter(2, [0.5 / (fs / 2), 4.0 / (fs / 2)], btype="band")
    out = []
    for s, e in finite_runs(np.isfinite(ppg)):
        seg = ppg[s:e]
        if len(seg) < 3 * fs:
            continue
        f = filtfilt(b, a, seg - seg.mean())
        pk, _ = find_peaks(f, distance=int(0.3 * fs), prominence=0.5 * float(np.std(f)))
        out.append((pk + s) / fs)
    return np.concatenate(out) if out else np.array([])


def spectrogram(ppg: np.ndarray, acc: np.ndarray, fs: float, win=8.0, step=2.0, lo=0.6, hi=3.5, supp=0.9):
    """Per-frame normalised PPG power in [lo, hi] Hz with accelerometer lines suppressed."""
    from scipy.signal import butter, filtfilt, get_window
    n, s = int(win * fs), int(step * fs)
    nfft = 1 << int(np.ceil(np.log2(n * 8)))
    freqs = np.fft.rfftfreq(nfft, 1 / fs)
    band = (freqs >= lo) & (freqs <= hi)
    pf = freqs[band]
    w = get_window("hann", n)
    b, a = butter(2, [lo / (fs / 2), hi / (fs / 2)], btype="band")
    frames, times = [], []
    for st in range(0, len(ppg) - n + 1, s):
        seg, aseg = ppg[st:st + n], acc[st:st + n]
        if not (np.all(np.isfinite(seg)) and np.all(np.isfinite(aseg))):
            continue
        P = np.abs(np.fft.rfft(filtfilt(b, a, seg - seg.mean()) * w, nfft))[band] ** 2
        A = np.abs(np.fft.rfft((aseg - aseg.mean()) * w, nfft))[band] ** 2
        P, A = P / P.max(), A / A.max()
        Asup = A.copy()
        for k in (2.0, 0.5):  # harmonics and sub-harmonics of the motion line
            Asup = np.maximum(Asup, np.interp(pf, pf * k, A, left=0, right=0))
        frames.append(P * (1 - supp * Asup))
        times.append((st + n / 2) / fs)
    return np.array(times), pf * 60.0, np.array(frames)


def ppg_spectral(ppg, acc, fs):
    t, bpm, S = spectrogram(ppg, acc, fs)
    if len(t) == 0:
        return t, np.array([])
    return t, bpm[S.argmax(axis=1)]


def ppg_tracked(ppg, acc, fs, jump_pen=0.03):
    t, bpm, S = spectrogram(ppg, acc, fs)
    if len(t) == 0:
        return t, np.array([])
    L = np.log(S + 1e-6)
    D = np.abs(bpm[:, None] - bpm[None, :]) * jump_pen
    cost = L[0].copy()
    back = np.zeros(L.shape, dtype=int)
    for i in range(1, len(L)):
        M = cost[None, :] - D
        back[i] = M.argmax(axis=1)
        cost = L[i] + M.max(axis=1)
    path = np.zeros(len(L), dtype=int)
    path[-1] = int(cost.argmax())
    for i in range(len(L) - 1, 0, -1):
        path[i - 1] = back[i, path[i]]
    return t, bpm[path]


# ---------------------------------------------------------------- writing

def rounded(a: np.ndarray, nd: int) -> list[float]:
    return [round(float(x), nd) for x in a]


def write_series(root: str, rec: str, slug: str, role: str, device: str, method: str, inputs: list, samples: dict) -> None:
    doc = {
        "recording": rec,
        "role": role,
        "device": device,
        "method": method,
        "adapter": ADAPTER,
        "inputs": inputs,
        "samples": samples,
    }
    path = os.path.join(root, "series", f"{rec}.{slug}.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(doc, f, separators=(",", ":"))
        f.write("\n")


def cmd_series(root: str) -> None:
    import wfdb
    os.makedirs(os.path.join(root, "series"), exist_ok=True)
    for rec in records(os.path.join(root, "corpus")):
        base = os.path.join(root, "corpus", "records", rec)
        r = wfdb.rdrecord(base)
        ann = wfdb.rdann(base, "atr")
        fs = float(r.fs)
        names = list(r.sig_name)
        ppg = r.p_signal[:, names.index("wrist_ppg")]
        acc_cols = [names.index(f"wrist_low_noise_accelerometer_{ax}") for ax in "xyz"]
        acc = np.sqrt((r.p_signal[:, acc_cols] ** 2).sum(axis=1))
        ref = ann.sample / fs
        inputs = [{"path": f"records/{rec}.{ext}", "sha256": sha256(f"{base}.{ext}")} for ext in ("hea", "dat", "atr")]
        raw_inputs = [i for i in inputs if not i["path"].endswith(".atr")]

        write_series(root, rec, "reference", "reference", REFERENCE_DEVICE,
                     "Beat times = .atr annotation sample index / 256 Hz. No processing.",
                     inputs, {"kind": "beats", "t_s": rounded(ref, 5)})

        pk = ppg_peaks(ppg, fs)
        if len(pk) >= 2:
            write_series(root, rec, "ppg-peaks", "device", f"{PPG_DEVICE}; ppg-peaks",
                         "Butterworth order-2 band-pass 0.5–4 Hz (filtfilt) on wrist_ppg, "
                         "scipy.signal.find_peaks with distance 0.3 s and prominence 0.5·SD; NaN runs skipped.",
                         raw_inputs, {"kind": "beats", "t_s": rounded(pk, 5)})

        t, hr = ppg_spectral(ppg, acc, fs)
        if len(t) >= 2:
            write_series(root, rec, "ppg-spectral", "device", f"{PPG_DEVICE}; ppg-spectral",
                         "8 s Hann frames every 2 s; band-pass 0.6–3.5 Hz; rate = argmax of PPG power "
                         "after multiplying by (1 − 0.9·accelerometer-magnitude power, incl. ×2 and ×½ lines).",
                         raw_inputs, {"kind": "rate", "t_s": rounded(t, 3), "bpm": rounded(hr, 2)})

        t, hr = ppg_tracked(ppg, acc, fs)
        if len(t) >= 2:
            write_series(root, rec, "ppg-tracked", "device", f"{PPG_DEVICE}; ppg-tracked",
                         "Same spectrogram as ppg-spectral; rate = Viterbi path maximising "
                         "Σ log power − 0.03·|Δbpm| between consecutive frames.",
                         raw_inputs, {"kind": "rate", "t_s": rounded(t, 3), "bpm": rounded(hr, 2)})

        # Known-answer controls, derived from the reference and declared as such.
        write_series(root, rec, "control-identity", "device", "control: reference beats, identity",
                     "The reference beat times, unchanged. Must align at 0 s with r = 1 and meet every threshold.",
                     inputs, {"kind": "beats", "t_s": rounded(ref, 5)})
        write_series(root, rec, "control-shift", "device", "control: reference beats shifted +60 s",
                     "The reference beat times plus 60 s. Alignment must recover a lag of −60 s; agreement must then be exact.",
                     inputs, {"kind": "beats", "t_s": rounded(ref + 60.0, 5)})
        rate_t = ref[1:]
        rate = 60.0 / np.diff(ref) * 1.15
        write_series(root, rec, "control-fast", "device", "control: reference rate ×1.15",
                     "Instantaneous reference rate (60/RR at the second beat) multiplied by 1.15. Must fail a 10% MAPE threshold.",
                     inputs, {"kind": "rate", "t_s": rounded(rate_t, 5), "bpm": rounded(rate, 3)})
        print(rec, "ok")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["corpus", "series"])
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    a = ap.parse_args()
    if a.cmd == "corpus":
        cmd_corpus(os.path.join(a.root, "corpus"))
    else:
        cmd_series(a.root)


if __name__ == "__main__":
    sys.exit(main())
