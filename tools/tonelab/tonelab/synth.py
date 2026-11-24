"""Synthetic clean guitar DI from transcribed notes (Karplus-Strong)."""
from __future__ import annotations

import numpy as np
from scipy import signal

TARGET_RMS_DB = -29.8

# Measured on a real passive-pickup DI: dB relative to the max 1/3-octave band.
DEFAULT_TILT = (
    (100, -4), (250, 0), (500, -3), (1000, -13), (1600, -16),
    (2500, -17), (4000, -28), (6300, -35), (10000, -48),
)


def _pluck(freq: float, n: int, sr: int, rng: np.random.Generator) -> np.ndarray:
    period = sr / freq
    damp = 0.996 if freq < 250 else 0.993
    d_total = period - 0.5  # linear interpolation of two taps delays by ~D + f
    d = max(int(np.floor(d_total)), 2)
    f = d_total - d
    a = np.zeros(d + 2)
    a[0] = 1.0
    a[d] -= damp * (1 - f)
    a[d + 1] -= damp * f
    burst = rng.uniform(-1, 1, d + 2)
    comb = max(int(round(0.18 * period)), 1)
    shaped = burst.copy()
    shaped[comb:] -= burst[:-comb]
    x = np.zeros(n)
    m = min(len(shaped), n)
    x[:m] = shaped[:m]
    y = signal.lfilter([1.0], a, x)
    peak = np.max(np.abs(y))
    return y / peak if peak > 0 else y


def _pickup(x: np.ndarray, sr: int) -> np.ndarray:
    w0 = 4500 / (sr / 2)
    b, a = signal.iirpeak(w0, Q=1.5)
    peak = signal.lfilter(b, a, x)
    x = x + 1.5 * peak
    sos = signal.butter(2, 6000, "low", fs=sr, output="sos")
    x = signal.sosfilt(sos, x)
    sos = signal.butter(2, 70, "high", fs=sr, output="sos")
    return signal.sosfilt(sos, x)


def _third_octave_centres(lo=40.0, hi=10000.0):
    n0 = int(np.floor(3 * np.log2(lo / 1000))) 
    n1 = int(np.ceil(3 * np.log2(hi / 1000)))
    return 1000 * 2 ** (np.arange(n0, n1 + 1) / 3)


def _band_db(x: np.ndarray, sr: int, centres: np.ndarray) -> np.ndarray:
    f, p = signal.welch(x, sr, nperseg=8192)
    out = np.full(len(centres), -200.0)
    for i, c in enumerate(centres):
        m = (f >= c / 2 ** (1 / 6)) & (f < c * 2 ** (1 / 6))
        if m.any():
            out[i] = 10 * np.log10(p[m].sum() + 1e-30)
    return out


def _fir_from_gain(freqs, gains_db, sr, taps=2047):
    f = np.concatenate([[0], freqs, [sr / 2]])
    g = np.concatenate([[gains_db[0]], gains_db, [gains_db[-1]]])
    g = np.clip(g, -40, 12)
    return signal.firwin2(taps, f, 10 ** (g / 20), fs=sr)


def _match(x: np.ndarray, ref: np.ndarray, sr: int) -> np.ndarray:
    f, px = signal.welch(x, sr, nperseg=8192)
    _, pr = signal.welch(ref, sr, nperseg=8192)
    ratio = 10 * np.log10((pr + 1e-30) / (px + 1e-30))
    # smooth over 1/3 octave, on a log-frequency grid
    lf = np.log2(np.maximum(f[1:], 1.0))
    grid = np.linspace(lf[0], lf[-1], 1500)
    r = np.interp(grid, lf, ratio[1:])
    k = max(int(round(1500 / (lf[-1] - lf[0]) / 3)), 1)
    r = np.convolve(np.pad(r, (k, k), mode="edge"), np.ones(2 * k + 1) / (2 * k + 1), "valid")
    fg = 2 ** grid
    r -= np.median(r[(fg >= 100) & (fg <= 1000)])
    h = _fir_from_gain(fg, r, sr)
    return signal.fftconvolve(x, h)[len(h) // 2 : len(h) // 2 + len(x)]


def _default_tilt(x: np.ndarray, sr: int) -> np.ndarray:
    fr = np.array([t[0] for t in DEFAULT_TILT], float)
    db = np.array([t[1] for t in DEFAULT_TILT], float)
    cen = _third_octave_centres()
    target = np.interp(np.log(cen), np.log(fr), db)
    # below 100 Hz and above 10 kHz keep sloping like the nearest segment
    lo = db[0] + (db[1] - db[0]) / np.log(fr[1] / fr[0]) * np.log(cen / fr[0])
    hi = db[-1] + (db[-1] - db[-2]) / np.log(fr[-1] / fr[-2]) * np.log(cen / fr[-1])
    target = np.where(cen < fr[0], lo, np.where(cen > fr[-1], hi, target))
    cur = _band_db(x, sr, cen)
    gain = target - cur
    gain -= np.median(gain[(cen >= 100) & (cen <= 1000)])
    h = _fir_from_gain(cen, gain, sr)
    return signal.fftconvolve(x, h)[len(h) // 2 : len(h) // 2 + len(x)]


def synth_di(
    notes: list[dict], sr: int = 48000, match_di: np.ndarray | None = None
) -> np.ndarray:
    """Synthesise a clean guitar DI (float32, mono) from `notes`."""
    if not notes:
        return np.zeros(sr // 10, dtype=np.float32)
    rng = np.random.default_rng(0)
    total = max(n["t"] + n["dur"] for n in notes)
    out = np.zeros(int(total * sr) + sr // 4)
    max_amp = max(n["amp"] for n in notes) or 1.0
    rel = int(0.012 * sr)
    for n in notes:
        freq = 440.0 * 2 ** ((n["midi"] - 69) / 12)
        length = max(int(n["dur"] * sr), rel * 2)
        y = _pluck(freq, length, sr, rng)
        y[-rel:] *= np.linspace(1, 0, rel)
        gain = 0.4 + 0.6 * np.sqrt(n["amp"] / max_amp)
        s = int(n["t"] * sr)
        e = min(s + length, len(out))
        out[s:e] += gain * y[: e - s]
    out = _pickup(out, sr)
    if match_di is not None:
        ref = np.asarray(match_di, dtype=np.float64)
        if ref.ndim > 1:
            ref = ref[:, 0] if ref.shape[1] < ref.shape[0] else ref[0]
        out = _match(out, ref, sr)
    else:
        out = _default_tilt(out, sr)
    # scale so the RMS of samples above 0.003 (after scaling) is the target
    active = out[np.abs(out) > 1e-12]
    target = 10 ** (TARGET_RMS_DB / 20)
    scale = target / (np.sqrt(np.mean(active**2)) + 1e-30)
    for _ in range(8):
        s_out = out * scale
        act = s_out[np.abs(s_out) > 0.003]
        if len(act) == 0:
            break
        r = np.sqrt(np.mean(act**2))
        scale *= target / r
        if abs(20 * np.log10(r / target)) < 0.01:
            break
    return (out * scale).astype(np.float32)
