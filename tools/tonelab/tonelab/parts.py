"""`parts` and `synth-di`: pick a section + channel, transcribe it, synthesise a DI."""
from __future__ import annotations

import numpy as np
import soundfile as sf

from .job import Job

PART_RATE = 48000
CHANNELS = ("mid", "left", "right")


class MissingModule(RuntimeError):
    pass


def extract_channel(stereo: np.ndarray, channel: str) -> np.ndarray:
    if channel not in CHANNELS:
        raise ValueError(f"channel must be one of {CHANNELS}")
    if stereo.shape[1] == 1:
        return stereo[:, 0]
    return {"mid": (stereo[:, 0] + stereo[:, 1]) / 2, "left": stereo[:, 0],
            "right": stereo[:, 1]}[channel]


def to_rate(y: np.ndarray, sr: int, target: int) -> np.ndarray:
    if sr == target:
        return y.astype(np.float32)
    from scipy.signal import resample_poly
    from math import gcd

    g = gcd(sr, target)
    return resample_poly(y, target // g, sr // g).astype(np.float32)


def _transcribe(target: np.ndarray, sr: int) -> list[dict]:
    try:
        from tonelab.transcribe import transcribe
    except ImportError as e:
        raise MissingModule(
            "tonelab.transcribe is not available yet (branch feat/tonelab-transcribe not merged): "
            f"{e}") from e
    return transcribe(target, sr)


def add_part(job: Job, start: float, end: float, channel: str, label: str | None = None) -> dict:
    if end <= start:
        raise ValueError("--end must be greater than --start")
    g, sr = sf.read(str(job.stem("guitar")), dtype="float32", always_2d=True)
    seg = g[int(start * sr): int(end * sr)]
    if len(seg) == 0:
        raise ValueError("the selected range is outside the song")
    target = to_rate(extract_channel(seg, channel), sr, PART_RATE)
    parts = job.read("parts.json", default=[])
    pid = f"p{max([int(p['id'][1:]) for p in parts] + [0]) + 1}"
    d = job.part_dir(pid)
    d.mkdir(parents=True, exist_ok=True)
    sf.write(str(d / "target.wav"), target, PART_RATE, subtype="FLOAT")
    part = {"id": pid, "label": label or f"{channel} {start:g}-{end:g}s", "start": start,
            "end": end, "channel": channel, "notes": []}
    warning = None
    try:
        part["notes"] = _transcribe(target, PART_RATE)
    except MissingModule as e:
        warning = str(e)
    parts.append(part)
    job.write("parts.json", parts)
    part["_warning"] = warning
    return part


def synth_part(job: Job, part_id: str, match_di: str | None = None):
    parts = job.read("parts.json", default=[])
    part = next((p for p in parts if p["id"] == part_id), None)
    if part is None:
        raise KeyError(f"no part {part_id} in {job.id} (see parts.json)")
    if not part["notes"]:
        raise ValueError(f"part {part_id} has no notes (transcription missing or empty)")
    try:
        from tonelab.synth import synth_di
    except ImportError as e:
        raise MissingModule(
            "tonelab.synth is not available yet (branch feat/tonelab-transcribe not merged): "
            f"{e}") from e
    di = None
    if match_di:
        y, sr = sf.read(match_di, dtype="float32", always_2d=True)
        di = to_rate(y.mean(axis=1), sr, PART_RATE)
    out = synth_di(part["notes"], sr=PART_RATE, match_di=di)
    path = job.part_dir(part_id) / "di-synth.wav"
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(path), np.asarray(out, dtype=np.float32), PART_RATE, subtype="FLOAT")
    return path
