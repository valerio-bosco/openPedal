//! Amp tone stack: interacting bass, mid and treble (task D4, see docs/PLAN.md §7).
//!
//! TMB model by D. T. Yeh (3rd-order analog filter), discretized with the
//! bilinear transform. The mid and treble pots are linear,
//! the bass one has an approximated logarithmic law (`l = bass²`).
//! The gain is normalized to 1 at 1 kHz with all knobs at 0.5.

use std::f64::consts::TAU;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const TONESTACK_DESC: BlockDesc = BlockDesc {
    kind: "tonestack",
    label: "Tone stack",
    category: Category::Amp,
    params: &[
        ParamDesc::number("bass", "Bass", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("mid", "Mid", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("treble", "Treble", 0.0, 1.0, 0.5, ""),
        ParamDesc::choice("model", "Model", &["fender", "marshall", "vox"], 0),
    ],
};

const BASS: usize = 0;
const MID: usize = 1;
const TREBLE: usize = 2;
const MODEL: usize = 3;

/// Coefficient recomputation step (samples).
const STEP: usize = 32;

/// Components of a model: R1 (treble), R2 (bass), R3 (mid), R4, C1, C2, C3.
struct Components {
    r1: f64,
    r2: f64,
    r3: f64,
    r4: f64,
    c1: f64,
    c2: f64,
    c3: f64,
}

const MODELS: [Components; 3] = [
    // Fender Bassman
    Components { r1: 250e3, r2: 1e6, r3: 25e3, r4: 56e3, c1: 250e-12, c2: 20e-9, c3: 20e-9 },
    // Marshall JCM800
    Components { r1: 220e3, r2: 1e6, r3: 25e3, r4: 33e3, c1: 470e-12, c2: 22e-9, c3: 22e-9 },
    // Vox AC30
    Components { r1: 1e6, r2: 1e6, r3: 10e3, r4: 100e3, c1: 50e-12, c2: 22e-9, c3: 22e-9 },
];

/// Analog coefficients H(s) = (b1 s + b2 s² + b3 s³) / (1 + a1 s + a2 s² + a3 s³).
fn analog(c: &Components, l: f64, m: f64, t: f64) -> ([f64; 3], [f64; 3]) {
    let Components { r1, r2, r3, r4, c1, c2, c3 } = *c;
    let b1 = t * c1 * r1 + m * c3 * r3 + l * (c1 * r2 + c2 * r2) + (c1 * r3 + c2 * r3);
    let b2 = t * (c1 * c2 * r1 * r4 + c1 * c3 * r1 * r4)
        - m * m * (c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + m * (c1 * c3 * r1 * r3 + c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * (c1 * c2 * r1 * r2 + c1 * c2 * r2 * r4 + c1 * c3 * r2 * r4)
        + l * m * (c1 * c3 * r2 * r3 + c2 * c3 * r2 * r3)
        + (c1 * c2 * r1 * r3 + c1 * c2 * r3 * r4 + c1 * c3 * r3 * r4);
    let b3 = l * m * (c1 * c2 * c3 * r1 * r2 * r3 + c1 * c2 * c3 * r2 * r3 * r4)
        - m * m * (c1 * c2 * c3 * r1 * r3 * r4 + c1 * c2 * c3 * r3 * r3 * r4)
        + m * (c1 * c2 * c3 * r1 * r3 * r3 + c1 * c2 * c3 * r3 * r3 * r4)
        + t * c1 * c2 * c3 * r1 * r3 * r4
        - t * m * c1 * c2 * c3 * r1 * r3 * r4
        + t * l * c1 * c2 * c3 * r1 * r2 * r4;
    let a1 = c1 * r1 + c1 * r3 + c2 * r3 + c2 * r4 + c3 * r4 + m * c3 * r3 + l * (c1 * r2 + c2 * r2);
    let a2 = m * (c1 * c3 * r1 * r3 - c2 * c3 * r3 * r4 + c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * m * (c1 * c3 * r2 * r3 + c2 * c3 * r2 * r3)
        - m * m * (c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * (c1 * c2 * r2 * r4 + c1 * c2 * r1 * r2 + c1 * c3 * r2 * r4 + c2 * c3 * r2 * r4)
        + (c1 * c2 * r1 * r4 + c1 * c3 * r1 * r4 + c1 * c2 * r3 * r4 + c1 * c2 * r1 * r3 + c1 * c3 * r3 * r4 + c2 * c3 * r3 * r4);
    let a3 = l * m * (c1 * c2 * c3 * r1 * r2 * r3 + c1 * c2 * c3 * r2 * r3 * r4)
        - m * m * (c1 * c2 * c3 * r1 * r3 * r4 + c1 * c2 * c3 * r3 * r3 * r4)
        + m * (c1 * c2 * c3 * r3 * r3 * r4 + c1 * c2 * c3 * r1 * r3 * r4)
        + l * c1 * c2 * c3 * r1 * r2 * r4
        + c1 * c2 * c3 * r1 * r3 * r4;
    ([b1, b2, b3], [a1, a2, a3])
}

/// Bilinear transform s = 2 sr (1 - z⁻¹)/(1 + z⁻¹): returns (b[0..4], a[0..4]) with a[0] = 1.
fn bilinear(b: [f64; 3], a: [f64; 3], sr: f64) -> ([f64; 4], [f64; 4]) {
    let k = 2.0 * sr;
    // (1 - z⁻¹)^i (1 + z⁻¹)^(3-i) for i = 0..3, in powers of z⁻¹
    const P: [[f64; 4]; 4] = [[1.0, 3.0, 3.0, 1.0], [1.0, 1.0, -1.0, -1.0], [1.0, -1.0, -1.0, 1.0], [1.0, -3.0, 3.0, -1.0]];
    let num = [0.0, b[0] * k, b[1] * k * k, b[2] * k * k * k];
    let den = [1.0, a[0] * k, a[1] * k * k, a[2] * k * k * k];
    let (mut nb, mut da) = ([0.0; 4], [0.0; 4]);
    for i in 0..4 {
        for j in 0..4 {
            nb[j] += num[i] * P[i][j];
            da[j] += den[i] * P[i][j];
        }
    }
    let a0 = da[0];
    ([nb[0] / a0, nb[1] / a0, nb[2] / a0, nb[3] / a0], [1.0, da[1] / a0, da[2] / a0, da[3] / a0])
}

/// Pot values (bass, mid, treble) for the knob positions 0..1.
fn pots(bass: f64, mid: f64, treble: f64) -> (f64, f64, f64) {
    (bass * bass, mid, treble)
}

/// Magnitude of the analog H(s) at `freq` (only used to normalize the gain).
fn analog_gain(c: &Components, l: f64, m: f64, t: f64, freq: f64) -> f64 {
    let (b, a) = analog(c, l, m, t);
    let w = TAU * freq;
    // s = jw: s² = -w², s³ = -j w³
    let nr = -b[1] * w * w;
    let ni = b[0] * w - b[2] * w * w * w;
    let dr = 1.0 - a[1] * w * w;
    let di = a[0] * w - a[2] * w * w * w;
    ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
}

pub fn build_tonestack(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = TONESTACK_DESC.params;
    let bass = v.number(&p[BASS]);
    let mid = v.number(&p[MID]);
    let treble = v.number(&p[TREBLE]);
    let mut ts = ToneStack {
        sr: sr as f64,
        model: v.choice(&p[MODEL]).min(2),
        cur: [bass, mid, treble],
        target: [bass, mid, treble],
        smooth: 1.0 - (-(STEP as f64) / (0.015 * sr as f64)).exp(),
        makeup: 1.0,
        b: [0.0; 4],
        a: [1.0, 0.0, 0.0, 0.0],
        z: [0.0; 3],
    };
    ts.update_model();
    Ok(Box::new(ts))
}

pub struct ToneStack {
    sr: f64,
    model: usize,
    /// current and target values of bass, mid, treble (smoothed per chunk)
    cur: [f32; 3],
    target: [f32; 3],
    smooth: f64,
    makeup: f64,
    b: [f64; 4],
    a: [f64; 4],
    z: [f64; 3],
}

impl ToneStack {
    /// Model change: recomputes the normalization and the coefficients.
    fn update_model(&mut self) {
        let c = &MODELS[self.model];
        let (l, m, t) = pots(0.5, 0.5, 0.5);
        self.makeup = 1.0 / analog_gain(c, l, m, t, 1000.0);
        self.update_coefs();
    }

    fn update_coefs(&mut self) {
        let (l, m, t) = pots(self.cur[0] as f64, self.cur[1] as f64, self.cur[2] as f64);
        let (b, a) = analog(&MODELS[self.model], l, m, t);
        let (mut bd, ad) = bilinear(b, a, self.sr);
        for x in &mut bd {
            *x *= self.makeup;
        }
        self.b = bd;
        self.a = ad;
    }
}

impl Block for ToneStack {
    fn process(&mut self, buf: &mut [f32]) {
        for chunk in buf.chunks_mut(STEP) {
            if self.cur != self.target {
                for i in 0..3 {
                    let d = self.target[i] - self.cur[i];
                    self.cur[i] = if d.abs() < 1e-4 { self.target[i] } else { self.cur[i] + d * self.smooth as f32 };
                }
                self.update_coefs();
            }
            let (b, a) = (self.b, self.a);
            for s in chunk {
                // transposed direct form II
                let x = *s as f64;
                let y = b[0] * x + self.z[0];
                self.z[0] = b[1] * x - a[1] * y + self.z[1];
                self.z[1] = b[2] * x - a[2] * y + self.z[2];
                self.z[2] = b[3] * x - a[3] * y;
                *s = y as f32;
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            BASS | MID | TREBLE => self.target[index] = value.clamp(0.0, 1.0),
            MODEL => {
                let m = (value.round().max(0.0) as usize).min(2);
                if m != self.model {
                    self.model = m;
                    self.update_model();
                }
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.z = [0.0; 3];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    const SR: f32 = 48000.0;

    fn make(model: usize, bass: f32, mid: f32, treble: f32) -> Box<dyn Block> {
        let mut v = ParamValues::default();
        v.set("model", Value::Number(model as f32));
        v.set("bass", Value::Number(bass));
        v.set("mid", Value::Number(mid));
        v.set("treble", Value::Number(treble));
        build_tonestack(&v, SR).unwrap()
    }

    /// Gain in dB at `freq`, measured with a sine.
    fn gain_db(model: usize, bass: f32, mid: f32, treble: f32, freq: f32) -> f32 {
        let mut ts = make(model, bass, mid, treble);
        let n = 24000;
        let mut buf: Vec<f32> = (0..n).map(|i| 0.1 * (std::f32::consts::TAU * freq * i as f32 / SR).sin()).collect();
        ts.process(&mut buf);
        let peak = buf[n / 2..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        20.0 * (peak / 0.1).log10()
    }

    #[test]
    fn bass_is_audible() {
        for (model, name) in ["fender", "marshall", "vox"].iter().enumerate() {
            let low = gain_db(model, 0.0, 0.5, 0.5, 100.0);
            let high = gain_db(model, 1.0, 0.5, 0.5, 100.0);
            println!("{name}: 100 Hz bass=0 {low:.1} dB, bass=1 {high:.1} dB");
            assert!(high - low > 6.0, "{name}: {low} vs {high}");
        }
    }

    #[test]
    fn gain_normalized_at_1khz() {
        for model in 0..3 {
            let g = gain_db(model, 0.5, 0.5, 0.5, 1000.0);
            assert!(g.abs() < 0.3, "model {model}: {g} dB");
        }
    }

    #[test]
    fn treble_is_audible() {
        let low = gain_db(0, 0.5, 0.5, 0.0, 5000.0);
        let high = gain_db(0, 0.5, 0.5, 1.0, 5000.0);
        println!("fender: 5 kHz treble=0 {low:.1} dB, treble=1 {high:.1} dB");
        assert!(high - low > 6.0);
    }

    #[test]
    fn stable_for_every_combination() {
        let vals = [0.0, 0.5, 1.0];
        for model in 0..3 {
            for &b in &vals {
                for &m in &vals {
                    for &t in &vals {
                        let mut ts = make(model, b, m, t);
                        let mut buf: Vec<f32> = (0..96000).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect();
                        ts.process(&mut buf);
                        let tail = buf[90000..].iter().fold(0.0f32, |mx, s| mx.max(s.abs()));
                        assert!(buf.iter().all(|s| s.is_finite()), "{model} {b} {m} {t}");
                        assert!(tail < 1e-4, "does not decay: {model} {b} {m} {t} tail {tail}");
                    }
                }
            }
        }
    }

    #[test]
    fn live_change_without_jumps() {
        let mut ts = make(0, 0.5, 0.5, 0.5);
        let mut buf: Vec<f32> = (0..9600).map(|i| 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin()).collect();
        let (a, b) = buf.split_at_mut(4800);
        ts.process(a);
        ts.set_param(BASS, 1.0);
        ts.set_param(TREBLE, 0.0);
        ts.process(b);
        let mut jump = 0.0f32;
        for w in buf[4000..].windows(2) {
            jump = jump.max((w[1] - w[0]).abs());
        }
        assert!(jump < 0.2, "jump {jump}");
    }
}
