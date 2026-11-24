//! Phase 90 / 95 style phaser: first-order all-pass stages swept by a sine LFO.

use std::f32::consts::{LN_10, PI};

use anyhow::Result;

use super::modulation::{Lfo, Wave};
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{Block, Smoothed};

pub const PHASER_DESC: BlockDesc = BlockDesc {
    kind: "phaser",
    label: "Phaser",
    category: Category::Modulation,
    params: &[
        ParamDesc::log("rate_hz", "Rate", 0.05, 10.0, 0.5, "Hz"),
        // 0 = the stages stay at the centre (about 630 Hz), 1 = the full 200 Hz-2 kHz sweep.
        ParamDesc::number("depth", "Depth", 0.0, 1.0, 1.0, ""),
        ParamDesc::number("feedback", "Feedback", 0.0, 0.9, 0.0, ""),
        // 0.5 = equal dry and wet: the classic notches.
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 0.5, ""),
        ParamDesc::choice("stages", "Stages", &["4_stage", "6_stage"], 0),
    ],
};

// Parameter indices in PHASER_DESC.
const RATE: usize = 0;
const DEPTH: usize = 1;
const FEEDBACK: usize = 2;
const MIX: usize = 3;
const STAGES: usize = 4;

/// Geometric centre of the 200 Hz-2 kHz sweep.
const CENTRE_HZ: f32 = 632.5;

/// Flushes denormals (and tiny values) to zero.
#[inline]
pub(crate) fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

/// First-order all-pass section (transposed direct form II).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AllPass {
    z: f32,
}

impl AllPass {
    /// Coefficient for a 90 degree phase shift at `freq` Hz.
    #[inline]
    pub(crate) fn coef(freq: f32, sr: f32) -> f32 {
        let t = (PI * freq.min(0.45 * sr) / sr).tan();
        (t - 1.0) / (t + 1.0)
    }

    #[inline]
    pub(crate) fn tick(&mut self, x: f32, a: f32) -> f32 {
        let y = a * x + self.z;
        self.z = flush(x - a * y);
        y
    }
}

pub fn build_phaser(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = PHASER_DESC.params;
    Ok(Box::new(Phaser {
        sr,
        lfo: Lfo::new(v.number(&p[RATE]), Wave::Sine, sr),
        stages: [AllPass::default(); 6],
        depth: Smoothed::new(v.number(&p[DEPTH]), 15.0, sr),
        feedback: Smoothed::new(v.number(&p[FEEDBACK]), 15.0, sr),
        mix: Smoothed::new(v.number(&p[MIX]), 15.0, sr),
        six: Smoothed::new(v.choice(&p[STAGES]) as f32, 15.0, sr),
        last: 0.0,
    }))
}

struct Phaser {
    sr: f32,
    lfo: Lfo,
    stages: [AllPass; 6],
    depth: Smoothed,
    feedback: Smoothed,
    mix: Smoothed,
    /// 0 = 4 stages, 1 = 6 stages (crossfaded when it changes).
    six: Smoothed,
    last: f32,
}

impl Block for Phaser {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let x = *s;
            let l = self.lfo.next();
            let d = self.depth.next();
            let fb = self.feedback.next();
            let m = self.mix.next();
            let six = self.six.next();
            let a = AllPass::coef(CENTRE_HZ * (0.5 * d * l * LN_10).exp(), self.sr);

            let mut y = x + fb * self.last;
            for st in &mut self.stages[..4] {
                y = st.tick(y, a);
            }
            let y4 = y;
            for st in &mut self.stages[4..] {
                y = st.tick(y, a);
            }
            let wet = y4 + six * (y - y4);
            self.last = flush(wet.tanh());
            *s = x + m * (wet - x);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            RATE => self.lfo.set_rate(value, self.sr),
            DEPTH => self.depth.target = value,
            FEEDBACK => self.feedback.target = value,
            MIX => self.mix.target = value,
            STAGES => self.six.target = if value >= 0.5 { 1.0 } else { 0.0 },
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.stages = [AllPass::default(); 6];
        self.last = 0.0;
    }
}

/// Helpers shared by the tests of the modulation blocks (phaser, univibe, wah).
#[cfg(test)]
pub(crate) mod testkit {
    use crate::{build_block, Block, BlockDesc, ParamKind, ParamValues};

    pub fn sine(freq: f32, amp: f32, n: usize, sr: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / sr).sin()).collect()
    }

    /// Deterministic white noise in [-amp, amp].
    pub fn noise(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut s = seed.wrapping_mul(2654435761).wrapping_add(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                amp * ((s as f32 / u32::MAX as f32) * 2.0 - 1.0)
            })
            .collect()
    }

    pub fn rms(b: &[f32]) -> f32 {
        (b.iter().map(|s| s * s).sum::<f32>() / b.len() as f32).sqrt()
    }

    /// Magnitude of the component at `freq` (Goertzel via direct correlation).
    pub fn tone(b: &[f32], freq: f32, sr: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, s) in b.iter().enumerate() {
            let w = std::f64::consts::TAU * freq as f64 * i as f64 / sr as f64;
            re += *s as f64 * w.cos();
            im += *s as f64 * w.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / b.len() as f64) as f32
    }

    pub fn make(kind: &str, params: &str, sr: f32) -> Box<dyn Block> {
        let values: ParamValues = toml::from_str(params).unwrap();
        build_block(kind, &values, sr).unwrap()
    }

    fn desc(kind: &str) -> &'static BlockDesc {
        crate::find(kind).unwrap().desc
    }

    fn extremes(kind: &str) -> Vec<(usize, f32, f32)> {
        desc(kind)
            .params
            .iter()
            .enumerate()
            .filter_map(|(i, p)| match p.kind {
                ParamKind::Number { min, max, .. } => Some((i, min, max)),
                ParamKind::Choice { options, .. } => Some((i, 0.0, (options.len() - 1) as f32)),
                ParamKind::File { .. } => None,
            })
            .collect()
    }

    /// Noise and impulses, built and live-swept over the extremes of every parameter: finite and bounded.
    pub fn check_finite_and_bounded(kind: &str, sr: f32, bound: f32) {
        let mut input = noise(9600, 0.8, 7);
        for i in (0..input.len()).step_by(1000) {
            input[i] = 1.0;
        }
        for (i, lo, hi) in extremes(kind) {
            for v in [lo, hi] {
                let mut b = make(kind, "", sr);
                b.set_param(i, v);
                let mut buf = input.clone();
                b.process(&mut buf);
                assert!(buf.iter().all(|s| s.is_finite() && s.abs() < bound), "{kind} param {i}={v}");
            }
        }
        // All parameters at their maximum, then all at their minimum, with no rebuild.
        let mut b = make(kind, "", sr);
        for pass in 0..2 {
            for (i, lo, hi) in extremes(kind) {
                b.set_param(i, if pass == 0 { hi } else { lo });
            }
            let mut buf = input.clone();
            b.process(&mut buf);
            assert!(buf.iter().all(|s| s.is_finite() && s.abs() < bound), "{kind} all-extremes pass {pass}");
        }
        // Silence stays finite (denormals).
        let mut buf = vec![0.0; 4800];
        b.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
    }

    /// Changing any numeric parameter live must not make the output jump.
    pub fn check_live_no_jumps(kind: &str, sr: f32, params: &str, max_step: f32) {
        let input = sine(220.0, 0.3, 24000, sr);
        for (i, lo, hi) in extremes(kind) {
            if matches!(desc(kind).params[i].kind, ParamKind::Choice { .. }) {
                continue;
            }
            for v in [hi, lo] {
                let mut b = make(kind, params, sr);
                let mut warm = input[..12000].to_vec();
                b.process(&mut warm);
                b.set_param(i, v);
                let mut buf = input[12000..].to_vec();
                b.process(&mut buf);
                let step = buf.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
                let prev = warm[warm.len() - 1];
                let first = (buf[0] - prev).abs();
                assert!(step < max_step && first < max_step, "{kind} param {i}->{v}: step {step}, first {first}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;

    const SR: f32 = 48000.0;

    #[test]
    fn finite_and_bounded() {
        check_finite_and_bounded("phaser", SR, 12.0);
        check_finite_and_bounded("phaser", 44100.0, 12.0);
    }

    #[test]
    fn live_params_do_not_jump() {
        check_live_no_jumps("phaser", SR, "feedback = 0.3", 0.3);
        // Also with the stage count switched live.
        let mut b = make("phaser", "", SR);
        let mut warm = sine(220.0, 0.3, 12000, SR);
        b.process(&mut warm);
        b.set_param(STAGES, 1.0);
        let mut buf = sine(220.0, 0.3, 12000, SR);
        b.process(&mut buf);
        let step = buf.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(step < 0.3, "{step}");
    }

    #[test]
    fn notches_move() {
        // A 630 Hz tone is notched when the sweep passes the matching stage frequency, and passes otherwise.
        let mut b = make("phaser", "rate_hz = 1\ndepth = 1\nmix = 0.5", SR);
        let mut buf = sine(630.0, 0.5, 96000, SR);
        b.process(&mut buf);
        let levels: Vec<f32> = buf.chunks(480).skip(10).map(rms).collect();
        let min = levels.iter().cloned().fold(f32::MAX, f32::min);
        let max = levels.iter().cloned().fold(0.0, f32::max);
        assert!(min < 0.2 * max, "min {min}, max {max}");
    }

    #[test]
    fn notch_depth_is_deep_at_mix_half() {
        // Static centre sweep (depth 0): the notch of 4 stages sits at about 0.41 * 632 Hz.
        let mut b = make("phaser", "depth = 0\nmix = 0.5", SR);
        let mut n = noise(48000, 0.5, 3);
        b.process(&mut n);
        let notch = tone(&n[4800..], 260.0, SR);
        let off = tone(&n[4800..], 1000.0, SR);
        assert!(notch < 0.4 * off, "{notch} vs {off}");
    }

    #[test]
    fn six_stages_have_a_second_notch() {
        // Dry/wet at 0.5: 4 stages notch once below 2.4 * fc... 6 stages add one more above.
        let mut four = make("phaser", "depth = 0\nstages = \"4_stage\"", SR);
        let mut six = make("phaser", "depth = 0\nstages = \"6_stage\"", SR);
        let mut a = noise(48000, 0.5, 5);
        let mut b = a.clone();
        four.process(&mut a);
        six.process(&mut b);
        // Where the 4-stage response is not a notch, the 6-stage one differs.
        assert!((tone(&a[4800..], 700.0, SR) - tone(&b[4800..], 700.0, SR)).abs() > 0.001);
    }

    #[test]
    fn sample_rate_independent() {
        let run = |sr: f32| {
            let mut b = make("phaser", "rate_hz = 2\nfeedback = 0.5", sr);
            let mut buf = noise((sr * 2.0) as usize, 0.5, 11);
            b.process(&mut buf);
            let n = buf.len() / 2;
            (tone(&buf[n..], 500.0, sr), rms(&buf[n..]))
        };
        let (_, r1) = run(44100.0);
        let (_, r2) = run(48000.0);
        assert!((r1 / r2 - 1.0).abs() < 0.1, "{r1} vs {r2}");
    }

    #[test]
    fn feedback_stays_stable() {
        let mut b = make("phaser", "feedback = 0.9\nmix = 1", SR);
        let mut buf = vec![0.0; 48000];
        buf[0] = 1.0;
        b.process(&mut buf);
        assert!(rms(&buf[24000..]) < 0.01);
    }
}
