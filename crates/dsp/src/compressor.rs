//! Feed-forward compressor (task D2, see docs/PLAN.md §7).

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, lin_to_db, one_pole_coef, Block, Smoothed};

pub const COMPRESSOR_DESC: BlockDesc = BlockDesc {
    kind: "compressor",
    label: "Compressor",
    category: Category::Dynamics,
    params: &[
        ParamDesc::number("threshold_db", "Threshold", -60.0, 0.0, -20.0, "dB"),
        ParamDesc::log("ratio", "Ratio", 1.0, 20.0, 4.0, ":1"),
        ParamDesc::log("attack_ms", "Attack", 0.1, 100.0, 5.0, "ms"),
        ParamDesc::log("release_ms", "Release", 10.0, 1000.0, 120.0, "ms"),
        ParamDesc::number("knee_db", "Knee", 0.0, 12.0, 6.0, "dB"),
        ParamDesc::number("makeup_db", "Gain", 0.0, 24.0, 0.0, "dB"),
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 1.0, ""),
    ],
};

const THRESHOLD: usize = 0;
const RATIO: usize = 1;
const ATTACK: usize = 2;
const RELEASE: usize = 3;
const KNEE: usize = 4;
const MAKEUP: usize = 5;
const MIX: usize = 6;

/// Smoothing time for output gain and mix, to avoid clicks.
const LEVEL_SMOOTH_MS: f32 = 15.0;

pub fn build_compressor(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = COMPRESSOR_DESC.params;
    Ok(Box::new(Compressor {
        sr,
        threshold_db: v.number(&p[THRESHOLD]),
        ratio: v.number(&p[RATIO]),
        knee_db: v.number(&p[KNEE]),
        attack_coef: one_pole_coef(v.number(&p[ATTACK]), sr),
        release_coef: one_pole_coef(v.number(&p[RELEASE]), sr),
        g_s: 0.0,
        makeup: Smoothed::new(db_to_lin(v.number(&p[MAKEUP])), LEVEL_SMOOTH_MS, sr),
        mix: Smoothed::new(v.number(&p[MIX]), LEVEL_SMOOTH_MS, sr),
    }))
}

pub struct Compressor {
    /// Needed to recompute the coefficients in `set_param`; in the binary it is not yet
    /// read by anyone, so the dead-code lint has to be silenced here.
    #[allow(dead_code)]
    sr: f32,
    threshold_db: f32,
    ratio: f32,
    knee_db: f32,
    attack_coef: f32,
    release_coef: f32,
    /// Smoothed gain reduction, in dB (always <= 0).
    g_s: f32,
    /// Smoothed output gain (linear).
    makeup: Smoothed,
    mix: Smoothed,
}

impl Compressor {
    /// Gain reduction in dB for an input level `x_db` (gain computer
    /// with soft knee, Giannoulis-Massberg-Reiss 2012). The result is <= 0.
    #[inline]
    fn gain_db(&self, x_db: f32) -> f32 {
        let over = x_db - self.threshold_db;
        let w = self.knee_db;
        let slope = 1.0 / self.ratio - 1.0;
        if 2.0 * over < -w {
            // Below the knee: no reduction.
            0.0
        } else if w > 0.0 && 2.0 * over.abs() <= w {
            // Inside the knee: quadratic transition. With w = 0 this is never reached.
            slope * (over + w / 2.0).powi(2) / (2.0 * w)
        } else {
            // Above the knee (with w = 0, also the hard cut): slope 1/R.
            slope * over
        }
    }
}

impl Block for Compressor {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let x = *s;
            let g = self.gain_db(lin_to_db(x.abs()));
            // Attack when the reduction must grow, release when it must shrink.
            let coef = if g < self.g_s { self.attack_coef } else { self.release_coef };
            self.g_s = coef * self.g_s + (1.0 - coef) * g;
            let wet = x * db_to_lin(self.g_s) * self.makeup.next();
            let mix = self.mix.next();
            *s = mix * wet + (1.0 - mix) * x;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            THRESHOLD => self.threshold_db = value,
            RATIO => self.ratio = value,
            ATTACK => self.attack_coef = one_pole_coef(value, self.sr),
            RELEASE => self.release_coef = one_pole_coef(value, self.sr),
            KNEE => self.knee_db = value,
            MAKEUP => self.makeup.target = db_to_lin(value),
            MIX => self.mix.target = value,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.g_s = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    const SR: f32 = 48000.0;

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    fn values(pairs: &[(&str, f32)]) -> ParamValues {
        let mut v = ParamValues::default();
        for (id, x) in pairs {
            v.set(id, Value::Number(*x));
        }
        v
    }

    fn build(pairs: &[(&str, f32)]) -> Box<dyn Block> {
        build_compressor(&values(pairs), SR).unwrap()
    }

    fn peak_db(buf: &[f32]) -> f32 {
        lin_to_db(peak(buf))
    }

    #[test]
    fn above_threshold_reduces_by_ratio() {
        // Peak -10 dBFS, threshold -20, ratio 4, knee 0: expected -17.5 dBFS.
        // With the default attack (5 ms) the per-sample detector makes g_s oscillate
        // and the output sits at about -16.9 dBFS: here the minimum attack isolates the law.
        let mut c = build(&[("threshold_db", -20.0), ("ratio", 4.0), ("knee_db", 0.0), ("attack_ms", 0.1)]);
        let amp = db_to_lin(-10.0);
        let mut buf = sine(1000.0, amp, 24000);
        c.process(&mut buf);
        let out = peak_db(&buf[19200..]);
        assert!((out - -17.5).abs() <= 0.5, "output {out} dBFS");
    }

    #[test]
    fn below_threshold_changes_nothing() {
        let mut c = build(&[("threshold_db", -20.0), ("ratio", 4.0), ("knee_db", 0.0)]);
        let input = sine(1000.0, db_to_lin(-40.0), 24000);
        let mut buf = input.clone();
        c.process(&mut buf);
        let diff = (peak_db(&buf[19200..]) - peak_db(&input[19200..])).abs();
        assert!(diff <= 0.1, "difference {diff} dB");
    }

    #[test]
    fn mix_zero_is_the_original_signal() {
        let mut c = build(&[("threshold_db", -20.0), ("ratio", 4.0), ("knee_db", 0.0)]);
        c.set_param(MIX, 0.0);
        let input = sine(1000.0, db_to_lin(-6.0), 24000);
        let mut buf = input.clone();
        c.process(&mut buf);
        // After the initial smoothing (15 ms) the output matches the input.
        for (i, (o, x)) in buf.iter().zip(&input).enumerate().skip(9600) {
            assert!((o - x).abs() < 1e-4, "sample {i}: {o} != {x}");
        }
    }

    #[test]
    fn set_param_threshold_changes_level() {
        let mut c = build(&[("threshold_db", -20.0), ("ratio", 4.0), ("knee_db", 0.0)]);
        let input = sine(1000.0, db_to_lin(-10.0), 24000);
        let mut buf = input.clone();
        c.process(&mut buf);
        let before = peak_db(&buf[19200..]);

        c.set_param(THRESHOLD, -30.0);
        let mut buf = input.clone();
        c.process(&mut buf);
        let after = peak_db(&buf[19200..]);

        assert!(before - after > 3.0, "before {before} dBFS, after {after} dBFS");
    }
}
