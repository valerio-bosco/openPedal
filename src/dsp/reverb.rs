//! Mono Freeverb reverb (task D3, see docs/PLAN.md §7).

use std::f32::consts::TAU;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{Block, Smoothed};

pub const REVERB_DESC: BlockDesc = BlockDesc {
    kind: "reverb",
    label: "Reverb",
    category: Category::Time,
    params: &[
        ParamDesc::number("room", "Room", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("damp", "Damping", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 0.25, ""),
        ParamDesc::number("predelay_ms", "Pre-delay", 0.0, 100.0, 10.0, "ms"),
        ParamDesc::log("width_hz", "Tone", 2000.0, 12000.0, 8000.0, "Hz"),
    ],
};

const ROOM: usize = 0;
const DAMP: usize = 1;
const MIX: usize = 2;
const PREDELAY: usize = 3;
const WIDTH: usize = 4;

/// Original Freeverb lengths at 44.1 kHz (samples).
const COMB_LENGTHS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LENGTHS: [usize; 4] = [556, 441, 341, 225];
const SAMPLE_RATE_REF: f32 = 44100.0;

/// Input gain of the reverb branch (as in Freeverb).
const FIXED_GAIN: f32 = 0.015;
/// The reverb branch comes out much quieter than the dry signal: as in Freeverb it is scaled by 3.
const WET_SCALE: f32 = 3.0;
const ALLPASS_FEEDBACK: f32 = 0.5;
const PREDELAY_MAX_MS: f32 = 100.0;
const SMOOTH_MS: f32 = 15.0;

fn scale_len(len: usize, sr: f32) -> usize {
    ((len as f32) * sr / SAMPLE_RATE_REF).round().max(1.0) as usize
}

/// Pre-delay line: like `DelayLine` (fractional reads), but can be cleared without reallocating.
struct PreDelay {
    buf: Vec<f32>,
    write: usize,
}

impl PreDelay {
    fn new(max_samples: usize) -> Self {
        Self { buf: vec![0.0; max_samples.max(4)], write: 0 }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) % self.buf.len();
    }

    /// Sample from `delay` samples ago; must be read before `push` (minimum delay 1).
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let delay = delay.clamp(1.0, (len - 2) as f32);
        let pos = self.write as f32 + len as f32 - delay;
        let i = pos as usize;
        let frac = pos - i as f32;
        let a = self.buf[i % len];
        let b = self.buf[(i + 1) % len];
        a + (b - a) * frac
    }
}

/// Comb filter with a low-pass in the feedback (damping).
struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
    feedback: f32,
    damp1: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len], idx: 0, store: 0.0, feedback: 0.0, damp1: 0.0 }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - self.damp1) + self.store * self.damp1;
        self.buf[self.idx] = input + self.store * self.feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

/// Diffusion allpass filter.
struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len], idx: 0 }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let bufout = self.buf[self.idx];
        let out = -input + bufout;
        self.buf[self.idx] = input + bufout * ALLPASS_FEEDBACK;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

pub fn build_reverb(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = REVERB_DESC.params;
    // The pre-delay is allocated for the maximum (100 ms).
    let mut reverb = Reverb {
        predelay: PreDelay::new((PREDELAY_MAX_MS * 0.001 * sr).ceil() as usize),
        pre: Smoothed::new(v.number(&p[PREDELAY]) * 0.001 * sr, SMOOTH_MS, sr),
        combs: COMB_LENGTHS.iter().map(|&n| Comb::new(scale_len(n, sr))).collect(),
        allpasses: ALLPASS_LENGTHS.iter().map(|&n| Allpass::new(scale_len(n, sr))).collect(),
        lp: 0.0,
        lp_coef: 0.0,
        sr,
        mix: Smoothed::new(v.number(&p[MIX]), SMOOTH_MS, sr),
    };
    reverb.set_param(ROOM, v.number(&p[ROOM]));
    reverb.set_param(DAMP, v.number(&p[DAMP]));
    reverb.set_param(WIDTH, v.number(&p[WIDTH]));
    Ok(Box::new(reverb))
}

pub struct Reverb {
    predelay: PreDelay,
    /// Pre-delay in samples.
    pre: Smoothed,
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
    /// Final low-pass (reverb tone).
    lp: f32,
    lp_coef: f32,
    sr: f32,
    mix: Smoothed,
}

impl Block for Reverb {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let dry = *s;
            // Read before write: the minimum delay is 1 sample.
            let early = self.predelay.read(self.pre.next());
            self.predelay.push(dry);

            let input = early * FIXED_GAIN;
            let mut wet = 0.0;
            for comb in &mut self.combs {
                wet += comb.process(input);
            }
            for allpass in &mut self.allpasses {
                wet = allpass.process(wet);
            }
            self.lp = wet + (self.lp - wet) * self.lp_coef;
            let wet = self.lp * WET_SCALE;

            let mix = self.mix.next();
            *s = dry * (1.0 - mix) + wet * mix;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            ROOM => {
                let feedback = value * 0.28 + 0.7;
                for comb in &mut self.combs {
                    comb.feedback = feedback;
                }
            }
            DAMP => {
                let damp1 = value * 0.4;
                for comb in &mut self.combs {
                    comb.damp1 = damp1;
                }
            }
            MIX => self.mix.target = value,
            PREDELAY => self.pre.target = value * 0.001 * self.sr,
            WIDTH => self.lp_coef = (-TAU * value / self.sr).exp(),
            _ => {}
        }
    }

    fn reset(&mut self) {
        // Fills the existing buffers with zeros: no reallocation.
        self.predelay.buf.fill(0.0);
        for comb in &mut self.combs {
            comb.buf.fill(0.0);
            comb.idx = 0;
            comb.store = 0.0;
        }
        for allpass in &mut self.allpasses {
            allpass.buf.fill(0.0);
            allpass.idx = 0;
        }
        self.lp = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::params::Value;

    const SR: f32 = 44100.0;

    fn build(room: f32, damp: f32, mix: f32, predelay_ms: f32, width_hz: f32) -> Box<dyn Block> {
        let mut v = ParamValues::default();
        let p = REVERB_DESC.params;
        for (desc, x) in p.iter().zip([room, damp, mix, predelay_ms, width_hz]) {
            v.set(desc.id, Value::Number(x));
        }
        build_reverb(&v, SR).unwrap()
    }

    /// Impulse response (mix = 1, so only the reverb is heard).
    fn impulse_response(block: &mut dyn Block, secs: f32) -> Vec<f32> {
        let mut buf = vec![0.0; (secs * SR) as usize];
        buf[0] = 1.0;
        block.process(&mut buf);
        buf
    }

    fn energy(buf: &[f32], from_s: f32, to_s: f32) -> f32 {
        buf[(from_s * SR) as usize..(to_s * SR) as usize].iter().map(|s| s * s).sum()
    }

    #[test]
    fn impulse_tail_decays() {
        let mut block = build(0.5, 0.5, 1.0, 0.0, 8000.0);
        let out = impulse_response(block.as_mut(), 1.6);
        let early = energy(&out, 0.1, 0.6);
        let late = energy(&out, 1.0, 1.5);
        assert!(early > 0.0, "no tail");
        assert!(late < early, "the tail does not decay: late {late}, early {early}");
    }

    #[test]
    fn mix_zero_is_the_dry_signal() {
        let input: Vec<f32> = (0..4800).map(|i| 0.5 * (TAU * 220.0 * i as f32 / SR).sin()).collect();
        let mut block = build(0.5, 0.5, 0.0, 10.0, 8000.0);
        let mut out = input.clone();
        block.process(&mut out);
        let max_diff = input.iter().zip(&out).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(max_diff < 1e-6, "difference {max_diff}");
    }

    #[test]
    fn full_scale_noise_produces_no_nan() {
        let mut seed: u32 = 12345;
        let mut buf: Vec<f32> = (0..(10.0 * SR) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / 16_777_216.0 * 2.0 - 1.0
            })
            .collect();
        let mut block = build(1.0, 0.5, 1.0, 10.0, 8000.0);
        for chunk in buf.chunks_mut(512) {
            block.process(chunk);
        }
        assert!(buf.iter().all(|s| s.is_finite()), "NaN or infinite values");
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak.is_finite(), "peak {peak}");
    }

    #[test]
    fn set_param_room_lengthens_tail() {
        let mut short = build(0.0, 0.5, 1.0, 0.0, 8000.0);
        let mut changed = build(0.0, 0.5, 1.0, 0.0, 8000.0);
        changed.set_param(ROOM, 1.0);
        let e_short = energy(&impulse_response(short.as_mut(), 1.6), 1.0, 1.5);
        let e_changed = energy(&impulse_response(changed.as_mut(), 1.6), 1.0, 1.5);
        assert!(e_changed > 100.0 * e_short, "room does not change the tail: {e_short} -> {e_changed}");
    }
}
