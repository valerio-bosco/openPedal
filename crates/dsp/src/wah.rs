//! Wah: resonant band-pass (state-variable filter), fixed position or driven by an envelope follower.

use std::f32::consts::PI;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::phaser::flush;
use super::{db_to_lin, one_pole_coef, Block, Smoothed};

pub const WAH_DESC: BlockDesc = BlockDesc {
    kind: "wah",
    label: "Wah",
    category: Category::Filter,
    params: &[
        // Centre frequency at position 0.5. The position sweeps +-1.33 octaves around it, so with the
        // default the pedal goes from 350 Hz (heel) to 2.2 kHz (toe).
        ParamDesc::log("freq", "Freq", 350.0, 2200.0, 877.0, "Hz"),
        ParamDesc::number("q", "Q", 2.0, 12.0, 5.0, ""),
        ParamDesc::choice("mode", "Mode", &["fixed", "auto"], 0),
        // Pedal position in "fixed" mode: 0 = heel, 1 = toe.
        ParamDesc::number("position", "Position", 0.0, 1.0, 0.5, ""),
        // Auto mode: how strongly the picking level opens the filter.
        ParamDesc::number("sensitivity", "Sensitivity", 0.0, 1.0, 0.5, ""),
        ParamDesc::log("attack_ms", "Attack", 1.0, 100.0, 8.0, "ms"),
        ParamDesc::log("release_ms", "Release", 20.0, 1000.0, 200.0, "ms"),
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 1.0, ""),
        ParamDesc::number("level_db", "Level", -24.0, 12.0, 0.0, "dB"),
    ],
};

// Parameter indices in WAH_DESC.
const FREQ: usize = 0;
const Q: usize = 1;
const MODE: usize = 2;
const POSITION: usize = 3;
const SENSITIVITY: usize = 4;
const ATTACK: usize = 5;
const RELEASE: usize = 6;
const MIX: usize = 7;
const LEVEL: usize = 8;

/// log2(2200 / 350): the sweep of the pedal in octaves.
const SWEEP_OCTAVES: f32 = 2.652;
/// Gain from envelope times sensitivity to position (a hard pick is about 0.3).
const ENV_TO_POSITION: f32 = 12.0;

pub fn build_wah(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = WAH_DESC.params;
    let pos = v.number(&p[POSITION]);
    Ok(Box::new(Wah {
        sr,
        freq: Smoothed::new(v.number(&p[FREQ]), 15.0, sr),
        q: Smoothed::new(v.number(&p[Q]), 15.0, sr),
        auto: v.choice(&p[MODE]) == 1,
        position: pos,
        pos_target: Smoothed::new(pos, 10.0, sr),
        sensitivity: v.number(&p[SENSITIVITY]),
        att: one_pole_coef(v.number(&p[ATTACK]), sr),
        rel: one_pole_coef(v.number(&p[RELEASE]), sr),
        env: 0.0,
        pos_smooth: pos,
        pos_coef: one_pole_coef(5.0, sr),
        mix: Smoothed::new(v.number(&p[MIX]), 15.0, sr),
        level: Smoothed::new(db_to_lin(v.number(&p[LEVEL])), 15.0, sr),
        ic1: 0.0,
        ic2: 0.0,
    }))
}

struct Wah {
    sr: f32,
    freq: Smoothed,
    q: Smoothed,
    auto: bool,
    /// Fixed position as set by the parameter (smoothed by `pos_target`).
    position: f32,
    pos_target: Smoothed,
    sensitivity: f32,
    att: f32,
    rel: f32,
    env: f32,
    /// Position actually used (also smoothed when switching mode).
    pos_smooth: f32,
    pos_coef: f32,
    mix: Smoothed,
    level: Smoothed,
    ic1: f32,
    ic2: f32,
}

impl Block for Wah {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let x = *s;
            let a = x.abs();
            let c = if a > self.env { self.att } else { self.rel };
            self.env = flush(a + (self.env - a) * c);

            self.pos_target.target = self.position;
            let target = if self.auto {
                (self.env * self.sensitivity * ENV_TO_POSITION).min(1.0)
            } else {
                self.pos_target.next()
            };
            self.pos_smooth = target + (self.pos_smooth - target) * self.pos_coef;

            let f = self.freq.next() * ((self.pos_smooth - 0.5) * SWEEP_OCTAVES).exp2();
            let k = 1.0 / self.q.next();
            let g = (PI * f.min(0.45 * self.sr) / self.sr).tan();
            let a1 = 1.0 / (1.0 + g * (g + k));
            let a2 = g * a1;
            let a3 = g * a2;
            let v3 = x - self.ic2;
            let v1 = a1 * self.ic1 + a2 * v3;
            let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
            self.ic1 = flush(2.0 * v1 - self.ic1);
            self.ic2 = flush(2.0 * v2 - self.ic2);

            // k * band-pass: unity gain at the centre frequency.
            let wet = k * v1;
            let m = self.mix.next();
            *s = (x + m * (wet - x)) * self.level.next();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            FREQ => self.freq.target = value,
            Q => self.q.target = value,
            MODE => self.auto = value >= 0.5,
            POSITION => self.position = value,
            SENSITIVITY => self.sensitivity = value,
            ATTACK => self.att = one_pole_coef(value, self.sr),
            RELEASE => self.rel = one_pole_coef(value, self.sr),
            MIX => self.mix.target = value,
            LEVEL => self.level.target = db_to_lin(value),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::phaser::testkit::*;

    const SR: f32 = 48000.0;

    #[test]
    fn finite_and_bounded() {
        check_finite_and_bounded("wah", SR, 20.0);
        check_finite_and_bounded("wah", 44100.0, 20.0);
    }

    #[test]
    fn live_params_do_not_jump() {
        check_live_no_jumps("wah", SR, "", 0.4);
        check_live_no_jumps("wah", SR, "mode = \"auto\"", 0.4);
    }

    #[test]
    fn peaks_near_its_frequency() {
        // Position 0.5 puts the peak at `freq`; 0 and 1 at 350 Hz and 2.2 kHz.
        for (pos, hz) in [(0.0, 350.0f32), (0.5, 877.0), (1.0, 2200.0)] {
            let mut b = make("wah", &format!("position = {pos}\nq = 8"), SR);
            let mut buf = noise(96000, 0.5, 9);
            b.process(&mut buf);
            let at = tone(&buf[9600..], hz, SR);
            for other in [hz / 3.0, hz * 3.0] {
                assert!(at > 3.0 * tone(&buf[9600..], other, SR), "pos {pos}: {at} vs {other}");
            }
            // Peak is not far from the others around it either.
            assert!(at > 1.3 * tone(&buf[9600..], hz * 1.5, SR));
            assert!(at > 1.3 * tone(&buf[9600..], hz / 1.5, SR));
        }
    }

    #[test]
    fn higher_q_is_narrower() {
        let width = |q: f32| {
            let mut b = make("wah", &format!("q = {q}"), SR);
            let mut buf = noise(96000, 0.5, 9);
            b.process(&mut buf);
            tone(&buf[9600..], 877.0 * 1.3, SR) / tone(&buf[9600..], 877.0, SR)
        };
        assert!(width(12.0) < width(2.0));
    }

    #[test]
    fn auto_opens_with_the_picking_level() {
        let high_over_low = |amp: f32| {
            let mut b = make("wah", "mode = \"auto\"\nsensitivity = 0.6\nq = 6", SR);
            let mut buf = noise(48000, amp, 1);
            b.process(&mut buf);
            tone(&buf[9600..], 2000.0, SR) / tone(&buf[9600..], 400.0, SR)
        };
        assert!(high_over_low(0.5) > 2.0 * high_over_low(0.02));
    }

    #[test]
    fn mix_zero_is_dry_and_level_applies() {
        let mut b = make("wah", "mix = 0\nlevel_db = -6", SR);
        let mut buf = sine(300.0, 0.5, 24000, SR);
        b.process(&mut buf);
        assert!((rms(&buf[4800..]) / rms(&sine(300.0, 0.5, 19200, SR)) - 0.501).abs() < 0.02);
    }

    #[test]
    fn sample_rate_independent() {
        for hz in [500.0f32, 877.0, 1300.0, 3000.0] {
            let run = |sr: f32| {
                let mut b = make("wah", "position = 0.7", sr);
                let mut buf = sine(hz, 0.5, sr as usize, sr);
                b.process(&mut buf);
                rms(&buf[buf.len() / 2..])
            };
            let (a, b) = (run(44100.0), run(48000.0));
            assert!((a / b - 1.0).abs() < 0.05, "{hz}: {a} vs {b}");
        }
    }
}
