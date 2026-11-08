//! Gated, sputtery transistor fuzz in the spirit of the Z.Vex Fuzz Factory
//! (task P1, see docs/PLAN.md §9).
//!
//! Model (per sample, in this order):
//! 1. Input coupling: one-pole high-pass at ~110 Hz (tightens the lows).
//! 2. At 4x oversampling, in the "core":
//!    - an envelope follower of the input (attack ~5 ms, release ~60 ms);
//!    - the **starved dead zone** `u = sign(x) * max(|x| - d, 0)` placed BEFORE the gain,
//!      so it works on the input level, with `d = d0 * (1.6 - env / (env + 2 * d0))` and
//!      `d0 = gate * 0.06`. A decaying note shrinks into narrowing pulses (sputter) and then
//!      silence, because the gap widens as the envelope falls; at a note onset the envelope
//!      is still low, so the gap starts wide and closes as the envelope charges (the note
//!      "blooms");
//!    - the pre-gain (+20..+50 dB from `drive`) and the asymmetric clipper
//!      `tanh(g*u + b) - tanh(b)`, cross-faded by `comp` with a hard clipper
//!      (`comp` = 0 smooth and compressed, `comp` = 1 hard and spiky). `bias` sets `b`.
//! 3. DC blocker, two cascaded one-pole low-passes at `tone_hz`, `level_db`.
//!
//! Every parameter is live: gains, dead zone, comp, bias and tone are smoothed (15 ms).

use std::f32::consts::TAU;

use anyhow::Result;

use super::oversample::Oversampler4;
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, one_pole_coef, Block, Smoothed};

pub const FUZZ_DESC: BlockDesc = BlockDesc {
    kind: "fuzz",
    label: "Fuzz",
    category: Category::Drive,
    params: &[
        ParamDesc::number("drive", "Drive", 0.0, 1.0, 0.7, ""),
        // Starves the transistors: small signals stop conducting, notes decay into "velcro" and cut off.
        ParamDesc::number("gate", "Gate", 0.0, 1.0, 0.3, ""),
        // Low = smooth, compressed saturation; high = harder, spikier clipping.
        ParamDesc::number("comp", "Comp", 0.0, 1.0, 0.5, ""),
        // Asymmetry of the clipping (even harmonics).
        ParamDesc::number("bias", "Bias", 0.0, 1.0, 0.4, ""),
        ParamDesc::log("tone_hz", "Tone", 1000.0, 12000.0, 6000.0, "Hz"),
        ParamDesc::number("level_db", "Level", -40.0, 6.0, -12.0, "dB"),
    ],
};

const DRIVE: usize = 0;
const GATE: usize = 1;
const COMP: usize = 2;
const BIAS: usize = 3;
const TONE: usize = 4;
const LEVEL: usize = 5;

/// Largest dead-zone half-width (input domain, after the coupling filter) at `gate = 1`.
const MAX_DEAD_ZONE: f32 = 0.06;
const SMOOTH_MS: f32 = 15.0;
const COUPLING_HZ: f32 = 110.0;

/// Pre-gain: +20 dB at `drive = 0`, +50 dB at `drive = 1`, slightly eased so the
/// middle of the knob is spread perceptually.
fn drive_gain(drive: f32) -> f32 {
    db_to_lin(20.0 + 30.0 * drive.powf(0.8))
}

fn tone_coef(hz: f32, sr: f32) -> f32 {
    1.0 - (-TAU * hz.min(0.45 * sr) / sr).exp()
}

impl Core {
    /// `sr` is the rate at which `tick` will be called.
    fn new(v: &ParamValues, sr: f32) -> Core {
        let p = FUZZ_DESC.params;
        Core {
            gain: Smoothed::new(drive_gain(v.number(&p[DRIVE])), SMOOTH_MS, sr),
            dead: Smoothed::new(v.number(&p[GATE]) * MAX_DEAD_ZONE, SMOOTH_MS, sr),
            comp: Smoothed::new(v.number(&p[COMP]), SMOOTH_MS, sr),
            bias: Smoothed::new(v.number(&p[BIAS]), SMOOTH_MS, sr),
            env: 0.0,
            env_att: one_pole_coef(5.0, sr),
            env_rel: one_pole_coef(60.0, sr),
        }
    }
}

pub fn build_fuzz(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = FUZZ_DESC.params;
    Ok(Box::new(Fuzz {
        core: Core::new(v, 4.0 * sr),
        os: Oversampler4::new(),
        hp_a: 1.0 - TAU * COUPLING_HZ / sr,
        hp_x: 0.0,
        hp_y: 0.0,
        dc_x: 0.0,
        dc_y: 0.0,
        dc_r: 1.0 - TAU * 10.0 / sr,
        tone: Smoothed::new(tone_coef(v.number(&p[TONE]), sr), SMOOTH_MS, sr),
        lp1: 0.0,
        lp2: 0.0,
        post: Smoothed::new(db_to_lin(v.number(&p[LEVEL])), SMOOTH_MS, sr),
        sr,
    }))
}

/// The nonlinear part, run at the oversampled rate.
struct Core {
    gain: Smoothed,
    /// Dead-zone half-width before the envelope modulation.
    dead: Smoothed,
    comp: Smoothed,
    bias: Smoothed,
    env: f32,
    env_att: f32,
    env_rel: f32,
}

impl Core {
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let (g, d0, c, b) = (self.gain.next(), self.dead.next(), self.comp.next(), self.bias.next());
        let ax = x.abs();
        let coef = if ax > self.env { self.env_att } else { self.env_rel };
        self.env = ax + (self.env - ax) * coef;
        // wide gap while the envelope is low (decay tail, note onset), narrow while it is high
        let d = d0 * (1.6 - self.env / (self.env + 2.0 * d0 + 1e-9));
        let u = x.signum() * (ax - d).max(0.0);
        let z = g * u + b;
        let soft = z.tanh() - b.tanh();
        let hard = z.clamp(-1.0, 1.0) - b.clamp(-1.0, 1.0);
        soft + c * (hard - soft)
    }
}

pub struct Fuzz {
    core: Core,
    os: Oversampler4,
    hp_a: f32,
    hp_x: f32,
    hp_y: f32,
    dc_x: f32,
    dc_y: f32,
    dc_r: f32,
    /// One-pole low-pass coefficient (`1 - exp(-2 pi f / sr)`), smoothed.
    tone: Smoothed,
    lp1: f32,
    lp2: f32,
    post: Smoothed,
    sr: f32,
}

impl Block for Fuzz {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let x = *s;
            let y = x - self.hp_x + self.hp_a * self.hp_y;
            self.hp_x = x;
            self.hp_y = y;
            *s = y;
        }
        let core = &mut self.core;
        self.os.process(buf, |x| core.tick(x));
        for s in buf.iter_mut() {
            let y = *s;
            let out = y - self.dc_x + self.dc_r * self.dc_y;
            self.dc_x = y;
            self.dc_y = out;
            let a = self.tone.next();
            self.lp1 += a * (out - self.lp1);
            self.lp2 += a * (self.lp1 - self.lp2);
            *s = self.lp2 * self.post.next();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            DRIVE => self.core.gain.target = drive_gain(value),
            GATE => self.core.dead.target = value * MAX_DEAD_ZONE,
            COMP => self.core.comp.target = value,
            BIAS => self.core.bias.target = value,
            TONE => self.tone.target = tone_coef(value, self.sr),
            LEVEL => self.post.target = db_to_lin(value),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.os.reset();
        self.core.env = 0.0;
        self.hp_x = 0.0;
        self.hp_y = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.lp1 = 0.0;
        self.lp2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::Value;

    const SR: f32 = 48000.0;

    fn values(params: &[(&str, f32)]) -> ParamValues {
        let mut v = ParamValues::default();
        for (k, x) in params {
            v.set(k, Value::Number(*x));
        }
        v
    }

    fn build(params: &[(&str, f32)]) -> Box<dyn Block> {
        build_fuzz(&values(params), SR).unwrap()
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (TAU * freq * i as f32 / SR).sin()).collect()
    }

    /// 220 Hz note, 0.3 peak, decaying 60 dB in 1.5 s.
    fn decaying_note(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                0.3 * 10f32.powf(-60.0 * t / 20.0 / 1.5) * (TAU * 220.0 * t).sin()
            })
            .collect()
    }

    fn db(x: f32) -> f32 {
        20.0 * x.max(1e-9).log10()
    }

    fn peak_db(buf: &[f32]) -> f32 {
        db(buf.iter().fold(0.0, |m, s| m.max(s.abs())))
    }

    /// Time (s) after which the peak of every 10 ms window stays below -60 dBFS.
    fn cutoff_time(out: &[f32]) -> f32 {
        let win = 480;
        let last = out.chunks(win).rposition(|w| peak_db(w) >= -60.0);
        (last.map_or(0, |i| i + 1) * win) as f32 / SR
    }

    /// Magnitude of the DFT bin at `freq`.
    fn mag(buf: &[f32], freq: f32) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &x) in buf.iter().enumerate() {
            let w = std::f64::consts::TAU * freq as f64 * i as f64 / SR as f64;
            re += x as f64 * w.cos();
            im -= x as f64 * w.sin();
        }
        (re * re + im * im).sqrt()
    }

    /// Energy below `fmax`, excluding DC.
    fn energy_below(buf: &[f32], fmax: f32) -> f64 {
        let n = buf.len();
        let kmax = (fmax / SR * n as f32) as usize;
        (1..kmax)
            .map(|k| {
                let m = mag(buf, k as f32 * SR / n as f32) / n as f64;
                m * m
            })
            .sum()
    }

    #[test]
    fn gate_cuts_decaying_note_earlier() {
        let n = (SR * 2.0) as usize;
        let mut times = Vec::new();
        for gate in [0.0, 0.8] {
            let mut f = build(&[("drive", 0.7), ("gate", gate)]);
            let mut buf = decaying_note(n);
            f.process(&mut buf);
            times.push(cutoff_time(&buf));
        }
        println!("cutoff (< -60 dBFS): gate 0 = {:.3} s, gate 0.8 = {:.3} s", times[0], times[1]);
        assert!(times[1] + 0.1 < times[0], "{times:?}");
    }

    #[test]
    fn drive_keeps_a_gradual_tail_without_gate() {
        // gate = 0: more drive sustains the tail longer, and it fades without sudden drops
        let n = (SR * 2.0) as usize;
        let mut lo = build(&[("drive", 0.3), ("gate", 0.0)]);
        let mut hi = build(&[("drive", 1.0), ("gate", 0.0)]);
        let (mut a, mut b) = (decaying_note(n), decaying_note(n));
        lo.process(&mut a);
        hi.process(&mut b);
        let (ta, tb) = (cutoff_time(&a), cutoff_time(&b));
        println!("gate 0 cutoff: drive 0.3 = {ta:.3} s, drive 1.0 = {tb:.3} s");
        assert!(tb > ta, "more drive must sustain longer: {ta} {tb}");
        let levels: Vec<f32> = b.chunks(4800).take(10).map(peak_db).collect();
        println!("drive 1.0 gate 0 levels per 100 ms: {levels:.1?}");
        for w in levels.windows(2) {
            assert!(w[0] - w[1] < 12.0, "{levels:?}");
        }
    }

    #[test]
    fn gated_tail_turns_into_pulses() {
        // while it dies, the note spends more and more samples at (almost) zero
        let mut f = build(&[("drive", 0.7), ("gate", 0.5)]);
        let n = (SR * 2.0) as usize;
        let mut buf = decaying_note(n);
        f.process(&mut buf);
        let quiet = |w: &[f32]| w.iter().filter(|s| s.abs() < 1e-3).count() as f32 / w.len() as f32;
        let (early, late) = (quiet(&buf[4800..9600]), quiet(&buf[24000..48000]));
        println!("near-zero fraction: early = {early:.2}, late = {late:.2}");
        assert!(late > early + 0.1);
    }

    #[test]
    fn output_is_bounded_at_full_scale_and_max_drive() {
        let mut f = build(&[("drive", 1.0), ("gate", 1.0), ("comp", 1.0), ("bias", 1.0), ("level_db", 6.0)]);
        let mut buf = sine(110.0, 1.0, 9600);
        f.process(&mut buf);
        let p = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        println!("max drive peak = {p:.2}");
        assert!(buf.iter().all(|s| s.is_finite()));
        assert!(p < 4.5, "{p}");
    }

    #[test]
    fn bias_produces_even_harmonics() {
        let n = 48000; // 1 s of 200 Hz = exactly 200 periods
        let h2 = |bias: f32| {
            let mut f = build(&[("drive", 0.5), ("gate", 0.0), ("comp", 0.5), ("bias", bias), ("tone_hz", 12000.0)]);
            let mut buf = sine(200.0, 0.3, 2 * n);
            f.process(&mut buf);
            let buf = &buf[n..];
            20.0 * (mag(buf, 400.0) / mag(buf, 200.0)).log10()
        };
        let (with, without) = (h2(0.8), h2(0.0));
        println!("2nd harmonic: bias 0.8 = {with:.1} dB, bias 0 = {without:.1} dB");
        assert!(with > -40.0, "{with}");
        assert!(without < -40.0, "{without}");
    }

    #[test]
    fn oversampling_reduces_aliasing() {
        let n = 4800;
        let src = sine(5000.0, 0.5, 2 * n);
        // drive 0.3 (~+28 dB, like the drive test); at max drive the curve is a square wave
        // and no 4x oversampler can keep its aliasing low (about 10 dB only)
        for (comp, min_db) in [(0.0, 20.0), (0.5, 18.0), (1.0, 15.0)] {
            let v = values(&[("drive", 0.3), ("gate", 0.0), ("comp", comp), ("tone_hz", 12000.0), ("level_db", 0.0)]);
            let mut hq = src.clone();
            build_fuzz(&v, SR).unwrap().process(&mut hq);
            // the same nonlinearity at the base rate (no oversampling)
            let mut core = Core::new(&v, SR);
            let naive: Vec<f32> = src.iter().map(|&x| core.tick(x)).collect();
            let a = energy_below(&naive[n..], 4000.0);
            let b = energy_below(&hq[n..], 4000.0);
            let reduction = 10.0 * (a / b.max(1e-30)).log10();
            println!("aliasing below 4 kHz, comp {comp}: 1x/4x = {reduction:.1} dB");
            assert!(reduction >= min_db, "comp {comp}: {reduction}");
        }
    }

    #[test]
    fn set_param_changes_the_output() {
        let src = sine(220.0, 0.2, 4800);
        let mut reference = src.clone();
        build(&[]).process(&mut reference);
        let new_values = [0.0, 1.0, 0.0, 1.0, 1000.0, 0.0];
        for (i, v) in new_values.iter().enumerate() {
            let mut f = build(&[]);
            f.set_param(i, *v);
            let mut buf = src.clone();
            f.process(&mut buf);
            assert!(buf.iter().all(|s| s.is_finite()));
            let diff: f32 = buf.iter().zip(&reference).map(|(a, b)| (a - b).abs()).sum();
            assert!(diff > 1.0, "param {i} has no effect ({diff})");
        }
    }

    #[test]
    fn live_changes_do_not_click() {
        let mut f = build(&[]);
        let mut buf = sine(220.0, 0.3, 4800);
        f.process(&mut buf);
        f.set_param(DRIVE, 0.0);
        f.set_param(LEVEL, -30.0);
        let mut buf2 = sine(220.0, 0.3, 480);
        f.process(&mut buf2);
        assert!(buf2.iter().all(|s| s.is_finite()));
        let jump = buf2.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(jump < 0.5, "{jump}");
    }

    #[test]
    fn handles_large_blocks() {
        let mut f = build(&[]);
        let mut buf: Vec<f32> = (0..8192).map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0).collect();
        f.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
    }
}
