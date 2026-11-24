//! pYIN (Mauch & Dixon 2014) as librosa implements it: YIN cumulative-mean difference, beta prior over
//! thresholds, Boltzmann prior over troughs, HMM Viterbi over pitch states (voiced + unvoiced halves).
use crate::dsp::Fft;

pub struct Pyin {
    pub f0: Vec<f64>, // NaN when unvoiced
    pub voiced: Vec<bool>,
    pub voiced_prob: Vec<f64>,
}

const N_THRESH: usize = 100;
const BOLTZMANN: f64 = 2.0;
const RESOLUTION: f64 = 0.1;
const MAX_TRANSITION_RATE: f64 = 35.92;
const SWITCH_PROB: f64 = 0.01;
const NO_TROUGH_PROB: f64 = 0.01;
const TINY: f64 = f64::MIN_POSITIVE;

fn beta_cdf(x: f64) -> f64 {
    // Beta(2, 18): 1 - (1-x)^18 (1 + 18 x)
    1.0 - (1.0 - x).powi(18) * (1.0 + 18.0 * x)
}

fn boltzmann(k: f64, n: f64) -> f64 {
    (1.0 - (-BOLTZMANN).exp()) * (-BOLTZMANN * k).exp() / (1.0 - (-BOLTZMANN * n).exp())
}

/// `y` mono; `frame_length` samples per analysis frame; centred with zero padding.
pub fn pyin(y: &[f32], fmin: f64, fmax: f64, sr: f64, frame_length: usize, hop: usize) -> Pyin {
    let mut fft = Fft::new();
    let pad = frame_length / 2;
    let mut yp = vec![0.0f64; pad];
    yp.extend(y.iter().map(|&v| v as f64));
    yp.extend(std::iter::repeat_n(0.0, pad));
    if yp.len() < frame_length {
        return Pyin { f0: vec![], voiced: vec![], voiced_prob: vec![] };
    }
    let n_frames = 1 + (yp.len() - frame_length) / hop;
    let min_period = (sr / fmax).floor() as usize;
    let max_period = ((sr / fmin).ceil() as usize).min(frame_length - 1);
    let n_per = max_period - min_period + 1;

    let thresholds: Vec<f64> = (0..=N_THRESH).map(|i| i as f64 / N_THRESH as f64).collect();
    let beta_probs: Vec<f64> = (0..N_THRESH).map(|i| beta_cdf(thresholds[i + 1]) - beta_cdf(thresholds[i])).collect();
    let nbs = (1.0 / RESOLUTION).ceil() as usize;
    let npb = (12.0 * nbs as f64 * (fmax / fmin).log2()).floor() as usize + 1;

    let nfft = (2 * frame_length).next_power_of_two();
    let mut obs = vec![0.0f64; 2 * npb * n_frames];
    let mut voiced_prob = vec![0.0f64; n_frames];
    let mut yin = vec![0.0f64; max_period + 1];
    let mut frame_yin = vec![0.0f64; n_per];

    for t in 0..n_frames {
        let fr = &yp[t * hop..t * hop + frame_length];
        let spec = fft.rfft(fr, nfft);
        let power: Vec<_> = spec.iter().map(|c| rustfft::num_complex::Complex::new(c.norm_sqr(), 0.0)).collect();
        let acf = fft.irfft(&power, nfft);
        let mut cs = 0.0;
        let mut csum = vec![0.0; max_period + 1];
        for (i, v) in fr.iter().enumerate().take(max_period + 1) {
            cs += v * v;
            csum[i] = cs;
        }
        yin[0] = 0.0;
        for k in 1..=max_period {
            yin[k] = 2.0 * (acf[0] - acf[k]) - csum[k - 1];
        }
        let mut run = 0.0;
        for k in 1..=max_period {
            run += yin[k];
            if k >= min_period {
                let den = run / k as f64;
                frame_yin[k - min_period] = yin[k] / (den + TINY);
            }
        }
        // troughs
        let mut troughs = vec![];
        for i in 0..n_per {
            let left = if i == 0 { frame_yin[0] } else { frame_yin[i - 1] };
            let right = if i == n_per - 1 { frame_yin[i] } else { frame_yin[i + 1] };
            let mut is = frame_yin[i] < left && frame_yin[i] <= right;
            if i == 0 {
                is = n_per > 1 && frame_yin[0] < frame_yin[1];
            }
            if is {
                troughs.push(i);
            }
        }
        if troughs.is_empty() {
            continue;
        }
        let heights: Vec<f64> = troughs.iter().map(|&i| frame_yin[i]).collect();
        let mut probs = vec![0.0; troughs.len()];
        for j in 0..N_THRESH {
            let th = thresholds[j + 1];
            let n = heights.iter().filter(|&&h| h < th).count();
            if n == 0 {
                continue;
            }
            let mut pos = 0.0;
            for (a, &h) in heights.iter().enumerate() {
                if h < th {
                    probs[a] += boltzmann(pos, n as f64) * beta_probs[j];
                    pos += 1.0;
                }
            }
        }
        let gm = heights.iter().enumerate().fold(0, |b, (i, &h)| if h < heights[b] { i } else { b });
        let n_below = (0..N_THRESH).filter(|&j| heights[gm] >= thresholds[j + 1]).count();
        probs[gm] += NO_TROUGH_PROB * beta_probs[..n_below].iter().sum::<f64>();

        for (a, &p) in probs.iter().enumerate() {
            if p == 0.0 {
                continue;
            }
            let idx = troughs[a];
            let mut shift = 0.0;
            if idx > 0 && idx < n_per - 1 {
                let (c, pl, mi) = (frame_yin[idx], frame_yin[idx + 1], frame_yin[idx - 1]);
                let (aa, bb) = (pl + mi - 2.0 * c, 0.5 * (pl - mi));
                if bb.abs() < aa.abs() {
                    shift = -bb / aa;
                }
            }
            let period = min_period as f64 + idx as f64 + shift;
            let bin = (12.0 * nbs as f64 * (sr / period / fmin).log2()).round_ties_even().clamp(0.0, npb as f64) as usize;
            obs[bin * n_frames + t] = p;
        }
        let vp: f64 = (0..npb).map(|b| obs[b * n_frames + t]).sum::<f64>().clamp(0.0, 1.0);
        voiced_prob[t] = vp;
        for b in npb..2 * npb {
            obs[b * n_frames + t] = (1.0 - vp) / npb as f64;
        }
    }
    // frames with no trough: unvoiced uniform
    for t in 0..n_frames {
        if voiced_prob[t] == 0.0 {
            for b in npb..2 * npb {
                obs[b * n_frames + t] = 1.0 / npb as f64;
            }
        }
    }

    // Viterbi (banded transition matrix)
    let max_semi = (MAX_TRANSITION_RATE * 12.0 * hop as f64 / sr).round_ties_even() as usize;
    let width = max_semi * nbs + 1;
    let half = width / 2; // reach of the triangle window on each side
    let tri = |d: isize| -> f64 { (half as f64 + 1.0 - d.unsigned_abs() as f64) / (half as f64 + 1.0) };
    let mut logw = vec![vec![f64::NEG_INFINITY; 2 * half + 1]; npb];
    for (p, row) in logw.iter_mut().enumerate() {
        let lo = p.saturating_sub(half);
        let hi = (p + half).min(npb - 1);
        let sum: f64 = (lo..=hi).map(|q| tri(q as isize - p as isize)).sum();
        for q in lo..=hi {
            let d = q as isize - p as isize;
            row[(d + half as isize) as usize] = (tri(d) / sum + TINY).ln();
        }
    }
    let l_stay = (1.0 - SWITCH_PROB).ln();
    let l_sw = SWITCH_PROB.ln();
    let ns = 2 * npb;
    let l_init = (1.0 / ns as f64 + TINY).ln();
    let lobs = |s: usize, t: usize| (obs[s * n_frames + t] + TINY).ln();
    let mut value: Vec<f64> = (0..ns).map(|s| lobs(s, 0) + l_init).collect();
    let mut ptr = vec![0u16; n_frames * ns];
    let mut next = vec![0.0; ns];
    for t in 1..n_frames {
        for j in 0..ns {
            let (hj, q) = (j / npb, j % npb);
            let lo = q.saturating_sub(half);
            let hi = (q + half).min(npb - 1);
            let mut best = f64::NEG_INFINITY;
            let mut arg = 0usize;
            for h in 0..2 {
                let lsw = if h == hj { l_stay } else { l_sw };
                for p in lo..=hi {
                    let lw = logw[p][(q as isize - p as isize + half as isize) as usize];
                    let c = value[h * npb + p] + lsw + lw;
                    if c > best {
                        best = c;
                        arg = h * npb + p;
                    }
                }
            }
            ptr[t * ns + j] = arg as u16;
            next[j] = lobs(j, t) + best;
        }
        std::mem::swap(&mut value, &mut next);
    }
    let mut state = value.iter().enumerate().fold(0, |b, (i, &v)| if v > value[b] { i } else { b });
    let mut states = vec![0usize; n_frames];
    for t in (0..n_frames).rev() {
        states[t] = state;
        if t > 0 {
            state = ptr[t * ns + state] as usize;
        }
    }
    let f0 = states.iter().map(|&s| if s < npb { fmin * 2f64.powf((s % npb) as f64 / (12.0 * nbs as f64)) } else { f64::NAN }).collect();
    let voiced = states.iter().map(|&s| s < npb).collect();
    Pyin { f0, voiced, voiced_prob }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_a_sine() {
        let sr = 16000.0;
        let y: Vec<f32> = (0..16000).map(|i| (2.0 * std::f64::consts::PI * 220.0 * i as f64 / sr).sin() as f32 * 0.5).collect();
        let p = pyin(&y, 55.0, 1100.0, sr, 2048, 256);
        let mid = p.f0.len() / 2;
        assert!(p.voiced[mid]);
        assert!((p.f0[mid] / 220.0 - 1.0).abs() < 0.01, "{}", p.f0[mid]);
    }
}
