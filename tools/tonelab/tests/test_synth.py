import sys
import time
from pathlib import Path

import numpy as np
from scipy import signal

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tonelab.synth import _band_db, _third_octave_centres, synth_di  # noqa: E402

SR = 48000


def melody(seconds, step=0.25):
    pool = [47, 50, 54, 59, 58, 61, 52, 64]
    return [
        {"t": i * step, "dur": step, "midi": pool[i % len(pool)], "amp": 0.1 + 0.05 * (i % 3)}
        for i in range(int(seconds / step))
    ]


def rms_db(x):
    a = x[np.abs(x) > 0.003]
    return 20 * np.log10(np.sqrt(np.mean(a**2)))


def test_rms_level():
    y = synth_di(melody(8), SR)
    assert abs(rms_db(y) - (-29.8)) < 0.2


def test_match_di_spectrum():
    rng = np.random.default_rng(1)
    b, a = signal.butter(2, 1500, "low", fs=SR)
    ref = signal.lfilter(b, a, rng.standard_normal(SR * 10)) * 0.05
    y = synth_di(melody(10), SR, match_di=ref)
    cen = _third_octave_centres(200, 5000)
    d = _band_db(y, SR, cen) - _band_db(ref, SR, cen)
    d -= np.median(d)
    assert np.sqrt(np.mean(d**2)) < 3.0


def test_speed_60s():
    t0 = time.time()
    y = synth_di(melody(60), SR)
    assert time.time() - t0 < 20
    assert len(y) >= 60 * SR
