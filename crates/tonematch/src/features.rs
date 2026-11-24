//! Spectral features: the 1/3-octave long-term average spectrum and the crest factor.

use std::f32::consts::PI;

use rustfft::{num_complex::Complex, FftPlanner};

/// Number of bands: `1000 * 2^(k/3)` for k = -14..=10 (about 40 Hz to 10 kHz).
pub const BANDS: usize = 25;
const FRAME: usize = 8192;
const HOP: usize = 4096;

/// Centre frequencies of the bands.
pub fn band_centres() -> [f32; BANDS] {
    std::array::from_fn(|i| 1000.0 * 2f32.powf((i as f32 - 14.0) / 3.0))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    /// Band levels in dB, normalised so the loudest band is 0.
    pub bands_db: [f32; BANDS],
    /// Peak over RMS of the samples with |x| > 1e-3, in dB.
    pub crest_db: f32,
}

/// Crest factor in dB (peak / RMS over samples with |x| > 1e-3).
pub fn crest_db(x: &[f32]) -> f32 {
    let (mut peak, mut sum, mut n) = (0f64, 0f64, 0usize);
    for &s in x {
        let a = s.abs() as f64;
        if a > 1e-3 {
            peak = peak.max(a);
            sum += a * a;
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    let rms = (sum / n as f64).sqrt();
    (20.0 * (peak / rms).log10()) as f32
}

/// Reusable FFT state (the plan and scratch buffers) for repeated analyses.
pub struct Analyzer {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    sample_rate: f32,
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FRAME);
        let window = (0..FRAME).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FRAME as f32).cos()).collect();
        let scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        Self { fft, window, buf: vec![Complex::new(0.0, 0.0); FRAME], scratch, sample_rate }
    }

    pub fn analyze(&mut self, x: &[f32]) -> Features {
        let bins = FRAME / 2 + 1;
        let mut power = vec![0f64; bins];
        let mut frames = 0usize;
        let mut start = 0;
        loop {
            for i in 0..FRAME {
                let s = x.get(start + i).copied().unwrap_or(0.0);
                self.buf[i] = Complex::new(s * self.window[i], 0.0);
            }
            self.fft.process_with_scratch(&mut self.buf, &mut self.scratch);
            for (p, c) in power.iter_mut().zip(&self.buf) {
                *p += c.norm_sqr() as f64;
            }
            frames += 1;
            start += HOP;
            if start + FRAME > x.len() {
                break;
            }
        }
        let hz_per_bin = self.sample_rate / FRAME as f32;
        let edge = 2f32.powf(1.0 / 6.0);
        let mut bands = [0f32; BANDS];
        for (b, f) in band_centres().iter().enumerate() {
            let (lo, hi) = (f / edge, f * edge);
            let sum: f64 = power
                .iter()
                .enumerate()
                .filter(|(k, _)| {
                    let hz = *k as f32 * hz_per_bin;
                    hz >= lo && hz < hi
                })
                .map(|(_, p)| *p)
                .sum();
            bands[b] = (10.0 * (sum / frames as f64 + 1e-20).log10()) as f32;
        }
        let max = bands.iter().cloned().fold(f32::MIN, f32::max);
        for v in &mut bands {
            *v -= max;
        }
        Features { bands_db: bands, crest_db: crest_db(x) }
    }
}

/// One-shot analysis.
pub fn analyze(x: &[f32], sample_rate: f32) -> Features {
    Analyzer::new(sample_rate).analyze(x)
}

/// Band differences `render - target` in dB.
pub fn diff_db(render: &Features, target: &Features) -> [f32; BANDS] {
    std::array::from_fn(|i| render.bands_db[i] - target.bands_db[i])
}

/// RMS of the band differences: the spectral error in dB.
pub fn spectral_error(render: &Features, target: &Features) -> f32 {
    let d = diff_db(render, target);
    (d.iter().map(|v| v * v).sum::<f32>() / BANDS as f32).sqrt()
}

/// Cost = spectral error + 0.3 x crest difference.
pub fn cost(render: &Features, target: &Features) -> f32 {
    spectral_error(render, target) + 0.3 * (render.crest_db - target.crest_db).abs()
}
