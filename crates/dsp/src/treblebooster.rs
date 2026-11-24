//! Single-transistor germanium treble booster in the spirit of the Dallas Rangemaster.
//!
//! Model: a shelf (low band attenuated by `a`, everything above the corner passes: the small
//! input capacitor), then a gain of +6..+30 dB from `boost`, then a mild asymmetric soft clipper
//! at 4x oversampling (`tanh(0.8 z + b) - tanh(b)`), a gentle top roll-off and `level_db`.
//! `range` moves the shelf: treble (corner 1.8 kHz, bass cut hard), mid (700 Hz), full (180 Hz,
//! bass mostly kept). All gains and the shelf are smoothed (15 ms), so every parameter is live.

use std::f32::consts::TAU;

use anyhow::Result;

use super::oversample::Oversampler4;
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, Block, Smoothed};

pub const TREBLEBOOSTER_DESC: BlockDesc = BlockDesc {
    kind: "treblebooster",
    label: "Treble Booster",
    category: Category::Drive,
    params: &[
        ParamDesc::choice("range", "Range", &["treble", "mid", "full"], 0),
        ParamDesc::number("boost", "Boost", 0.0, 1.0, 0.6, ""),
        ParamDesc::number("level_db", "Level", -30.0, 12.0, -6.0, "dB"),
    ],
};

const RANGE: usize = 0;
const BOOST: usize = 1;
const LEVEL: usize = 2;
const SMOOTH_MS: f32 = 15.0;

/// (corner Hz, gain left below the corner)
fn shelf(range: usize) -> (f32, f32) {
    match range {
        0 => (1800.0, 0.1),
        1 => (700.0, 0.25),
        _ => (180.0, 0.55),
    }
}

fn hp_pole(hz: f32, sr: f32) -> f32 {
    (-TAU * hz / sr).exp()
}

fn boost_gain(b: f32) -> f32 {
    db_to_lin(6.0 + 24.0 * b)
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
    gain: Smoothed,
}

impl Core {
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let b = 0.2;
        let z = self.gain.next() * x;
        // 0.8 = headroom factor, the small-signal gain is then 1 * (1 - tanh(b)^2) ~ 0.96
        ((0.8 * z + b).tanh() - b.tanh()) * (1.0 / 0.8)
    }
}

pub fn build_treblebooster(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = TREBLEBOOSTER_DESC.params;
    let (hz, a) = shelf(v.choice(&p[RANGE]));
    Ok(Box::new(TrebleBooster {
        core: Core { gain: Smoothed::new(boost_gain(v.number(&p[BOOST])), SMOOTH_MS, 4.0 * sr) },
        os: Oversampler4::new(),
        pole: Smoothed::new(hp_pole(hz, sr), SMOOTH_MS, sr),
        low: Smoothed::new(a, SMOOTH_MS, sr),
        hp_x: 0.0,
        hp_y: 0.0,
        lp: 0.0,
        lp_a: 1.0 - (-TAU * 9000.0 / sr).exp(),
        post: Smoothed::new(db_to_lin(v.number(&p[LEVEL])), SMOOTH_MS, sr),
        sr,
    }))
}

pub struct TrebleBooster {
    core: Core,
    os: Oversampler4,
    pole: Smoothed,
    low: Smoothed,
    hp_x: f32,
    hp_y: f32,
    lp: f32,
    lp_a: f32,
    post: Smoothed,
    sr: f32,
}

impl Block for TrebleBooster {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let x = *s;
            let pole = self.pole.next();
            // one-pole high-pass y[n] = pole * (y[n-1] + x[n] - x[n-1]); shelf = a*x + (1-a)*hp
            let hp = flush(pole * (self.hp_y + x - self.hp_x));
            self.hp_x = x;
            self.hp_y = hp;
            let a = self.low.next();
            *s = a * x + (1.0 - a) * hp;
        }
        let core = &mut self.core;
        self.os.process(buf, |x| core.tick(x));
        for s in buf.iter_mut() {
            self.lp = flush(self.lp + self.lp_a * (*s - self.lp));
            *s = self.lp * self.post.next();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            RANGE => {
                let (hz, a) = shelf(value.round().clamp(0.0, 2.0) as usize);
                self.pole.target = hp_pole(hz, self.sr);
                self.low.target = a;
            }
            BOOST => self.core.gain.target = boost_gain(value),
            LEVEL => self.post.target = db_to_lin(value),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.os.reset();
        self.hp_x = 0.0;
        self.hp_y = 0.0;
        self.lp = 0.0;
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
        build_treblebooster(&v, SR).unwrap()
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn rms(b: &[f32]) -> f32 {
        (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt()
    }

    fn gain_db(params: &[(&str, f32)], freq: f32) -> f32 {
        let mut f = build(params);
        let mut buf = sine(freq, 0.005, 24000);
        f.process(&mut buf);
        20.0 * (rms(&buf[12000..]) / (0.005 / 2f32.sqrt())).log10()
    }

    #[test]
    fn boosts_treble_more_than_bass() {
        for range in [0.0, 1.0, 2.0] {
            let p = [("range", range)];
            let (lo, hi) = (gain_db(&p, 200.0), gain_db(&p, 3000.0));
            println!("range {range}: 200 Hz {lo:.1} dB, 3 kHz {hi:.1} dB");
            let min = if range < 2.0 { 3.0 } else { 0.5 };
            assert!(hi > lo + min, "{lo} {hi}");
        }
        // treble range cuts more bass than full
        assert!(gain_db(&[("range", 0.0)], 200.0) < gain_db(&[("range", 2.0)], 200.0) - 6.0);
    }

    #[test]
    fn stays_finite_and_bounded_over_sweeps() {
        for range in [0.0, 1.0, 2.0] {
            for boost in [0.0, 0.5, 1.0] {
                let mut f = build(&[("range", range), ("boost", boost), ("level_db", 12.0)]);
                let mut s = 1u32;
                let mut buf: Vec<f32> = (0..9600)
                    .map(|_| {
                        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                        (s >> 8) as f32 / 8388608.0 - 1.0
                    })
                    .collect();
                buf[50] = 1.0;
                buf[51] = -1.0;
                buf.extend(sine(3000.0, 1.0, 4800));
                buf.extend(vec![0.0; 4800]);
                f.process(&mut buf);
                let p = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                assert!(buf.iter().all(|s| s.is_finite()));
                assert!(p < 8.0, "{range} {boost}: {p}");
            }
        }
    }

    #[test]
    fn pushed_hard_it_clips_asymmetrically() {
        let mut f = build(&[("boost", 1.0), ("range", 2.0)]);
        let mut buf = sine(1000.0, 0.5, 9600);
        f.process(&mut buf);
        let b = &buf[4800..];
        let (mx, mn) = b.iter().fold((0.0f32, 0.0f32), |(a, c), s| (a.max(*s), c.min(*s)));
        println!("asymmetry: +{mx:.3} / {mn:.3}");
        assert!((mx + mn).abs() > 0.01 * mx);
    }

    #[test]
    fn live_set_param_does_not_click() {
        for (idx, val) in [(RANGE, 2.0), (BOOST, 0.0), (LEVEL, -30.0)] {
            let mut f = build(&[]);
            let mut warm = sine(1000.0, 0.1, 9600);
            f.process(&mut warm);
            f.set_param(idx, val);
            let mut buf: Vec<f32> = (0..960).map(|i| 0.1 * (TAU * 1000.0 * (9600 + i) as f32 / SR).sin()).collect();
            f.process(&mut buf);
            let jump = buf.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            assert!(buf.iter().all(|s| s.is_finite()));
            assert!(jump < 0.6, "param {idx}: {jump}");
        }
    }

    #[test]
    fn every_param_changes_the_output() {
        let src = sine(1000.0, 0.05, 4800);
        let mut reference = src.clone();
        build(&[]).process(&mut reference);
        for (i, v) in [2.0, 0.0, -30.0].iter().enumerate() {
            let mut f = build(&[]);
            f.set_param(i, *v);
            let mut buf = src.clone();
            f.process(&mut buf);
            let diff: f32 = buf.iter().zip(&reference).map(|(a, b)| (a - b).abs()).sum();
            assert!(diff > 0.5, "param {i}: {diff}");
        }
    }
}
