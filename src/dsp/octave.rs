//! Analog-style octaver: one octave down by frequency division, one octave up by
//! rectification (task P2, see docs/PLAN.md §9).
//!
//! Signal flow: the input goes through a low-pass at `track_hz` (tracking signal).
//! - down: a flip-flop toggles on rising zero crossings of the tracking signal (with
//!   hysteresis); the voice is `ff * tracking`, a square-wave-modulated copy one octave down;
//! - up: `|tracking|` minus its DC, one octave up;
//! - both voices go through a low-pass at `tone_hz`, then are mixed with the dry signal.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{one_pole_coef, Block, Smoothed};

pub const OCTAVE_DESC: BlockDesc = BlockDesc {
    kind: "octave",
    label: "Octave",
    category: Category::Modulation,
    params: &[
        ParamDesc::number("dry", "Dry", 0.0, 1.0, 1.0, ""),
        ParamDesc::number("down", "Down", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("up", "Up", 0.0, 1.0, 0.0, ""),
        // Low-pass on the octave voices.
        ParamDesc::log("tone_hz", "Tone", 300.0, 8000.0, 2000.0, "Hz"),
        // Low-pass on the tracking signal: lower = steadier tracking, less range.
        ParamDesc::log("track_hz", "Tracking", 200.0, 2000.0, 800.0, "Hz"),
    ],
};

// Parameter indices in OCTAVE_DESC.
const DRY: usize = 0;
const DOWN: usize = 1;
const UP: usize = 2;
const TONE: usize = 3;
const TRACK: usize = 4;

/// Smoothing time of the mix levels.
const MIX_MS: f32 = 15.0;
/// Time constant of the envelope of the tracking signal (the hysteresis follows it).
const ENV_MS: f32 = 50.0;
/// Hysteresis of the divider, relative to the envelope of the tracking signal.
const HYSTERESIS: f32 = 0.02;
/// Absolute minimum hysteresis: silence and very low noise do not toggle the divider.
const HYSTERESIS_FLOOR: f32 = 1e-4;
/// Corner frequency of the DC blocker on the rectified signal.
const DC_HZ: f32 = 20.0;

pub fn build_octave(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = OCTAVE_DESC.params;
    let tone_hz = v.number(&p[TONE]);
    let track_hz = v.number(&p[TRACK]);
    Ok(Box::new(Octave {
        dry: Smoothed::new(v.number(&p[DRY]), MIX_MS, sr),
        down: Smoothed::new(v.number(&p[DOWN]), MIX_MS, sr),
        up: Smoothed::new(v.number(&p[UP]), MIX_MS, sr),
        track: Lowpass::new(track_hz, sr),
        tone_down: Lowpass::new(tone_hz, sr),
        tone_up: Lowpass::new(tone_hz, sr),
        sr,
        env: 0.0,
        env_coef: one_pole_coef(ENV_MS, sr),
        ff: 1.0,
        armed: false,
        dc_x: 0.0,
        dc_y: 0.0,
        dc_r: (-TAU * DC_HZ / sr).exp(),
    }))
}

pub struct Octave {
    dry: Smoothed,
    down: Smoothed,
    up: Smoothed,
    /// Tracking signal: the input through a low-pass.
    track: Lowpass,
    tone_down: Lowpass,
    tone_up: Lowpass,
    sr: f32,
    /// Slow envelope of the magnitude of the tracking signal.
    env: f32,
    env_coef: f32,
    /// Flip-flop of the divider: +1 or -1.
    ff: f32,
    /// True after the tracking signal went below -h and before the next toggle.
    armed: bool,
    // DC blocker on the rectified signal.
    dc_x: f32,
    dc_y: f32,
    dc_r: f32,
}

impl Block for Octave {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let x = *s;
            let t = self.track.process(x);
            let mag = t.abs();
            self.env = self.env_coef * self.env + (1.0 - self.env_coef) * mag;
            let h = (HYSTERESIS * self.env).max(HYSTERESIS_FLOOR);
            // Toggle on a rising crossing: the signal must first go below -h, then above +h.
            if t < -h {
                self.armed = true;
            } else if self.armed && t > h {
                self.ff = -self.ff;
                self.armed = false;
            }
            let down = self.tone_down.process(self.ff * t);

            let dc = mag - self.dc_x + self.dc_r * self.dc_y;
            self.dc_x = mag;
            self.dc_y = dc;
            let up = self.tone_up.process(dc);

            *s = self.dry.next() * x + self.down.next() * down + self.up.next() * up;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            DRY => self.dry.target = value,
            DOWN => self.down.target = value,
            UP => self.up.target = value,
            TONE => {
                self.tone_down.set_freq(value, self.sr);
                self.tone_up.set_freq(value, self.sr);
            }
            TRACK => self.track.set_freq(value, self.sr),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.track.reset();
        self.tone_down.reset();
        self.tone_up.reset();
        self.env = 0.0;
        self.ff = 1.0;
        self.armed = false;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
    }
}

/// Second-order low-pass (RBJ "Audio EQ Cookbook", Q = 1/sqrt(2)), transposed direct form II.
/// Same formulas as `filter.rs`, kept private here to avoid coupling the two blocks.
struct Lowpass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Lowpass {
    fn new(hz: f32, sr: f32) -> Self {
        let mut f = Lowpass { b0: 0.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 };
        f.set_freq(hz, sr);
        f
    }

    /// Recomputes the coefficients. Does not allocate, so it is also safe in `set_param`.
    fn set_freq(&mut self, hz: f32, sr: f32) {
        let f = hz.clamp(10.0, sr * 0.45);
        let (sin, cos) = (TAU * f / sr).sin_cos();
        let alpha = sin / (2.0 * FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - cos) / 2.0 / a0;
        self.b1 = (1.0 - cos) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::Value;

    const SR: f32 = 48000.0;

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (TAU * freq * i as f32 / SR).sin()).collect()
    }

    /// Single-bin DFT magnitude at `freq`, normalized by the length.
    /// The window must contain an integer number of cycles of `freq`.
    fn bin(buf: &[f32], freq: f32) -> f64 {
        let n = buf.len();
        let k = (freq * n as f32 / SR).round() as f64;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &x) in buf.iter().enumerate() {
            let w = std::f64::consts::TAU * k * i as f64 / n as f64;
            re += x as f64 * w.cos();
            im -= x as f64 * w.sin();
        }
        (re * re + im * im).sqrt() / n as f64
    }

    fn build(dry: f32, down: f32, up: f32) -> Box<dyn Block> {
        let mut v = ParamValues::default();
        v.set("dry", Value::Number(dry));
        v.set("down", Value::Number(down));
        v.set("up", Value::Number(up));
        build_octave(&v, SR).unwrap()
    }

    #[test]
    fn octave_down_is_one_octave_lower() {
        let mut o = build(0.0, 1.0, 0.0);
        let mut buf = sine(220.0, 0.5, 14400);
        o.process(&mut buf);
        // skip the first 0.1 s (startup), then 9600 samples = 0.2 s = whole cycles of 110 and 220 Hz
        let win = &buf[4800..];
        let (m110, m220) = (bin(win, 110.0), bin(win, 220.0));
        println!("octave down: |110 Hz| = {m110:.5}, |220 Hz| = {m220:.5}");
        assert!(m110 > m220, "110 Hz {m110} must dominate 220 Hz {m220}");
        assert!(m110 > 0.05, "the 110 Hz voice must be audible: {m110}");
    }

    #[test]
    fn octave_up_is_one_octave_higher() {
        let mut o = build(0.0, 0.0, 1.0);
        let mut buf = sine(220.0, 0.5, 14400);
        o.process(&mut buf);
        let win = &buf[4800..];
        let (m220, m440) = (bin(win, 220.0), bin(win, 440.0));
        println!("octave up: |220 Hz| = {m220:.5}, |440 Hz| = {m440:.5}");
        assert!(m440 > m220, "440 Hz {m440} must dominate 220 Hz {m220}");
        assert!(m440 > 0.05, "the 440 Hz voice must be audible: {m440}");
    }

    #[test]
    fn dry_only_is_passthrough() {
        let mut o = build(1.0, 0.0, 0.0);
        let input = sine(220.0, 0.5, 4800);
        let mut buf = input.clone();
        o.process(&mut buf);
        let max_diff = buf.iter().zip(&input).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(max_diff < 1e-6, "max difference {max_diff}");
    }

    #[test]
    fn white_noise_gives_finite_output() {
        let mut o = build(1.0, 1.0, 1.0);
        let mut seed = 12345u32;
        let mut buf: Vec<f32> = (0..96000)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect();
        o.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak < 10.0, "peak {peak}");
    }

    #[test]
    fn set_param_down_changes_output() {
        let mut o = build(0.0, 0.0, 0.0);
        let mut buf = sine(220.0, 0.5, 4800);
        o.process(&mut buf);
        assert!(buf.iter().all(|s| *s == 0.0), "with all mixes at zero the output is silent");

        o.set_param(DOWN, 1.0);
        let mut buf = sine(220.0, 0.5, 9600);
        o.process(&mut buf);
        let peak = buf[4800..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.1, "down voice must be audible after set_param: {peak}");
    }
}
