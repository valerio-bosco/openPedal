"""`analyse`: windows of the guitar stem -> kind, level, brightness; tempo, key, low notes."""
from __future__ import annotations

import numpy as np
import soundfile as sf

from .job import Job

WIN_S = 4.0
CORR_CENTRE = 0.8
CHROMA_DOUBLE = 0.9
QUIET_ABS_DB = -55.0      # below this the guitar is absent
QUIET_REL_DB = 30.0       # or this far under the loudest window
LOW_HZ = 80.0
EPS = 1e-12

_KS_MAJOR = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
_KS_MINOR = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])
_NOTES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def _db(x: float) -> float:
    return float(20 * np.log10(max(x, EPS)))


def _rms(x: np.ndarray) -> float:
    return float(np.sqrt(np.mean(np.square(x, dtype=np.float64)))) if x.size else 0.0


def _corr(l: np.ndarray, r: np.ndarray) -> float:
    l = l - l.mean()
    r = r - r.mean()
    d = np.sqrt(np.sum(l * l) * np.sum(r * r))
    return float(np.sum(l * r) / d) if d > EPS else 1.0


def _chroma_sim(l: np.ndarray, r: np.ndarray, sr: int) -> float:
    import librosa

    cl = librosa.feature.chroma_stft(y=l, sr=sr).mean(axis=1)
    cr = librosa.feature.chroma_stft(y=r, sr=sr).mean(axis=1)
    d = np.linalg.norm(cl) * np.linalg.norm(cr)
    return float(np.dot(cl, cr) / d) if d > EPS else 0.0


def _centroid(x: np.ndarray, sr: int) -> float:
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x))))
    freqs = np.fft.rfftfreq(len(x), 1 / sr)
    s = spec.sum()
    return float((freqs * spec).sum() / s) if s > EPS else 0.0


def window_features(g: np.ndarray, bass: np.ndarray | None, sr: int) -> dict:
    """g, bass: (n, 2) arrays for one window."""
    l, r = g[:, 0], g[:, 1]
    mid, side = (l + r) / 2, (l - r) / 2
    return {
        "guitar_db": _db(_rms(g)),
        "corr": _corr(l, r),
        "side_mid_db": _db(_rms(side)) - _db(_rms(mid)),
        "centroid_hz": _centroid(mid, sr),
        "bass_db": _db(_rms(bass)) if bass is not None else -120.0,
        "chroma_sim": _chroma_sim(l, r, sr),
    }


def classify(f: dict, loudest_db: float) -> str:
    if f["guitar_db"] < QUIET_ABS_DB or f["guitar_db"] < loudest_db - QUIET_REL_DB:
        return "none"
    if f["corr"] >= CORR_CENTRE:
        return "centre"
    return "double" if f["chroma_sim"] >= CHROMA_DOUBLE else "wide"


def merge_sections(wins: list[dict]) -> list[dict]:
    out: list[dict] = []
    for w in wins:
        if out and out[-1]["kind"] == w["kind"]:
            out[-1]["_w"].append(w)
            out[-1]["end"] = w["end"]
        else:
            out.append({"kind": w["kind"], "start": w["start"], "end": w["end"], "_w": [w]})
    res = []
    for s in out:
        ws = s.pop("_w")
        avg = lambda k: round(float(np.mean([w[k] for w in ws])), 2)
        res.append({
            "start": round(s["start"], 2), "end": round(s["end"], 2),
            "guitar_db": avg("guitar_db"), "corr": avg("corr"),
            "side_mid_db": avg("side_mid_db"), "centroid_hz": round(avg("centroid_hz")),
            "kind": s["kind"], "bass_db": avg("bass_db"),
        })
    return res


def estimate_key(y: np.ndarray, sr: int) -> str:
    import librosa

    chroma = librosa.feature.chroma_cqt(y=y, sr=sr).mean(axis=1)
    best, name = -2.0, "C major"
    for tonic in range(12):
        for prof, mode in ((_KS_MAJOR, "major"), (_KS_MINOR, "minor")):
            c = float(np.corrcoef(chroma, np.roll(prof, tonic))[0, 1])
            if c > best:
                best, name = c, f"{_NOTES[tonic]} {mode}"
    return name


def find_low_notes(guitar_mono: np.ndarray, sr: int, windows: list[dict],
                   bass_thresh_db: float) -> list[dict]:
    """pYIN below LOW_HZ in the guitar stem, only where the bass stem is quiet."""
    import librosa

    sr_p = 16000
    y = librosa.resample(guitar_mono.astype(np.float32), orig_sr=sr, target_sr=sr_p)
    hop = 512
    notes: list[dict] = []
    for w in windows:
        if w["kind"] == "none" or w["bass_db"] > bass_thresh_db:
            continue
        seg = y[int(w["start"] * sr_p): int(w["end"] * sr_p)]
        if len(seg) < 4096:
            continue
        f0, voiced, _ = librosa.pyin(seg, fmin=30, fmax=400, sr=sr_p, frame_length=4096,
                                     hop_length=hop)
        t = w["start"] + librosa.times_like(f0, sr=sr_p, hop_length=hop)
        ok = voiced & np.isfinite(f0) & (f0 < LOW_HZ)
        for ti, fi in zip(t[ok], f0[ok]):
            hz = float(fi)
            if notes and ti - notes[-1]["end"] <= 0.1 and abs(np.log2(hz / np.median(notes[-1]["_hz"]))) < 0.15:
                notes[-1]["end"] = float(ti + hop / sr_p)
                notes[-1]["_hz"].append(hz)
            else:
                notes.append({"start": float(ti), "end": float(ti + hop / sr_p), "_hz": [hz]})
    out = []
    for n in notes:
        if n["end"] - n["start"] < 0.15:
            continue
        out.append({"start": round(n["start"], 2), "end": round(n["end"], 2),
                    "hz": round(float(np.median(n["_hz"])), 1)})
    return out


def analyse(job: Job) -> dict:
    import librosa

    g, sr = sf.read(str(job.stem("guitar")), dtype="float32", always_2d=True)
    if g.shape[1] == 1:
        g = np.repeat(g, 2, axis=1)
    bass = None
    if job.stem("bass").exists():
        bass, _ = sf.read(str(job.stem("bass")), dtype="float32", always_2d=True)
    n = int(WIN_S * sr)
    wins: list[dict] = []
    for i in range(0, len(g), n):
        seg = g[i:i + n]
        if len(seg) < n // 2 and wins:
            wins[-1]["end"] = len(g) / sr  # fold the short tail into the last window
            break
        f = window_features(seg, bass[i:i + n] if bass is not None else None, sr)
        f["start"], f["end"] = i / sr, min((i + n) / sr, len(g) / sr)
        wins.append(f)
    loudest = max(w["guitar_db"] for w in wins)
    for w in wins:
        w["kind"] = classify(w, loudest)

    mix, msr = sf.read(str(job.mix), dtype="float32", always_2d=True)
    mono = librosa.resample(mix.mean(axis=1), orig_sr=msr, target_sr=22050)
    tempo, _ = librosa.beat.beat_track(y=mono, sr=22050, hop_length=256)
    tempo = float(np.atleast_1d(tempo)[0])
    key = estimate_key(mono, 22050)

    active = [w["bass_db"] for w in wins if w["kind"] != "none"]
    bass_thresh = (float(np.median(active)) - 6.0) if active else -120.0
    low = find_low_notes(g.mean(axis=1), sr, wins, bass_thresh) if bass is not None else []

    result = {"tempo_bpm": round(tempo, 1), "key": key,
              "sections": merge_sections(wins), "low_notes": low}
    job.write("analysis.json", result)
    return result


def format_table(a: dict) -> str:
    lines = [f"tempo {a['tempo_bpm']:.0f} BPM, key {a['key']}",
             f"{'start':>6} {'end':>6}  {'kind':<7} {'gtr dB':>7} {'corr':>5} {'S/M dB':>7} "
             f"{'centroid':>8} {'bass dB':>7}"]
    for s in a["sections"]:
        lines.append(f"{s['start']:6.1f} {s['end']:6.1f}  {s['kind']:<7} {s['guitar_db']:7.1f} "
                     f"{s['corr']:5.2f} {s['side_mid_db']:7.1f} {s['centroid_hz']:8.0f} "
                     f"{s['bass_db']:7.1f}")
    if a["low_notes"]:
        lines.append("low notes (octave-down hint): " + ", ".join(
            f"{n['hz']:.0f} Hz {n['start']:.0f}-{n['end']:.0f}s" for n in a["low_notes"]))
    return "\n".join(lines)
