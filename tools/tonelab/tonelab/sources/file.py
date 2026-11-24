"""File source: any audio ffmpeg reads -> mix.wav (float32, stereo, original rate)."""
from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path

import soundfile as sf


def _probe_rate(path: Path) -> int:
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries",
         "stream=sample_rate", "-of", "json", str(path)],
        capture_output=True, text=True, check=True,
    ).stdout
    streams = json.loads(out).get("streams") or []
    if not streams:
        raise RuntimeError(f"no audio stream in {path}")
    return int(streams[0]["sample_rate"])


def to_mix_wav(src: Path, dest: Path) -> None:
    if not shutil.which("ffmpeg"):
        raise RuntimeError("ffmpeg not found; run through bin/tonelab (it provides nixpkgs ffmpeg)")
    rate = _probe_rate(src)
    subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-i", str(src), "-vn", "-ac", "2", "-ar", str(rate),
         "-c:a", "pcm_f32le", str(dest)],
        check=True,
    )


def fetch(ref: str, dest_dir: Path) -> dict:
    """Write dest_dir/mix.wav; return {'kind','ref','title'}."""
    src = Path(ref).expanduser()
    if not src.is_file():
        raise FileNotFoundError(f"file not found: {ref}")
    to_mix_wav(src, dest_dir / "mix.wav")
    return {"kind": "file", "ref": str(src.resolve()), "title": src.stem}


def info(mix: Path) -> tuple[int, float]:
    i = sf.info(str(mix))
    return i.samplerate, i.frames / i.samplerate
