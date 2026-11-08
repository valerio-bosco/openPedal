//! 4x oversampling with two 2x stages of polyphase half-band filters (task D4).
//! Used by the drive around the waveshaper to reduce aliasing.
//!
//! Each stage is a half-band FIR (Kaiser window): half of the coefficients
//! are zero and the center one is 0.5, so one polyphase branch is a pure delay.
//! Total latency at 48 kHz: 27 + 4.5 = 31.5 samples (about 0.55 ms).

use std::f64::consts::PI;

/// Samples processed at a time (the internal buffers have this size).
const CHUNK: usize = 256;
/// FIR length: 4m+3. The first stage works at 96 kHz with a narrow
/// transition band (20-28 kHz), the second at 192 kHz and can be short.
const TAPS1: usize = 55;
const TAPS2: usize = 19;
const BETA1: f64 = 6.5;
const BETA2: f64 = 6.5;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..60 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-14 * sum {
            break;
        }
    }
    sum
}

/// Even coefficients h[2i] (without the center one) of a half-band with unity DC gain.
fn halfband_taps(n: usize, beta: f64) -> Vec<f32> {
    let c = (n - 1) / 2;
    let i0b = bessel_i0(beta);
    let h: Vec<f64> = (0..n)
        .map(|k| {
            let d = k as f64 - c as f64;
            let sinc = if d == 0.0 { 0.5 } else { (PI * d / 2.0).sin() / (PI * d) };
            let r = d / c as f64;
            sinc * bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0b
        })
        .collect();
    // The non-zero coefficients are the even positions (the center one is odd).
    (0..=(n - 1) / 2).map(|i| h[2 * i] as f32).collect()
}

/// Circular buffer with a contiguous window: `s()[i]` is the sample from i steps ago.
struct Ring {
    buf: Vec<f32>,
    pos: usize,
    len: usize,
}

impl Ring {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; 2 * len], pos: 0, len }
    }

    #[inline]
    fn push(&mut self, v: f32) {
        self.pos = if self.pos == 0 { self.len - 1 } else { self.pos - 1 };
        self.buf[self.pos] = v;
        self.buf[self.pos + self.len] = v;
    }

    #[inline]
    fn s(&self) -> &[f32] {
        &self.buf[self.pos..self.pos + self.len]
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// 2x stage for upsampling (interpolator) and downsampling (decimator).
struct HalfBand {
    h: Vec<f32>,
    /// delay of the center branch
    center: usize,
    up: Ring,
    dn_even: Ring,
    dn_odd: Ring,
}

impl HalfBand {
    fn new(n: usize, beta: f64) -> Self {
        let h = halfband_taps(n, beta);
        let len = h.len();
        Self { h, center: (n - 1) / 2, up: Ring::new(len), dn_even: Ring::new(len), dn_odd: Ring::new(len) }
    }

    /// `input.len()` samples -> `2 * input.len()` samples.
    fn upsample(&mut self, input: &[f32], out: &mut [f32]) {
        let d = (self.center - 1) / 2;
        for (i, &x) in input.iter().enumerate() {
            self.up.push(x);
            let s = self.up.s();
            out[2 * i] = 2.0 * dot(&self.h, s);
            out[2 * i + 1] = s[d];
        }
    }

    /// `2 * out.len()` samples -> `out.len()` samples.
    fn downsample(&mut self, input: &[f32], out: &mut [f32]) {
        let d = (self.center + 1).div_ceil(2);
        for (i, o) in out.iter_mut().enumerate() {
            self.dn_even.push(input[2 * i]);
            *o = dot(&self.h, self.dn_even.s()) + 0.5 * self.dn_odd.s()[d - 1];
            self.dn_odd.push(input[2 * i + 1]);
        }
    }

    fn reset(&mut self) {
        self.up.clear();
        self.dn_even.clear();
        self.dn_odd.clear();
    }
}

/// 4x oversampler: `process` applies a non-linear function at four times the rate.
pub struct Oversampler4 {
    s1: HalfBand,
    s2: HalfBand,
    mid: Vec<f32>,
    hi: Vec<f32>,
}

impl Oversampler4 {
    pub fn new() -> Self {
        Self {
            s1: HalfBand::new(TAPS1, BETA1),
            s2: HalfBand::new(TAPS2, BETA2),
            mid: vec![0.0; 2 * CHUNK],
            hi: vec![0.0; 4 * CHUNK],
        }
    }

    /// Introduced latency, in samples at the base rate.
    #[allow(dead_code)] // will be needed to compensate the latency between parallel branches of the graph
    pub fn latency() -> f32 {
        (TAPS1 - 1) as f32 / 2.0 + (TAPS2 - 1) as f32 / 4.0
    }

    /// Processes `buf` in-place (any length, without allocating).
    pub fn process(&mut self, buf: &mut [f32], mut f: impl FnMut(f32) -> f32) {
        for chunk in buf.chunks_mut(CHUNK) {
            let n = chunk.len();
            self.s1.upsample(chunk, &mut self.mid[..2 * n]);
            self.s2.upsample(&self.mid[..2 * n], &mut self.hi[..4 * n]);
            for v in &mut self.hi[..4 * n] {
                *v = f(*v);
            }
            self.s2.downsample(&self.hi[..4 * n], &mut self.mid[..2 * n]);
            self.s1.downsample(&self.mid[..2 * n], chunk);
        }
    }

    pub fn reset(&mut self) {
        self.s1.reset();
        self.s2.reset();
    }
}

impl Default for Oversampler4 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magnitude (dB) of the full half-band FIR response at the normalized frequency `f_norm`.
    fn mag_db(n: usize, beta: f64, f_norm: f64) -> f64 {
        let hs = halfband_taps(n, beta);
        let c = (n - 1) / 2;
        let (mut re, mut im) = (0.5 * (2.0 * PI * f_norm * c as f64).cos(), -0.5 * (2.0 * PI * f_norm * c as f64).sin());
        for (i, &h) in hs.iter().enumerate() {
            let k = 2 * i;
            let w = 2.0 * PI * f_norm * k as f64;
            re += h as f64 * w.cos();
            im -= h as f64 * w.sin();
        }
        20.0 * (re * re + im * im).sqrt().max(1e-12).log10()
    }

    #[test]
    fn halfband_stopband() {
        // stage 1 (fs = 96 kHz): passband up to 20 kHz, stopband from 28 kHz
        assert!(mag_db(TAPS1, BETA1, 20_000.0 / 96_000.0).abs() < 0.1);
        let worst = (0..200).map(|i| mag_db(TAPS1, BETA1, (28_000.0 + i as f64 * 100.0) / 96_000.0)).fold(-200.0, f64::max);
        assert!(worst < -60.0, "stage 1: {worst} dB");
        // stage 2 (fs = 192 kHz): stopband from 76 kHz
        let worst2 = (0..200).map(|i| mag_db(TAPS2, BETA2, (76_000.0 + i as f64 * 100.0) / 192_000.0)).fold(-200.0, f64::max);
        assert!(worst2 < -60.0, "stage 2: {worst2} dB");
        assert!(mag_db(TAPS2, BETA2, 20_000.0 / 192_000.0).abs() < 0.1);
    }

    #[test]
    fn identity_and_latency() {
        let mut os = Oversampler4::new();
        let n = 2000;
        let mut buf: Vec<f32> = (0..n).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin()).collect();
        let orig = buf.clone();
        os.process(&mut buf, |x| x);
        let lat = Oversampler4::latency() as usize + 1; // 26.5 -> local search
        let mut best = f32::MAX;
        for d in (lat - 3)..(lat + 3) {
            let e: f32 = (200..n).map(|i| (buf[i] - orig[i - d]).abs()).fold(0.0, f32::max);
            best = best.min(e);
        }
        // fractional latency: compare the amplitude
        let amp = buf[500..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((amp - 1.0).abs() < 0.02, "amplitude {amp}");
        assert!(best < 0.5);
    }

    #[test]
    fn chunking_is_transparent() {
        let x: Vec<f32> = (0..1000).map(|i| ((i * 37 % 101) as f32 / 50.0 - 1.0) * 0.5).collect();
        let mut a = x.clone();
        Oversampler4::new().process(&mut a, |v| v.tanh());
        let mut os = Oversampler4::new();
        let mut b = x.clone();
        for c in b.chunks_mut(77) {
            os.process(c, |v| v.tanh());
        }
        for (p, q) in a.iter().zip(&b) {
            assert!((p - q).abs() < 1e-5);
        }
    }
}
