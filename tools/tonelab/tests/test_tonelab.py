import json
import sys
import types
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

from tonelab import analyse as an
from tonelab import sources
from tonelab.config import ConfigError, load_config, resolve_device
from tonelab.job import Job, new_job_id, slugify

SR = 22050


def tone(freqs, dur, seed, detune=0.0):
    rng = np.random.default_rng(seed)
    t = np.arange(int(dur * SR)) / SR
    y = np.zeros_like(t)
    for f in freqs:
        for h in range(1, 6):
            y += np.sin(2 * np.pi * f * (1 + detune) * h * t + rng.uniform(0, 6.28)) / h
    return (0.2 * y / np.abs(y).max()).astype(np.float32)


def test_config_defaults_and_local_override(tmp_path):
    (tmp_path / "tonelab.toml").write_text(
        '[sources]\nyoutube = true\n[gpu]\ndevice = "cuda"\n[paths]\njobs = "~/j"\n')
    assert load_config(tmp_path)["sources"]["youtube"] is True
    (tmp_path / "tonelab.local.toml").write_text('[sources]\nyoutube = false\n[gpu]\ndevice = "cpu"\n')
    cfg = load_config(tmp_path)
    assert cfg["sources"]["youtube"] is False and cfg["gpu"]["device"] == "cpu"
    assert cfg["paths"]["jobs"] == "~/j"
    assert resolve_device(cfg) == "cpu"


def test_committed_defaults():
    cfg = load_config(Path(__file__).resolve().parent.parent)
    assert cfg["gpu"]["device"] == "cuda" and cfg["separate"]["segment"] == 7


def test_cuda_missing_fails_clearly(monkeypatch):
    import torch

    monkeypatch.setattr(torch.cuda, "is_available", lambda: False)
    with pytest.raises(ConfigError, match="CUDA is not available"):
        resolve_device({"gpu": {"device": "cuda"}})


def test_youtube_disabled(monkeypatch):
    with pytest.raises(sources.SourceError, match="disabled"):
        sources.load_source("youtube", {"sources": {"youtube": False}})
    assert "tonelab.sources.youtube" not in sys.modules


def test_youtube_extra_missing(monkeypatch):
    monkeypatch.delitem(sys.modules, "tonelab.sources.youtube", raising=False)
    monkeypatch.setitem(sys.modules, "yt_dlp", None)  # import fails
    with pytest.raises(sources.SourceError, match="uv sync --extra youtube"):
        sources.load_source("youtube", {"sources": {"youtube": True}})


def test_youtube_mocked(monkeypatch, tmp_path):
    class FakeYDL:
        def __init__(self, opts):
            self.opts = opts

        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

        def extract_info(self, url, download):
            out = Path(self.opts["outtmpl"].replace("%(ext)s", "wav"))
            sf.write(str(out), tone([110], 1, 1), SR)
            return {"title": "Fake Song"}

    monkeypatch.setitem(sys.modules, "yt_dlp", types.SimpleNamespace(YoutubeDL=FakeYDL))
    monkeypatch.delitem(sys.modules, "tonelab.sources.youtube", raising=False)
    mod = sources.load_source("youtube", {"sources": {"youtube": True}})
    meta = mod.fetch("https://youtu.be/x", tmp_path)
    assert meta["title"] == "Fake Song" and (tmp_path / "mix.wav").exists()
    sf.info(str(tmp_path / "mix.wav"))


def test_file_source_and_fetch(tmp_path):
    from tonelab.fetch import fetch

    src = tmp_path / "My Song!.wav"
    sf.write(str(src), tone([110], 2, 1), SR)  # mono in, stereo out
    cfg = load_config()
    job = fetch(str(src), cfg, tmp_path / "jobs")
    info = sf.info(str(job.mix))
    assert info.channels == 2 and info.samplerate == SR and info.subtype == "FLOAT"
    meta = json.loads((job.path / "job.json").read_text())
    assert meta["source"]["kind"] == "file" and meta["sample_rate"] == SR
    assert meta["id"].endswith("-my-song") and abs(meta["duration_s"] - 2) < 0.01
    assert fetch(str(src), cfg, tmp_path / "jobs").id.endswith("-my-song-2")


def test_job_helpers(tmp_path):
    assert slugify("Muse - Cryogen (live)") == "muse-cryogen-live"
    j = Job(tmp_path)
    j.write("parts.json", [{"id": "p1"}])
    assert j.read("parts.json") == [{"id": "p1"}]
    with pytest.raises(FileNotFoundError):
        j.read("analysis.json")


def make_job(tmp_path, left, right):
    (tmp_path / "stems").mkdir()
    st = np.stack([left, right], axis=1)
    sf.write(str(tmp_path / "stems" / "guitar.wav"), st, SR, subtype="FLOAT")
    sf.write(str(tmp_path / "mix.wav"), st, SR, subtype="FLOAT")
    return Job(tmp_path)


def test_analyse_centre_double_none(tmp_path):
    notes = [110.0, 123.47, 146.83]
    parts = []
    # 8 s centre, 8 s double-tracked (same notes, independent phases, slight detune), 8 s silence
    c = tone(notes, 8, 1)
    dl, dr = tone(notes, 8, 2), tone(notes, 8, 3, detune=0.004)
    s = np.zeros(8 * SR, np.float32)
    job = make_job(tmp_path, np.concatenate([c, dl, s]), np.concatenate([c, dr, s]))
    a = an.analyse(job)
    kinds = [(x["start"], x["end"], x["kind"]) for x in a["sections"]]
    assert kinds == [(0.0, 8.0, "centre"), (8.0, 16.0, "double"), (16.0, 24.0, "none")]
    assert a["sections"][0]["corr"] > 0.95 and a["sections"][1]["corr"] < 0.8
    assert (tmp_path / "analysis.json").exists()
    assert "double" in an.format_table(a)


def test_parts_without_transcribe(tmp_path, monkeypatch):
    from tonelab import parts

    job = make_job(tmp_path, tone([110], 4, 1), tone([110], 4, 1))
    monkeypatch.setattr(parts, "_transcribe", lambda t, sr: (_ for _ in ()).throw(
        parts.MissingModule("not merged")))
    p = parts.add_part(job, 1, 3, "mid", "riff")
    assert p["id"] == "p1" and p["_warning"]
    t, sr = sf.read(str(job.part_dir("p1") / "target.wav"))
    assert sr == 48000 and t.ndim == 1 and abs(len(t) - 2 * 48000) < 10
    assert parts.add_part(job, 0, 1, "left")["id"] == "p2"
    assert [x["id"] for x in job.read("parts.json")] == ["p1", "p2"]


def test_synth_di_call(tmp_path, monkeypatch):
    from tonelab import parts

    job = make_job(tmp_path, tone([110], 4, 1), tone([110], 4, 1))
    monkeypatch.setattr(parts, "_transcribe", lambda t, sr: [
        {"t": 0.0, "dur": 0.5, "midi": 45, "amp": 0.5, "octave_shifted": False}])
    parts.add_part(job, 0, 2, "mid")
    seen = {}

    def fake(notes, sr=48000, match_di=None):
        seen.update(n=len(notes), sr=sr, di=match_di)
        return np.zeros(48000, np.float32)

    monkeypatch.setitem(sys.modules, "tonelab.synth", types.SimpleNamespace(synth_di=fake))
    di = tmp_path / "di.wav"
    sf.write(str(di), tone([110], 1, 1), SR)
    out = parts.synth_part(job, "p1", str(di))
    assert out.name == "di-synth.wav" and seen["sr"] == 48000 and seen["n"] == 1
    assert seen["di"].ndim == 1


def test_analyse_wide_when_parts_differ(tmp_path):
    # different notes left and right: the frame-wise chroma similarity is low -> "wide"
    left = tone([110.0, 146.83], 8, 1)
    right = tone([174.61, 233.08], 8, 2)
    a = an.analyse(make_job(tmp_path, left, right))
    assert [s["kind"] for s in a["sections"]] == ["wide"]


def test_low_notes_need_a_silent_bass_and_a_stable_pitch(tmp_path):
    t = np.arange(6 * SR) / SR
    low = (0.3 * np.sin(2 * np.pi * 62.0 * t)).astype(np.float32)
    job = make_job(tmp_path, low, low)
    # bass stem as loud as the guitar: the low note is the bass's, not an octave-down guitar
    sf.write(str(job.stem("bass")), np.stack([low, low], axis=1), SR, subtype="FLOAT")
    assert an.analyse(job)["low_notes"] == []
    # silent bass stem: reported
    sf.write(str(job.stem("bass")), np.zeros((len(low), 2), np.float32), SR, subtype="FLOAT")
    notes = an.analyse(job)["low_notes"]
    assert notes and all(abs(n["hz"] - 62) < 3 for n in notes)
    # a glide (> 1 semitone) is not a stable note
    glide = (0.3 * np.sin(2 * np.pi * np.cumsum(np.linspace(30, 79, len(t))) / SR)).astype(np.float32)
    (tmp_path / "g").mkdir()
    job2 = make_job(tmp_path / "g", glide, glide)
    sf.write(str(job2.stem("bass")), np.zeros((len(glide), 2), np.float32), SR, subtype="FLOAT")
    assert an.analyse(job2)["low_notes"] == []


def test_caps_json_and_progress_lines(tmp_path, monkeypatch, capsys):
    from tonelab import cli

    monkeypatch.setattr(cli, "load_config", lambda: {
        "sources": {"youtube": True}, "gpu": {"device": "cpu"}, "paths": {"jobs": str(tmp_path)}})
    assert cli.main(["caps", "--json"]) == 0
    caps = json.loads(capsys.readouterr().out)
    assert caps == {"youtube": True, "device": "cpu", "jobs_dir": str(tmp_path)}
    cli.progress("fit", None, "hello world")
    cli.progress("fit", 0.5, "half")
    err = capsys.readouterr().err.splitlines()
    assert err == ["PROGRESS fit - hello world", "PROGRESS fit 0.500 half"]
