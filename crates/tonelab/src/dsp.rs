//! Signal-processing helpers (ports of the librosa/scipy pieces the Python tool uses).
use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

pub struct Fft {
    planner: FftPlanner<f64>,
}

impl Default for Fft {
    fn default() -> Self {
        Self { planner: FftPlanner::new() }
    }
}

impl Fft {
    pub fn new() -> Self {
        Self::default()
    }
    /// Forward FFT of real input zero-padded to `n`; returns `n/2+1` bins.
    pub fn rfft(&mut self, x: &[f64], n: usize) -> Vec<Complex<f64>> {
        let f = self.planner.plan_fft_forward(n);
        let mut b: Vec<Complex<f64>> = (0..n).map(|i| Complex::new(x.get(i).copied().unwrap_or(0.0), 0.0)).collect();
        f.process(&mut b);
        b.truncate(n / 2 + 1);
        b
    }
    /// Inverse of `rfft` (real output of length `n`, normalised).
    pub fn irfft(&mut self, spec: &[Complex<f64>], n: usize) -> Vec<f64> {
        let f = self.planner.plan_fft_inverse(n);
        let mut b = vec![Complex::new(0.0, 0.0); n];
        for k in 0..n {
            b[k] = if k < spec.len() && k <= n / 2 {
                spec[k]
            } else {
                let m = n - k;
                if m < spec.len() {
                    spec[m].conj()
                } else {
                    Complex::new(0.0, 0.0)
                }
            };
        }
        b[0].im = 0.0;
        if n % 2 == 0 {
            b[n / 2].im = 0.0;
        }
        f.process(&mut b);
        b.iter().map(|c| c.re / n as f64).collect()
    }
    /// Linear convolution, same length as `x + h - 1`.
    pub fn convolve(&mut self, x: &[f64], h: &[f64]) -> Vec<f64> {
        let n = x.len() + h.len() - 1;
        let size = n.next_power_of_two();
        let a = self.rfft(x, size);
        let b = self.rfft(h, size);
        let p: Vec<_> = a.iter().zip(&b).map(|(a, b)| a * b).collect();
        let mut y = self.irfft(&p, size);
        y.truncate(n);
        y
    }
}

pub fn hann_periodic(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()).collect()
}

/// Power STFT (|X|^2), librosa defaults: centred, zero padded, periodic Hann. Frames x bins.
pub fn stft_power(fft: &mut Fft, y: &[f32], n_fft: usize, hop: usize) -> Vec<Vec<f64>> {
    let win = hann_periodic(n_fft);
    let pad = n_fft / 2;
    let n_frames = 1 + y.len() / hop;
    let plan = fft.planner.plan_fft_forward(n_fft);
    let mut buf = vec![Complex::new(0.0, 0.0); n_fft];
    let mut out = Vec::with_capacity(n_frames);
    for t in 0..n_frames {
        for (i, b) in buf.iter_mut().enumerate() {
            let idx = (t * hop + i) as isize - pad as isize;
            let v = if idx >= 0 && (idx as usize) < y.len() { y[idx as usize] as f64 } else { 0.0 };
            *b = Complex::new(v * win[i], 0.0);
        }
        plan.process(&mut buf);
        out.push(buf[..n_fft / 2 + 1].iter().map(|c| c.norm_sqr()).collect());
    }
    out
}

fn hz_to_mel(f: f64) -> f64 {
    if f >= 1000.0 {
        15.0 + (f / 1000.0).ln() / (6.4f64.ln() / 27.0)
    } else {
        f / (200.0 / 3.0)
    }
}
fn mel_to_hz(m: f64) -> f64 {
    if m >= 15.0 {
        1000.0 * ((6.4f64.ln() / 27.0) * (m - 15.0)).exp()
    } else {
        m * 200.0 / 3.0
    }
}

/// Slaney mel filterbank (n_mels x (n_fft/2+1)).
pub fn mel_filterbank(sr: f64, n_fft: usize, n_mels: usize) -> Vec<Vec<f64>> {
    let nb = n_fft / 2 + 1;
    let fftf: Vec<f64> = (0..nb).map(|i| i as f64 * sr / n_fft as f64).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mf: Vec<f64> = (0..n_mels + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64)).collect();
    let mut w = vec![vec![0.0; nb]; n_mels];
    for i in 0..n_mels {
        let fdiff = [mf[i + 1] - mf[i], mf[i + 2] - mf[i + 1]];
        let enorm = 2.0 / (mf[i + 2] - mf[i]);
        for (k, f) in fftf.iter().enumerate() {
            let lower = (f - mf[i]) / fdiff[0];
            let upper = (mf[i + 2] - f) / fdiff[1];
            w[i][k] = lower.min(upper).max(0.0) * enorm;
        }
    }
    w
}

/// librosa `onset_strength` (mel, power_to_db top 80, lag 1, centred). `median` selects the
/// aggregate across mel bands (mean by default, median for beat_track).
pub fn onset_strength(fft: &mut Fft, y: &[f32], sr: f64, hop: usize, median: bool) -> Vec<f64> {
    let n_fft = 2048;
    let s = stft_power(fft, y, n_fft, hop);
    let fb = mel_filterbank(sr, n_fft, 128);
    let nf = s.len();
    let mut db = vec![vec![0.0f64; nf]; 128];
    let mut mx = f64::NEG_INFINITY;
    for (t, fr) in s.iter().enumerate() {
        for (m, row) in fb.iter().enumerate() {
            let mut a = 0.0;
            for (k, w) in row.iter().enumerate() {
                if *w != 0.0 {
                    a += w * fr[k];
                }
            }
            let v = 10.0 * a.max(1e-10).log10();
            db[m][t] = v;
            mx = mx.max(v);
        }
    }
    for r in db.iter_mut() {
        for v in r.iter_mut() {
            *v = v.max(mx - 80.0);
        }
    }
    let pad = 1 + n_fft / (2 * hop);
    let mut env = vec![0.0; pad];
    let mut col = vec![0.0; 128];
    for t in 1..nf {
        for m in 0..128 {
            col[m] = (db[m][t] - db[m][t - 1]).max(0.0);
        }
        env.push(if median {
            let mut c = col.clone();
            c.sort_by(|a, b| a.partial_cmp(b).unwrap());
            (c[63] + c[64]) / 2.0
        } else {
            col.iter().sum::<f64>() / 128.0
        });
    }
    env.truncate(nf);
    env
}

/// librosa `util.peak_pick` (greedy).
pub fn peak_pick(x: &[f64], pre_max: usize, post_max: usize, pre_avg: usize, post_avg: usize, delta: f64, wait: usize) -> Vec<usize> {
    let n = x.len();
    let mut peaks = vec![];
    if n == 0 {
        return peaks;
    }
    let max_of = |a: usize, b: usize| x[a..b].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mean_of = |a: usize, b: usize| x[a..b].iter().sum::<f64>() / (b - a) as f64;
    let mut i;
    if x[0] >= max_of(0, post_max.min(n)) && x[0] >= mean_of(0, post_avg.min(n)) + delta {
        peaks.push(0);
        i = wait + 1;
    } else {
        i = 1;
    }
    while i < n {
        let m = max_of(i.saturating_sub(pre_max), (i + post_max).min(n));
        if x[i] != m {
            i += 1;
            continue;
        }
        let a = mean_of(i.saturating_sub(pre_avg), (i + post_avg).min(n));
        if x[i] < a + delta {
            i += 1;
            continue;
        }
        peaks.push(i);
        i += wait + 1;
    }
    peaks
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > sum * 1e-17 {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// scipy `resample_poly` (Kaiser beta 5 FIR), direct polyphase evaluation.
pub fn resample_poly(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return x.to_vec();
    }
    let g = gcd(from as usize, to as usize);
    let (up, down) = (to as usize / g, from as usize / g);
    let half = 10 * up.max(down);
    let taps = 2 * half + 1;
    let cutoff = 1.0 / up.max(down) as f64;
    let beta = 5.0;
    let mut h: Vec<f64> = (0..taps)
        .map(|j| {
            let m = j as f64 - half as f64;
            let a = PI * cutoff * m;
            let sinc = if m == 0.0 { 1.0 } else { a.sin() / a };
            let r = 2.0 * j as f64 / (taps - 1) as f64 - 1.0;
            cutoff * sinc * bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta)
        })
        .collect();
    let s: f64 = h.iter().sum();
    for v in h.iter_mut() {
        *v *= up as f64 / s;
    }
    let n_out = (x.len() * up).div_ceil(down);
    let mut out = vec![0.0f32; n_out];
    for (m, o) in out.iter_mut().enumerate() {
        let n = (m * down + half) as isize;
        // j = n - k*up in [0, taps-1]  =>  k in [(n-taps+1)/up, n/up]
        let k_hi = (n / up as isize).min(x.len() as isize - 1);
        let k_lo = ((n - taps as isize + 1 + up as isize - 1).div_euclid(up as isize)).max(0);
        let mut a = 0.0f64;
        let mut k = k_lo;
        while k <= k_hi {
            a += x[k as usize] as f64 * h[(n - k * up as isize) as usize];
            k += 1;
        }
        *o = a as f32;
    }
    out
}

/// scipy `signal.welch(x, fs, nperseg)`: Hann, 50 % overlap, constant detrend, density.
pub fn welch(fft: &mut Fft, x: &[f64], fs: f64, nperseg: usize) -> (Vec<f64>, Vec<f64>) {
    let nper = nperseg.min(x.len()).max(2);
    let win = hann_periodic(nper);
    let step = nper / 2;
    let s2: f64 = win.iter().map(|w| w * w).sum();
    let nb = nper / 2 + 1;
    let mut acc = vec![0.0; nb];
    let mut cnt = 0;
    let mut i = 0;
    while i + nper <= x.len() {
        let seg = &x[i..i + nper];
        let mean = seg.iter().sum::<f64>() / nper as f64;
        let d: Vec<f64> = seg.iter().zip(&win).map(|(v, w)| (v - mean) * w).collect();
        for (a, c) in acc.iter_mut().zip(fft.rfft(&d, nper)) {
            *a += c.norm_sqr();
        }
        cnt += 1;
        i += step;
    }
    let scale = 1.0 / (fs * s2) / cnt.max(1) as f64;
    let mut p: Vec<f64> = acc.iter().map(|a| a * scale).collect();
    for (k, v) in p.iter_mut().enumerate() {
        if k != 0 && !(nper % 2 == 0 && k == nb - 1) {
            *v *= 2.0;
        }
    }
    let f = (0..nb).map(|k| k as f64 * fs / nper as f64).collect();
    (f, p)
}

/// numpy `interp` (clamped at both ends).
pub fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    if x <= xp[0] {
        return fp[0];
    }
    if x >= xp[xp.len() - 1] {
        return fp[fp.len() - 1];
    }
    let i = xp.partition_point(|&v| v <= x) - 1;
    let (x0, x1) = (xp[i], xp[i + 1]);
    if x1 == x0 {
        return fp[i + 1];
    }
    fp[i] + (fp[i + 1] - fp[i]) * (x - x0) / (x1 - x0)
}

/// scipy `firwin2(ntaps, freq, gain, fs=fs)` with the default Hamming window.
pub fn firwin2(fft: &mut Fft, ntaps: usize, freq: &[f64], gain: &[f64], fs: f64) -> Vec<f64> {
    let nyq = fs / 2.0;
    let nfreqs = 1 + ntaps.next_power_of_two().max(2);
    let nfreqs = if ntaps.is_power_of_two() { 1 + ntaps } else { nfreqs };
    let xs: Vec<f64> = (0..nfreqs).map(|i| nyq * i as f64 / (nfreqs - 1) as f64).collect();
    let spec: Vec<Complex<f64>> = xs
        .iter()
        .map(|&x| {
            let g = interp(x, freq, gain);
            let ph = -(ntaps as f64 - 1.0) / 2.0 * PI * x / nyq;
            Complex::new(g * ph.cos(), g * ph.sin())
        })
        .collect();
    let full = fft.irfft(&spec, 2 * (nfreqs - 1));
    (0..ntaps).map(|i| full[i] * (0.54 - 0.46 * (2.0 * PI * i as f64 / (ntaps as f64 - 1.0)).cos())).collect()
}

/// Direct-form biquad coefficients (b0,b1,b2,a1,a2 normalised).
#[derive(Clone, Copy)]
pub struct Biquad {
    pub b: [f64; 3],
    pub a: [f64; 2],
}

impl Biquad {
    /// 2nd-order Butterworth, same as `signal.butter(2, fc, btype, fs)`.
    pub fn butter2(fc: f64, fs: f64, high: bool) -> Self {
        let k = (PI * fc / fs).tan();
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let n = 1.0 / (1.0 + k / q + k * k);
        let (b0, b1, b2) = if high { (n, -2.0 * n, n) } else { (k * k * n, 2.0 * k * k * n, k * k * n) };
        Self { b: [b0, b1, b2], a: [2.0 * (k * k - 1.0) * n, (1.0 - k / q + k * k) * n] }
    }
    /// `signal.iirpeak(w0, Q)` with `w0` in Hz.
    pub fn peak(f0: f64, q: f64, fs: f64) -> Self {
        let w0 = f0 / (fs / 2.0) * PI;
        let bw = w0 / q;
        let beta = (bw / 2.0).tan();
        let gain = 1.0 / (1.0 + beta);
        let (b0, b1, b2) = ((1.0 - gain), 0.0, -(1.0 - gain));
        Self { b: [b0, b1, b2], a: [-2.0 * gain * w0.cos(), 2.0 * gain - 1.0] }
    }
    pub fn process(&self, x: &[f64]) -> Vec<f64> {
        let (mut z1, mut z2) = (0.0, 0.0);
        x.iter()
            .map(|&v| {
                let y = self.b[0] * v + z1;
                z1 = self.b[1] * v - self.a[0] * y + z2;
                z2 = self.b[2] * v - self.a[1] * y;
                y
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_sine() {
        let x: Vec<f32> = (0..48000).map(|i| (2.0 * PI * 440.0 * i as f64 / 48000.0).sin() as f32).collect();
        let y = resample_poly(&x, 48000, 22050);
        assert_eq!(y.len(), 22050);
        let err = (5000..15000).map(|i| (y[i] as f64 - (2.0 * PI * 440.0 * i as f64 / 22050.0).sin()).abs()).fold(0.0, f64::max);
        assert!(err < 2e-3, "{err}");
    }

    #[test]
    fn firwin2_flat() {
        let mut f = Fft::new();
        let h = firwin2(&mut f, 2047, &[0.0, 24000.0], &[1.0, 1.0], 48000.0);
        assert!((h[1023] - 1.0).abs() < 1e-2);
    }
}
