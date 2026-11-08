//! Oversampling 4x con due stadi 2x a filtri half-band polifase (task D4).
//! Usato dal drive attorno al waveshaper per ridurre l'aliasing.
//!
//! Ogni stadio è un FIR half-band (finestra di Kaiser): metà dei coefficienti
//! è zero e il centrale vale 0.5, quindi una fase polifase è un puro ritardo.
//! Latenza totale a 48 kHz: 27 + 4.5 = 31.5 campioni (circa 0.55 ms).

use std::f64::consts::PI;

/// Campioni elaborati per volta (i buffer interni sono di questa dimensione).
const CHUNK: usize = 256;
/// Lunghezza dei FIR: 4m+3. Il primo stadio lavora a 96 kHz con una banda di
/// transizione stretta (20-28 kHz), il secondo a 192 kHz e può essere corto.
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

/// Coefficienti pari h[2i] (senza il centrale) di un half-band con guadagno 1 in continua.
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
    // I coefficienti non nulli sono le posizioni pari (il centrale è dispari).
    (0..=(n - 1) / 2).map(|i| h[2 * i] as f32).collect()
}

/// Memoria circolare a finestra contigua: `s()[i]` è il campione di i passi fa.
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

/// Stadio 2x in salita (interpolatore) e in discesa (decimatore).
struct HalfBand {
    h: Vec<f32>,
    /// ritardo del ramo centrale
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

    /// `input.len()` campioni -> `2 * input.len()` campioni.
    fn upsample(&mut self, input: &[f32], out: &mut [f32]) {
        let d = (self.center - 1) / 2;
        for (i, &x) in input.iter().enumerate() {
            self.up.push(x);
            let s = self.up.s();
            out[2 * i] = 2.0 * dot(&self.h, s);
            out[2 * i + 1] = s[d];
        }
    }

    /// `2 * out.len()` campioni -> `out.len()` campioni.
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

/// Oversampler 4x: `process` applica una funzione non lineare a frequenza quadrupla.
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

    /// Latenza introdotta, in campioni alla frequenza base.
    pub fn latency() -> f32 {
        (TAPS1 - 1) as f32 / 2.0 + (TAPS2 - 1) as f32 / 4.0
    }

    /// Elabora `buf` in-place (qualsiasi lunghezza, senza allocare).
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

    /// Modulo (dB) della risposta del FIR half-band completo alla pulsazione normalizzata `w`.
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
        // stadio 1 (fs = 96 kHz): banda passante fino a 20 kHz, bloccata da 28 kHz
        assert!(mag_db(TAPS1, BETA1, 20_000.0 / 96_000.0).abs() < 0.1);
        let worst = (0..200).map(|i| mag_db(TAPS1, BETA1, (28_000.0 + i as f64 * 100.0) / 96_000.0)).fold(-200.0, f64::max);
        assert!(worst < -60.0, "stadio 1: {worst} dB");
        // stadio 2 (fs = 192 kHz): bloccata da 76 kHz
        let worst2 = (0..200).map(|i| mag_db(TAPS2, BETA2, (76_000.0 + i as f64 * 100.0) / 192_000.0)).fold(-200.0, f64::max);
        assert!(worst2 < -60.0, "stadio 2: {worst2} dB");
        assert!(mag_db(TAPS2, BETA2, 20_000.0 / 192_000.0).abs() < 0.1);
    }

    #[test]
    fn identity_and_latency() {
        let mut os = Oversampler4::new();
        let n = 2000;
        let mut buf: Vec<f32> = (0..n).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin()).collect();
        let orig = buf.clone();
        os.process(&mut buf, |x| x);
        let lat = Oversampler4::latency() as usize + 1; // 26.5 -> ricerca locale
        let mut best = f32::MAX;
        for d in (lat - 3)..(lat + 3) {
            let e: f32 = (200..n).map(|i| (buf[i] - orig[i - d]).abs()).fold(0.0, f32::max);
            best = best.min(e);
        }
        // latenza frazionaria: confronto in ampiezza
        let amp = buf[500..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((amp - 1.0).abs() < 0.02, "ampiezza {amp}");
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
