"""`fetch`: reference (path or URL) -> new job folder with job.json + mix.wav."""
from __future__ import annotations

import datetime as _dt
import shutil
from pathlib import Path

from . import sources
from .job import Job, new_job_id


def fetch(ref: str, cfg: dict, jobs: Path, title: str | None = None) -> Job:
    kind = "youtube" if sources.is_url(ref) else "file"
    src = sources.load_source(kind, cfg)  # fails early when YouTube is disabled
    jobs.mkdir(parents=True, exist_ok=True)
    tmp = jobs / f".incoming-{_dt.datetime.now():%H%M%S%f}"
    tmp.mkdir()
    try:
        meta = src.fetch(ref, tmp)
        song_title = title or meta["title"]
        jid = new_job_id(song_title, jobs)
        path = jobs / jid
        tmp.rename(path)
    except BaseException:
        shutil.rmtree(tmp, ignore_errors=True)
        raise
    from .sources.file import info

    rate, dur = info(path / "mix.wav")
    job = Job(path)
    job.write("job.json", {
        "id": jid,
        "source": {"kind": meta["kind"], "ref": meta["ref"], "title": song_title},
        "created": _dt.datetime.now().astimezone().isoformat(timespec="seconds"),
        "sample_rate": rate,
        "duration_s": round(dur, 3),
    })
    return job
