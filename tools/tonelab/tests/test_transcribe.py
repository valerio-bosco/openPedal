import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tonelab.synth import synth_di  # noqa: E402
from tonelab.transcribe import transcribe  # noqa: E402

SR = 44100


def test_roundtrip_melody():
    pool = [47, 50, 54, 59, 58, 61, 52, 64, 55, 57, 62, 49]
    notes = [{"t": 0.2 + i * 0.4, "dur": 0.4, "midi": m, "amp": 0.1} for i, m in enumerate(pool)]
    y = synth_di(notes, SR)
    got = transcribe(y, SR)
    hits = 0
    for n in notes:
        if any(g["midi"] == n["midi"] and abs(g["t"] - n["t"]) <= 0.03 for g in got):
            hits += 1
    assert hits / len(notes) >= 0.85, (hits, [(g["t"], g["midi"]) for g in got])


def test_octave_flag():
    notes = [
        {"t": 0.2, "dur": 0.6, "midi": 35, "amp": 0.1},  # B1, below E2
        {"t": 1.0, "dur": 0.6, "midi": 59, "amp": 0.1},
    ]
    got = transcribe(synth_di(notes, SR), SR)
    low = [g for g in got if abs(g["t"] - 0.2) < 0.05]
    assert low and low[0]["octave_shifted"] and low[0]["midi"] >= 40
    assert low[0]["midi"] % 12 == 35 % 12
    hi = [g for g in got if abs(g["t"] - 1.0) < 0.05]
    assert hi and not hi[0]["octave_shifted"]
