//! Synthetic clean guitar DI from transcribed notes (port of synth.py): Karplus-Strong, pickup
//! filter, spectral match (to a real DI or the default tilt), fixed RMS.
use crate::dsp::{firwin2, interp, welch, Biquad, Fft};
use crate::job::Note;

pub const TARGET_RMS_DB: f64 = -29.8;

/// Measured on a real passive-pickup DI: dB relative to the max 1/3-octave band.
const DEFAULT_TILT: [(f64, f64); 9] =
    [(100.0, -4.0), (250.0, 0.0), (500.0, -3.0), (1000.0, -13.0), (1600.0, -16.0), (2500.0, -17.0), (4000.0, -28.0), (6300.0, -35.0), (10000.0, -48.0)];

struct Rng(u64);
impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545F4914F6CDD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

fn pluck(freq: f64, n: usize, sr: f64, rng: &mut Rng) -> Vec<f64> {
    let period = sr / freq;
    let damp = if freq < 250.0 { 0.996 } else { 0.993 };
    let d_total = period - 0.5;
    let d = (d_total.floor() as usize).max(2);
    let f = d_total - d as f64;
    let burst: Vec<f64> = (0..d + 2).map(|_| rng.uniform()).collect();
    let comb = ((0.18 * period).round() as usize).max(1);
    let mut x = vec![0.0; n];
    for i in 0..burst.len().min(n) {
        x[i] = burst[i] - if i >= comb { burst[i - comb] } else { 0.0 };
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut v = x[i];
        if i >= d {
            v += damp * (1.0 - f) * y[i - d];
        }
        if i > d {
            v += damp * f * y[i - d - 1];
        }
        y[i] = v;
    }
    let peak = y.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if peak > 0.0 {
        y.iter_mut().for_each(|v| *v /= peak);
    }
    y
}

fn pickup(x: &[f64], sr: f64) -> Vec<f64> {
    let peak = Biquad::peak(4500.0, 1.5, sr).process(x);
    let y: Vec<f64> = x.iter().zip(&peak).map(|(a, p)| a + 1.5 * p).collect();
    let y = Biquad::butter2(6000.0, sr, false).process(&y);
    Biquad::butter2(70.0, sr, true).process(&y)
}

fn third_octave_centres() -> Vec<f64> {
    let n0 = (3.0 * (40.0f64 / 1000.0).log2()).floor() as i32;
    let n1 = (3.0 * (10000.0f64 / 1000.0).log2()).ceil() as i32;
    (n0..=n1).map(|n| 1000.0 * 2f64.powf(n as f64 / 3.0)).collect()
}

fn band_db(fft: &mut Fft, x: &[f64], sr: f64, centres: &[f64]) -> Vec<f64> {
    let (f, p) = welch(fft, x, sr, 8192);
    let w = 2f64.powf(1.0 / 6.0);
    centres
        .iter()
        .map(|&c| {
            let mut s = 0.0;
            let mut any = false;
            for (fi, pi) in f.iter().zip(&p) {
                if *fi >= c / w && *fi < c * w {
                    s += pi;
                    any = true;
                }
            }
            if any {
                10.0 * (s + 1e-30).log10()
            } else {
                -200.0
            }
        })
        .collect()
}

fn fir_from_gain(fft: &mut Fft, freqs: &[f64], gains_db: &[f64], sr: f64) -> Vec<f64> {
    let mut f = vec![0.0];
    f.extend_from_slice(freqs);
    f.push(sr / 2.0);
    let mut g = vec![gains_db[0]];
    g.extend_from_slice(gains_db);
    g.push(gains_db[gains_db.len() - 1]);
    let lin: Vec<f64> = g.iter().map(|v| 10f64.powf(v.clamp(-40.0, 12.0) / 20.0)).collect();
    firwin2(fft, 2047, &f, &lin, sr)
}

fn apply_fir(fft: &mut Fft, x: &[f64], h: &[f64]) -> Vec<f64> {
    let y = fft.convolve(x, h);
    y[h.len() / 2..h.len() / 2 + x.len()].to_vec()
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

fn spectral_match(fft: &mut Fft, x: &[f64], reference: &[f64], sr: f64) -> Vec<f64> {
    let (f, px) = welch(fft, x, sr, 8192);
    let (_, pr) = welch(fft, reference, sr, 8192);
    let ratio: Vec<f64> = (1..f.len()).map(|i| 10.0 * ((pr[i] + 1e-30) / (px[i] + 1e-30)).log10()).collect();
    let lf: Vec<f64> = f[1..].iter().map(|v| v.max(1.0).log2()).collect();
    let (lo, hi) = (lf[0], lf[lf.len() - 1]);
    let grid: Vec<f64> = (0..1500).map(|i| lo + (hi - lo) * i as f64 / 1499.0).collect();
    let r: Vec<f64> = grid.iter().map(|&g| interp(g, &lf, &ratio)).collect();
    let k = ((1500.0 / (hi - lo) / 3.0).round() as usize).max(1);
    let mut padded = vec![r[0]; k];
    padded.extend_from_slice(&r);
    padded.extend(std::iter::repeat_n(r[r.len() - 1], k));
    let w = 2 * k + 1;
    let mut sm: Vec<f64> = (0..r.len()).map(|i| padded[i..i + w].iter().sum::<f64>() / w as f64).collect();
    let fg: Vec<f64> = grid.iter().map(|g| 2f64.powf(*g)).collect();
    let sel: Vec<f64> = sm.iter().zip(&fg).filter(|(_, f)| **f >= 100.0 && **f <= 1000.0).map(|(v, _)| *v).collect();
    let med = median(&sel);
    sm.iter_mut().for_each(|v| *v -= med);
    let h = fir_from_gain(fft, &fg, &sm, sr);
    apply_fir(fft, x, &h)
}

fn default_tilt(fft: &mut Fft, x: &[f64], sr: f64) -> Vec<f64> {
    let fr: Vec<f64> = DEFAULT_TILT.iter().map(|t| t.0).collect();
    let db: Vec<f64> = DEFAULT_TILT.iter().map(|t| t.1).collect();
    let cen = third_octave_centres();
    let lfr: Vec<f64> = fr.iter().map(|v| v.ln()).collect();
    let target: Vec<f64> = cen
        .iter()
        .map(|&c| {
            if c < fr[0] {
                db[0] + (db[1] - db[0]) / (fr[1] / fr[0]).ln() * (c / fr[0]).ln()
            } else if c > fr[8] {
                db[8] + (db[8] - db[7]) / (fr[8] / fr[7]).ln() * (c / fr[8]).ln()
            } else {
                interp(c.ln(), &lfr, &db)
            }
        })
        .collect();
    let cur = band_db(fft, x, sr, &cen);
    let mut gain: Vec<f64> = target.iter().zip(&cur).map(|(t, c)| t - c).collect();
    let sel: Vec<f64> = gain.iter().zip(&cen).filter(|(_, c)| **c >= 100.0 && **c <= 1000.0).map(|(g, _)| *g).collect();
    let med = median(&sel);
    gain.iter_mut().for_each(|g| *g -= med);
    let h = fir_from_gain(fft, &cen, &gain, sr);
    apply_fir(fft, x, &h)
}

/// Clean guitar DI (mono) from `notes`; matched to `match_di` (a real DI at `sr`) or the default tilt.
pub fn synth_di(notes: &[Note], sr: u32, match_di: Option<&[f32]>) -> Vec<f32> {
    let srf = sr as f64;
    if notes.is_empty() {
        return vec![0.0; sr as usize / 10];
    }
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let total = notes.iter().map(|n| n.t + n.dur).fold(0.0, f64::max);
    let mut out = vec![0.0f64; (total * srf) as usize + sr as usize / 4];
    let max_amp = notes.iter().map(|n| n.amp).fold(0.0, f64::max);
    let max_amp = if max_amp == 0.0 { 1.0 } else { max_amp };
    let rel = (0.012 * srf) as usize;
    for n in notes {
        let freq = 440.0 * 2f64.powf((n.midi as f64 - 69.0) / 12.0);
        let length = ((n.dur * srf) as usize).max(rel * 2);
        let mut y = pluck(freq, length, srf, &mut rng);
        for i in 0..rel {
            y[length - rel + i] *= 1.0 - i as f64 / (rel - 1) as f64;
        }
        let gain = 0.4 + 0.6 * (n.amp / max_amp).sqrt();
        let s = (n.t * srf) as usize;
        let e = (s + length).min(out.len());
        for i in s..e {
            out[i] += gain * y[i - s];
        }
    }
    let mut fft = Fft::new();
    let mut out = pickup(&out, srf);
    out = match match_di {
        Some(r) => {
            let r: Vec<f64> = r.iter().map(|&v| v as f64).collect();
            spectral_match(&mut fft, &out, &r, srf)
        }
        None => default_tilt(&mut fft, &out, srf),
    };
    let target = 10f64.powf(TARGET_RMS_DB / 20.0);
    let rms = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / v.len().max(1) as f64).sqrt();
    let active: Vec<f64> = out.iter().cloned().filter(|v| v.abs() > 1e-12).collect();
    let mut scale = target / (rms(&active) + 1e-30);
    for _ in 0..8 {
        let act: Vec<f64> = out.iter().map(|v| v * scale).filter(|v| v.abs() > 0.003).collect();
        if act.is_empty() {
            break;
        }
        let r = rms(&act);
        scale *= target / r;
        if (20.0 * (r / target).log10()).abs() < 0.01 {
            break;
        }
    }
    out.iter().map(|v| (v * scale) as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn di_has_target_level_and_pitch() {
        let notes = vec![
            Note { t: 0.0, dur: 0.5, midi: 52, amp: 0.2, octave_shifted: false },
            Note { t: 0.5, dur: 0.5, midi: 59, amp: 0.1, octave_shifted: false },
        ];
        let y = synth_di(&notes, 48000, None);
        assert!(y.len() >= 48000);
        let act: Vec<f64> = y.iter().map(|&v| v as f64).filter(|v| v.abs() > 0.003).collect();
        let r = (act.iter().map(|v| v * v).sum::<f64>() / act.len() as f64).sqrt();
        assert!((20.0 * r.log10() - TARGET_RMS_DB).abs() < 0.5);
        // 52 = E3 = 164.8 Hz: the first 0.5 s correlates best at its period
        let x: Vec<f64> = y[2000..20000].iter().map(|&v| v as f64).collect();
        let lag = (48000.0f64 / 164.81).round() as usize;
        let ac = |l: usize| (0..x.len() - l).map(|i| x[i] * x[i + l]).sum::<f64>();
        assert!(ac(lag) > 0.5 * ac(0) && ac(lag) > ac(lag / 2));
    }
}
