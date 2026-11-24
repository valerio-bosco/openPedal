import os
import shutil
from pathlib import Path


def pytest_configure(config):
    """Outside bin/tonelab ffmpeg may not be on PATH: use the one the wrapper cached."""
    if shutil.which("ffmpeg"):
        return
    venv = Path(os.environ.get("TONELAB_VENV", "~/.local/share/openpedal-gpu")).expanduser()
    cache = venv / "nix-paths.env"
    if cache.exists():
        for line in cache.read_text().splitlines():
            if line.startswith("TL_FFMPEG="):
                os.environ["PATH"] = line.split("=", 1)[1] + "/bin:" + os.environ["PATH"]
