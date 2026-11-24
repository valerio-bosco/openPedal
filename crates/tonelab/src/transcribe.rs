//! Note transcription of a separated mono guitar part (port of transcribe.py).
use crate::dsp::{onset_strength, peak_pick, Fft};
use crate::job::Note;
use crate::pyin::pyin;

const HOP: usize = 256;
const FRAME: usize = 4096;
const LOWEST_MIDI: i32 = 40;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Onset frames as `librosa.onset.onset_detect(hop_length=hop, backtrack=True)`.
pub fn onset_frames(y: &[f32], sr: u32, hop: usize) -> Vec<usize> {
    let mut fft = Fft::new();
    let mut env = onset_strength(&mut fft, y, sr as f64, hop, false);
    let mn = env.iter().cloned().fold(f64::INFINITY, f64::min);
    for v in env.iter_mut() {
        *v -= mn;
    }
    let mx = env.iter().cloned().fold(0.0, f64::max);
    if mx <= 0.0 {
        return vec![];
    }
    for v in env.iter_mut() {
        *v /= mx + f64::MIN_POSITIVE;
    }
    let (srf, h) = (sr as f64, hop as f64);
    let ev = peak_pick(
        &env,
        (0.03 * srf / h).floor() as usize,
        1,
        (0.10 * srf / h).floor() as usize,
        (0.10 * srf / h).floor() as usize + 1,
        0.07,
        (0.03 * srf / h).floor() as usize,
    );
    // backtrack to the preceding local minimum of the envelope (frame 0 always allowed)
    let mut minima = vec![0usize];
    for i in 1..env.len().saturating_sub(1) {
        if env[i] <= env[i - 1] && env[i] < env[i + 1] {
            minima.push(i);
        }
    }
    let mut out: Vec<usize> = ev.iter().map(|&e| *minima.iter().rev().find(|&&m| m <= e).unwrap_or(&0)).collect();
    out.dedup();
    out
}

/// Notes `{t, dur, midi, amp}` found in `mono`. Notes below E2 are moved up by octaves and flagged.
pub fn transcribe(mono: &[f32], sr: u32) -> Vec<Note> {
    if mono.len() < FRAME {
        return vec![];
    }
    let onsets = onset_frames(mono, sr, HOP);
    if onsets.is_empty() {
        return vec![];
    }
    let p = pyin(mono, 55.0, 1100.0, sr as f64, FRAME, HOP);
    let n_frames = p.f0.len();
    let mut bounds = onsets.clone();
    bounds.push(n_frames.min(mono.len().div_ceil(HOP)));
    let mut notes = vec![];
    for (i, &start) in onsets.iter().enumerate() {
        let end = bounds[i + 1].min(n_frames);
        if end <= start {
            continue;
        }
        let mut midis: Vec<f64> = (start..end)
            .filter(|&k| p.voiced[k] && p.f0[k].is_finite())
            .map(|k| 12.0 * (p.f0[k] / 440.0).log2() + 69.0)
            .collect();
        if midis.len() < 2 {
            continue;
        }
        let mut midi = median(&mut midis).round_ties_even() as i32;
        let mut shifted = false;
        while midi < LOWEST_MIDI {
            midi += 12;
            shifted = true;
        }
        let a = (start * HOP).min(mono.len());
        let b = (end * HOP).min(mono.len());
        let amp = if b > a { (mono[a..b].iter().map(|&v| v as f64 * v as f64).sum::<f64>() / (b - a) as f64).sqrt() as f32 } else { 0.0 };
        notes.push(Note {
            t: (start * HOP) as f64 / sr as f64,
            dur: ((end - start) * HOP) as f64 / sr as f64,
            midi,
            amp: amp as f64,
            octave_shifted: shifted,
        });
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_two_plucked_notes() {
        let sr = 48000u32;
        let mut y = vec![0.0f32; sr as usize * 2];
        for (t0, f) in [(0.3, 220.0f64), (1.1, 329.63)] {
            for i in 0..(sr as usize * 6 / 10) {
                let t = i as f64 / sr as f64;
                let env = (-t * 4.0).exp() * (1.0 - (-t * 400.0).exp());
                let s = (2.0 * std::f64::consts::PI * f * t).sin() + 0.3 * (4.0 * std::f64::consts::PI * f * t).sin();
                y[(t0 * sr as f64) as usize + i] += (0.5 * env * s) as f32;
            }
        }
        let notes = transcribe(&y, sr);
        let m: Vec<i32> = notes.iter().map(|n| n.midi).collect();
        assert!(m.contains(&57) && m.contains(&64), "{m:?}");
        let n = notes.iter().find(|n| n.midi == 57).unwrap();
        assert!((n.t - 0.3).abs() < 0.05, "{}", n.t);
    }
}
