"""Job folder (PLAN §14.2): layout and JSON helpers."""
from __future__ import annotations

import datetime as _dt
import json
import re
import unicodedata
from pathlib import Path

STEMS = ("guitar", "bass", "drums", "vocals", "piano", "other")


def slugify(text: str) -> str:
    t = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    t = re.sub(r"[^a-zA-Z0-9]+", "-", t).strip("-").lower()
    return t[:48] or "song"


def new_job_id(title: str, jobs: Path, today: _dt.date | None = None) -> str:
    base = f"{(today or _dt.date.today()).isoformat()}-{slugify(title)}"
    jid, n = base, 2
    while (jobs / jid).exists():
        jid, n = f"{base}-{n}", n + 1
    return jid


class Job:
    def __init__(self, path: Path):
        self.path = Path(path)

    @classmethod
    def open(cls, jobs: Path, ref: str) -> "Job":
        p = Path(ref)
        if not (p / "job.json").exists():
            p = jobs / ref
        if not (p / "job.json").exists():
            raise FileNotFoundError(f"job not found: {ref} (looked in {jobs})")
        return cls(p)

    @property
    def id(self) -> str:
        return self.path.name

    @property
    def mix(self) -> Path:
        return self.path / "mix.wav"

    @property
    def stems(self) -> Path:
        return self.path / "stems"

    def stem(self, name: str) -> Path:
        return self.stems / f"{name}.wav"

    def part_dir(self, part_id: str) -> Path:
        return self.path / "parts" / part_id

    def read(self, name: str, default=None):
        p = self.path / name
        if not p.exists():
            if default is not None:
                return default
            raise FileNotFoundError(f"{p} missing (run the earlier command first)")
        return json.loads(p.read_text())

    def write(self, name: str, data) -> None:
        (self.path / name).write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
