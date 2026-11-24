//! Two-transistor shunt-feedback fuzz in the spirit of the Dallas Arbiter Fuzz Face.
//!
//! Model (per sample, in this order):
//! 1. Pickup loading: the very low input impedance of the circuit shaves the top of the
//!    guitar (one-pole low-pass, lower for germanium) after a coupling high-pass at ~70 Hz.
//! 2. At 4x oversampling, in the "core":
//!    - an envelope follower of the input (attack ~8 ms, release ~120 ms);
//!    - the **clean-up law**: the gain is `g_lo + (g_hi - g_lo) * s^2` with
//!      `s = env / (env + knee)`. A loud input sees the full fuzz gain, a quiet one (guitar
//!      volume rolled back) stays near `g_lo` and goes through the soft part of the curve,
//!      so it gets cleaner and not only quieter;
//!    - germanium "sputter": a small dead zone that widens as the envelope falls, scaled by
//!      `(1 - bias)^2` (starved transistor, only for germanium);
//!    - stage 1: asymmetric `tanh(z + b) - tanh(b)` (germanium: bigger offset; silicon:
//!      mixed with a harder clipper), stage 2: a second, harder `tanh` for the smooth
//!      square-ish top at full fuzz.
//! 3. DC blocker, two cascaded one-pole low-passes (dark output; silicon brighter), volume.
//!
//! `fuzz`, `volume_db` and `bias` are smoothed (15 ms); `transistor` switches the voicing
//! live through smoothed mixing values as well, so it does not click.

use std::f32::consts::TAU;

use anyhow::Result;

use super::oversample::Oversampler4;
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, one_pole_coef, Block, Smoothed};

pub const FUZZFACE_DESC: BlockDesc = BlockDesc {
    kind: "fuzzface",
    label: "Fuzz Face",
    category: Category::Drive,
    params: &[
        ParamDesc::number("fuzz", "Fuzz", 0.0, 1.0, 0.7, ""),
        ParamDesc::number("volume_db", "Volume", -40.0, 6.0, -10.0, "dB"),
        ParamDesc::choice("transistor", "Transistor", &["germanium", "silicon"], 0),
        // Germanium: low bias starves the transistors (sputter, gating).
        ParamDesc::number("bias", "Bias", 0.0, 1.0, 0.7, ""),
    ],
};

const FUZZ: usize = 0;
const VOLUME: usize = 1;
const TRANSISTOR: usize = 2;
const BIAS: usize = 3;

const SMOOTH_MS: f32 = 15.0;
const MAX_SPUTTER: f32 = 0.025;

fn lp_coef(hz: f32, sr: f32) -> f32 {
    1.0 - (-TAU * hz.min(0.45 * sr) / sr).exp()
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

struct Core {
    fuzz: Smoothed,
    bias: Smoothed,
    /// 0 = germanium, 1 = silicon (smoothed so the switch does not click).
    si: Smoothed,
    env: f32,
    env_att: f32,
    env_rel: f32,
}

impl Core {
    fn new(v: &ParamValues, sr: f32) -> Core {
        let p = FUZZFACE_DESC.params;
        Core {
            fuzz: Smoothed::new(v.number(&p[FUZZ]), SMOOTH_MS, sr),
            bias: Smoothed::new(v.number(&p[BIAS]), SMOOTH_MS, sr),
            si: Smoothed::new(v.choice(&p[TRANSISTOR]) as f32, SMOOTH_MS, sr),
            env: 0.0,
            env_att: one_pole_coef(8.0, sr),
            env_rel: one_pole_coef(120.0, sr),
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let (fuzz, bias, si) = (self.fuzz.next(), self.bias.next(), self.si.next());
        let ax = x.abs();
        let coef = if ax > self.env { self.env_att } else { self.env_rel };
        self.env = flush(ax + (self.env - ax) * coef);

        // clean-up law
        let knee = 0.14 - 0.05 * si;
        let s = self.env / (self.env + knee);
        let s2 = s * s;
        let g_lo = 1.5 + 3.0 * fuzz;
        let g_hi = db_to_lin(14.0 + 34.0 * fuzz + 6.0 * si);
        let g = g_lo + (g_hi - g_lo) * s2;

        // germanium sputter
        let d0 = (1.0 - bias) * (1.0 - bias) * MAX_SPUTTER * (1.0 - si);
        let d = d0 * (1.6 - self.env / (self.env + 2.0 * d0 + 1e-9));
        let u = x.signum() * (ax - d).max(0.0);

        // stage 1: asymmetric, silicon adds a harder clipper
        let b = (0.32 - 0.22 * si) * (0.5 + 0.5 * bias);
        let z = g * u + b;
        let soft = z.tanh() - b.tanh();
        let hard = z.clamp(-1.0, 1.0) - b.clamp(-1.0, 1.0);
        let y1 = soft + 0.45 * si * (hard - soft);
        // stage 2: smooth square-ish top
        let k2 = 1.6;
        (k2 * y1).tanh() * (1.0 / k2.tanh())
    }
}

pub fn build_fuzzface(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = FUZZFACE_DESC.params;
    let si = v.choice(&p[TRANSISTOR]) as f32;
    Ok(Box::new(FuzzFace {
        core: Core::new(v, 4.0 * sr),
        os: Oversampler4::new(),
        hp_a: 1.0 - TAU * 70.0 / sr,
        hp_x: 0.0,
        hp_y: 0.0,
        load: 0.0,
        load_a: Smoothed::new(load_coef(si, sr), SMOOTH_MS, sr),
        dc_x: 0.0,
        dc_y: 0.0,
        dc_r: 1.0 - TAU * 10.0 / sr,
        tone_a: Smoothed::new(tone(si, sr), SMOOTH_MS, sr),
        lp1: 0.0,
        lp2: 0.0,
        post: Smoothed::new(db_to_lin(v.number(&p[VOLUME])), SMOOTH_MS, sr),
        sr,
    }))
}

fn load_coef(si: f32, sr: f32) -> f32 {
    lp_coef(4200.0 + 2800.0 * si, sr)
}

fn tone(si: f32, sr: f32) -> f32 {
    lp_coef(3600.0 + 2400.0 * si, sr)
}

pub struct FuzzFace {
    core: Core,
    os: Oversampler4,
    hp_a: f32,
    hp_x: f32,
    hp_y: f32,
    load: f32,
    load_a: Smoothed,
    dc_x: f32,
    dc_y: f32,
    dc_r: f32,
    tone_a: Smoothed,
    lp1: f32,
    lp2: f32,
    post: Smoothed,
    sr: f32,
}

impl Block for FuzzFace {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let x = *s;
            let y = flush(x - self.hp_x + self.hp_a * self.hp_y);
            self.hp_x = x;
            self.hp_y = y;
            self.load = flush(self.load + self.load_a.next() * (y - self.load));
            *s = self.load;
        }
        let core = &mut self.core;
        self.os.process(buf, |x| core.tick(x));
        for s in buf.iter_mut() {
            let y = *s;
            let out = flush(y - self.dc_x + self.dc_r * self.dc_y);
            self.dc_x = y;
            self.dc_y = out;
            let a = self.tone_a.next();
            self.lp1 = flush(self.lp1 + a * (out - self.lp1));
            self.lp2 = flush(self.lp2 + a * (self.lp1 - self.lp2));
            *s = self.lp2 * self.post.next();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            FUZZ => self.core.fuzz.target = value,
            VOLUME => self.post.target = db_to_lin(value),
            TRANSISTOR => {
                let si = value.round().clamp(0.0, 1.0);
                self.core.si.target = si;
                self.load_a.target = load_coef(si, self.sr);
                self.tone_a.target = tone(si, self.sr);
            }
            BIAS => self.core.bias.target = value,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.os.reset();
        self.core.env = 0.0;
        self.hp_x = 0.0;
        self.hp_y = 0.0;
        self.load = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.lp1 = 0.0;
        self.lp2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    const SR: f32 = 48000.0;

    fn build(params: &[(&str, f32)]) -> Box<dyn Block> {
        let mut v = ParamValues::default();
        for (k, x) in params {
            v.set(k, Value::Number(*x));
        }
        build_fuzzface(&v, SR).unwrap()
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn noise(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                amp * ((s >> 8) as f32 / 8388608.0 - 1.0)
            })
            .collect()
    }

    fn mag(buf: &[f32], freq: f32) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &x) in buf.iter().enumerate() {
            let w = std::f64::consts::TAU * freq as f64 * i as f64 / SR as f64;
            re += x as f64 * w.cos();
            im -= x as f64 * w.sin();
        }
        (re * re + im * im).sqrt()
    }

    /// THD (harmonics 2..8 over the fundamental) of 1 kHz sine, after the envelope settled.
    fn thd(params: &[(&str, f32)], amp_db: f32) -> f32 {
        let mut f = build(params);
        let n = 48000; // 1000 periods
        let mut buf = sine(1000.0, 10f32.powf(amp_db / 20.0), 2 * n);
        f.process(&mut buf);
        let b = &buf[n..];
        let fund = mag(b, 1000.0);
        let h: f64 = (2..=8).map(|k| mag(b, 1000.0 * k as f32).powi(2)).sum::<f64>().sqrt();
        (h / fund.max(1e-12)) as f32
    }

    #[test]
    fn cleans_up_with_lower_input_level() {
        for tr in [0.0, 1.0] {
            let p = [("fuzz", 0.8), ("transistor", tr), ("volume_db", 0.0)];
            let (quiet, loud) = (thd(&p, -30.0), thd(&p, -10.0));
            println!("transistor {tr}: THD at -30 dBFS = {quiet:.3}, at -10 dBFS = {loud:.3}");
            assert!(quiet * 2.0 < loud, "{quiet} {loud}");
        }
    }

    #[test]
    fn silicon_is_harsher_than_germanium() {
        let ge = thd(&[("fuzz", 0.8), ("transistor", 0.0)], -10.0);
        let si = thd(&[("fuzz", 0.8), ("transistor", 1.0)], -10.0);
        println!("THD -10 dBFS: germanium {ge:.3}, silicon {si:.3}");
        assert!(si > ge);
    }

    #[test]
    fn stays_finite_and_bounded_over_sweeps() {
        for fuzz in [0.0, 0.5, 1.0] {
            for tr in [0.0, 1.0] {
                for bias in [0.0, 0.5, 1.0] {
                    let mut f = build(&[("fuzz", fuzz), ("transistor", tr), ("bias", bias), ("volume_db", 6.0)]);
                    let mut buf = noise(9600, 1.0, 7);
                    buf[100] = 1.0;
                    buf[101] = -1.0;
                    buf.extend(sine(110.0, 1.0, 4800));
                    buf.extend(vec![0.0; 4800]);
                    f.process(&mut buf);
                    let p = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                    assert!(buf.iter().all(|s| s.is_finite()), "{fuzz} {tr} {bias}");
                    assert!(p < 4.0, "{fuzz} {tr} {bias}: {p}");
                }
            }
        }
    }

    #[test]
    fn silence_decays_to_exact_zero() {
        let mut f = build(&[]);
        let mut buf = sine(220.0, 0.5, 4800);
        buf.extend(vec![0.0; 96000]);
        f.process(&mut buf);
        assert!(buf[buf.len() - 100..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn live_set_param_does_not_click() {
        for (idx, val) in [(FUZZ, 0.0), (VOLUME, -30.0), (TRANSISTOR, 1.0), (BIAS, 0.0)] {
            let mut f = build(&[]);
            let mut warm = sine(220.0, 0.3, 9600);
            f.process(&mut warm);
            f.set_param(idx, val);
            let mut buf = sine(220.0, 0.3, 960);
            // continue the phase of the warm-up so only the parameter jump can cause a step
            for (i, s) in buf.iter_mut().enumerate() {
                *s = 0.3 * (TAU * 220.0 * (9600 + i) as f32 / SR).sin();
            }
            f.process(&mut buf);
            assert!(buf.iter().all(|s| s.is_finite()));
            let jump = buf.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            assert!(jump < 0.6, "param {idx}: {jump}");
        }
    }

    #[test]
    fn every_param_changes_the_output() {
        let src = sine(220.0, 0.2, 4800);
        let mut reference = src.clone();
        build(&[]).process(&mut reference);
        for (i, v) in [0.0, -30.0, 1.0, 0.0].iter().enumerate() {
            let mut f = build(&[]);
            f.set_param(i, *v);
            let mut buf = src.clone();
            f.process(&mut buf);
            let diff: f32 = buf.iter().zip(&reference).map(|(a, b)| (a - b).abs()).sum();
            assert!(diff > 1.0, "param {i}: {diff}");
        }
    }
}
