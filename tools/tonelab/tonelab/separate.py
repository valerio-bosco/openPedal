"""`separate`: Demucs htdemucs_6s on the configured device -> stems/*.wav."""
from __future__ import annotations

import shutil

import sys

import numpy as np
import soundfile as sf

from .config import resolve_device
from .job import STEMS, Job

DEMUCS_RATE = 44100
GUITAR_ONLY_FRACTION = 0.9  # guitar stem share of total stem energy that counts as "guitar only"
CHECK_SECONDS = 30


def _load_44k(path):
    import torch
    import torchaudio

    data, sr = sf.read(str(path), dtype="float32", always_2d=True)
    wav = torch.from_numpy(data.T.copy())
    if sr != DEMUCS_RATE:
        wav = torchaudio.functional.resample(wav, sr, DEMUCS_RATE)
    return wav


def _run_demucs(wav, device: str, segment: int):
    """wav: (2, n) float tensor at 44.1 kHz -> (dict name -> (2, n) numpy)."""
    import torch
    from demucs.apply import apply_model
    from demucs.pretrained import get_model

    model = get_model("htdemucs_6s")
    model.eval()
    ref = wav.mean(0)
    x = (wav - ref.mean()) / (ref.std() + 1e-8)
    with torch.no_grad():
        out = apply_model(model, x[None], device=device, split=True, overlap=0.25,
                          segment=segment, shifts=0, progress=sys.stderr.isatty())[0]
    out = out * (ref.std() + 1e-8) + ref.mean()
    if device.startswith("cuda"):
        torch.cuda.empty_cache()
    return {name: out[i].cpu().numpy() for i, name in enumerate(model.sources)}


def _guitar_fraction(stems: dict) -> float:
    e = {k: float(np.sum(v.astype(np.float64) ** 2)) for k, v in stems.items()}
    total = sum(e.values()) or 1.0
    return e.get("guitar", 0.0) / total


def looks_guitar_only(job: Job, device: str, segment: int) -> tuple[bool, float]:
    """Quick check: separate CHECK_SECONDS from the middle and look at the stem energies."""
    wav = _load_44k(job.mix)
    n = CHECK_SECONDS * DEMUCS_RATE
    if wav.shape[1] > n:
        s = (wav.shape[1] - n) // 2
        wav = wav[:, s:s + n]
    frac = _guitar_fraction(_run_demucs(wav, device, segment))
    return frac >= GUITAR_ONLY_FRACTION, frac


def separate(job: Job, cfg: dict, mode: str | None = None) -> str:
    """mode: auto | always | never (default from config). Returns what was done."""
    mode = mode or cfg["separate"]["mode"]
    if mode not in ("auto", "always", "never"):
        raise ValueError(f"separate mode must be auto, always or never, got {mode}")
    job.stems.mkdir(exist_ok=True)
    if mode == "never":
        shutil.copyfile(job.mix, job.stem("guitar"))
        return "skipped (forced): guitar.wav = mix"
    device = resolve_device(cfg)
    segment = int(cfg["separate"]["segment"])
    if mode == "auto":
        only, frac = looks_guitar_only(job, device, segment)
        if only:
            shutil.copyfile(job.mix, job.stem("guitar"))
            return f"skipped: source looks guitar only ({frac:.0%} of the energy in the guitar stem)"
    stems = _run_demucs(_load_44k(job.mix), device, segment)
    for name in STEMS:
        sf.write(str(job.stem(name)), stems[name].T, DEMUCS_RATE, subtype="FLOAT")
    return f"separated on {device} into {', '.join(STEMS)}"
