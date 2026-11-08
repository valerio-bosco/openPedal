use std::f32::consts::PI;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const FILTER_DESC: BlockDesc = BlockDesc {
    kind: "filter",
    label: "Filter / EQ",
    category: Category::Filter,
    params: &[
        ParamDesc::choice(
            "kind",
            "Type",
            &["lowpass", "highpass", "bandpass", "notch", "peak", "lowshelf", "highshelf"],
            0,
        ),
        ParamDesc::log("freq", "Frequency", 20.0, 20000.0, 1000.0, "Hz"),
        ParamDesc::log("q", "Q", 0.1, 20.0, 0.707, ""),
        // Only used by peak, lowshelf and highshelf.
        ParamDesc::number("gain_db", "Gain", -24.0, 24.0, 0.0, "dB"),
    ],
};

// Parameter indices in FILTER_DESC.
const KIND: usize = 0;
const FREQ: usize = 1;
const Q: usize = 2;
const GAIN_DB: usize = 3;

pub fn build_filter(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = FILTER_DESC.params;
    let params = FilterParams {
        kind: FilterKind::ALL[v.choice(&p[0])],
        freq: v.number(&p[1]),
        q: v.number(&p[2]),
        gain_db: v.number(&p[3]),
    };
    Ok(Box::new(Biquad::new(&params, sr)))
}

#[derive(Debug, Clone, Copy)]
pub enum FilterKind {
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peak,
    Lowshelf,
    Highshelf,
}

impl FilterKind {
    /// Same order as the choices in FILTER_DESC.
    pub const ALL: [FilterKind; 7] = [
        FilterKind::Lowpass,
        FilterKind::Highpass,
        FilterKind::Bandpass,
        FilterKind::Notch,
        FilterKind::Peak,
        FilterKind::Lowshelf,
        FilterKind::Highshelf,
    ];
}

#[derive(Debug, Clone)]
pub struct FilterParams {
    pub kind: FilterKind,
    pub freq: f32,
    pub q: f32,
    /// Only used by peak, lowshelf and highshelf.
    pub gain_db: f32,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self { kind: FilterKind::Lowpass, freq: 1000.0, q: 0.707, gain_db: 0.0 }
    }
}

/// Normalized biquad coefficients (a0 = 1).
#[derive(Debug, Clone, Copy)]
struct Coefs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// Biquad ("Audio EQ Cookbook" formulas by R. Bristow-Johnson),
/// transposed direct form II.
pub struct Biquad {
    /// Parameters requested by set_param and the effective parameters, which reach them
    /// gradually: sweeping the knob must not click.
    target: FilterParams,
    current: FilterParams,
    glide: f32,
    sr: f32,
    c: Coefs,
    z1: f32,
    z2: f32,
}

/// Every how many samples the coefficients are recomputed during a sweep.
const GLIDE_STEP: usize = 16;

impl Biquad {
    pub fn new(p: &FilterParams, sr: f32) -> Self {
        Self {
            target: p.clone(),
            current: p.clone(),
            // about 20 ms to reach the target
            glide: (-(GLIDE_STEP as f32) / (0.02 * sr)).exp(),
            sr,
            c: coefficients(p, sr),
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Moves the effective parameters toward the requested ones; true if they changed.
    fn glide(&mut self) -> bool {
        let (t, c, k) = (&self.target, &mut self.current, self.glide);
        if t.freq == c.freq && t.q == c.q && t.gain_db == c.gain_db {
            return false;
        }
        let close = (t.freq / c.freq - 1.0).abs() < 1e-3 && (t.q - c.q).abs() < 1e-3 && (t.gain_db - c.gain_db).abs() < 1e-2;
        if close {
            c.freq = t.freq;
            c.q = t.q;
            c.gain_db = t.gain_db;
        } else {
            // frequency moves on a logarithmic scale, as it is heard
            c.freq = (t.freq.ln() + (c.freq.ln() - t.freq.ln()) * k).exp();
            c.q = t.q + (c.q - t.q) * k;
            c.gain_db = t.gain_db + (c.gain_db - t.gain_db) * k;
        }
        true
    }
}

/// Coefficient computation: does not allocate, so it is also safe in `set_param`.
fn coefficients(p: &FilterParams, sr: f32) -> Coefs {
    let freq = p.freq.clamp(10.0, sr * 0.45);
    let q = p.q.max(0.05);
    let w0 = 2.0 * PI * freq / sr;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a = 10f32.powf(p.gain_db / 40.0);

    let (b0, b1, b2, a0, a1, a2) = match p.kind {
        FilterKind::Lowpass => {
            ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
        }
        FilterKind::Highpass => {
            ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
        }
        FilterKind::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        FilterKind::Peak => {
            (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
        }
        FilterKind::Lowshelf => {
            let sq = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) - (a - 1.0) * cos + sq),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - sq),
                (a + 1.0) + (a - 1.0) * cos + sq,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - sq,
            )
        }
        FilterKind::Highshelf => {
            let sq = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) + (a - 1.0) * cos + sq),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - sq),
                (a + 1.0) - (a - 1.0) * cos + sq,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - sq,
            )
        }
    };

    Coefs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

impl Block for Biquad {
    fn process(&mut self, buf: &mut [f32]) {
        for chunk in buf.chunks_mut(GLIDE_STEP) {
            if self.glide() {
                self.c = coefficients(&self.current, self.sr);
            }
            let c = self.c;
            for s in chunk {
                let x = *s;
                let y = c.b0 * x + self.z1;
                self.z1 = c.b1 * x - c.a1 * y + self.z2;
                self.z2 = c.b2 * x - c.a2 * y;
                *s = y;
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            // changing the type is a jump anyway: it is applied immediately
            KIND => {
                let kind = FilterKind::ALL[(value as usize).min(FilterKind::ALL.len() - 1)];
                self.target.kind = kind;
                self.current.kind = kind;
                self.c = coefficients(&self.current, self.sr);
            }
            FREQ => self.target.freq = value.max(1.0),
            Q => self.target.q = value,
            GAIN_DB => self.target.gain_db = value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn sine(freq: f32, amp: f32, from: usize, n: usize) -> Vec<f32> {
        (from..from + n).map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn set_param_changes_frequency_live() {
        let mut f = Biquad::new(&FilterParams { freq: 1000.0, ..Default::default() }, SR);
        let mut buf = sine(1000.0, 1.0, 0, 9600);
        f.process(&mut buf);
        let before = peak(&buf[4800..]);
        assert!(before > 0.6, "with freq 1000 the 1 kHz sine passes: {before}");

        f.set_param(FREQ, 200.0);
        let mut buf = sine(1000.0, 1.0, 9600, 9600);
        f.process(&mut buf);
        let after = peak(&buf[4800..]);
        assert!(after < 0.1, "with freq 200 the 1 kHz sine must be attenuated: {after}");
    }

    #[test]
    fn frequency_sweep_has_no_jumps() {
        // 300 Hz sine through a low-pass that suddenly drops from 8 kHz to 400 Hz:
        // the output must change gradually, without discontinuities.
        let mut f = Biquad::new(&FilterParams { freq: 8000.0, ..Default::default() }, SR);
        let n = 9600;
        let mut buf: Vec<f32> = (0..n).map(|i| (std::f32::consts::TAU * 300.0 * i as f32 / SR).sin() * 0.5).collect();
        f.process(&mut buf[..4800]);
        f.set_param(FREQ, 400.0);
        f.process(&mut buf[4800..]);
        let max_step = buf.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // the maximum slope of a 300 Hz sine of amplitude 0.5 is about 0.02 per sample
        assert!(max_step < 0.03, "jump {max_step}");
    }
}
