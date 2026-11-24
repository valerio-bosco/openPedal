"""Note transcription of a separated, mono guitar part."""
from __future__ import annotations

import librosa
import numpy as np

HOP = 256
FRAME = 4096
LOWEST_MIDI = 40  # E2, the lowest note of a standard-tuned guitar


def transcribe(target: np.ndarray, sr: int) -> list[dict]:
    """Return notes {t, dur, midi, amp, octave_shifted} found in `target`.

    Notes below E2 cannot come from the guitar itself: they are produced by an
    octave-down effect, so they are moved up by octaves and flagged.
    """
    y = np.asarray(target, dtype=np.float32)
    if y.ndim > 1:
        y = y.mean(axis=0 if y.shape[0] < y.shape[-1] else 1)
    if len(y) < FRAME:
        return []

    onsets = librosa.onset.onset_detect(
        y=y, sr=sr, hop_length=HOP, backtrack=True, units="frames"
    )
    if len(onsets) == 0:
        return []
    f0, voiced, _ = librosa.pyin(
        y, fmin=55, fmax=1100, sr=sr, frame_length=FRAME, hop_length=HOP
    )
    n_frames = len(f0)
    bounds = list(onsets) + [min(n_frames, (len(y) + HOP - 1) // HOP)]
    notes: list[dict] = []
    for i, start in enumerate(onsets):
        end = bounds[i + 1]
        seg = f0[start:end]
        ok = voiced[start:end] & np.isfinite(seg)
        if ok.sum() < 2:
            continue
        midi = float(np.median(librosa.hz_to_midi(seg[ok])))
        midi_i = int(round(midi))
        shifted = False
        while midi_i < LOWEST_MIDI:
            midi_i += 12
            shifted = True
        a, b = start * HOP, min(len(y), end * HOP)
        seg_audio = y[a:b]
        amp = float(np.sqrt(np.mean(seg_audio**2))) if len(seg_audio) else 0.0
        notes.append(
            {
                "t": float(start * HOP / sr),
                "dur": float((end - start) * HOP / sr),
                "midi": midi_i,
                "amp": amp,
                "octave_shifted": shifted,
            }
        )
    return notes
