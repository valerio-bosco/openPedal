"""YouTube source (personal use; may break YouTube's Terms of Service).

The only file that knows about yt-dlp. Remove the feature: set [sources] youtube = false,
or delete this file and the `youtube` extra.
"""
from __future__ import annotations

import tempfile
from pathlib import Path

import yt_dlp

from .file import to_mix_wav


def fetch(ref: str, dest_dir: Path) -> dict:
    with tempfile.TemporaryDirectory(prefix="tonelab-yt-") as tmp:
        opts = {
            "format": "bestaudio/best",
            "outtmpl": str(Path(tmp) / "audio.%(ext)s"),
            "noplaylist": True,
            "quiet": True,
            "no_warnings": True,
        }
        with yt_dlp.YoutubeDL(opts) as ydl:
            meta = ydl.extract_info(ref, download=True)
        files = sorted(Path(tmp).glob("audio.*"))
        if not files:
            raise RuntimeError("yt-dlp produced no audio file")
        to_mix_wav(files[0], dest_dir / "mix.wav")
    return {"kind": "youtube", "ref": ref, "title": (meta or {}).get("title") or "youtube"}
