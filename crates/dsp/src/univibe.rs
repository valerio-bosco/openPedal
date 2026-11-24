//! Uni-Vibe style: four staggered all-pass stages swept by a lamp-like asymmetric LFO.

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::phaser::{flush, AllPass};
use super::{db_to_lin, one_pole_coef, Block, Smoothed};

pub const UNIVIBE_DESC: BlockDesc = BlockDesc {
    kind: "univibe",
    label: "Uni-Vibe",
    category: Category::Modulation,
    params: &[
        ParamDesc::log("rate_hz", "Speed", 0.5, 12.0, 4.0, "Hz"),
        // Sweep width of the stages (0 = static, 1 = about 3 octaves).
        ParamDesc::number("intensity", "Intensity", 0.0, 1.0, 0.7, ""),
        // chorus = 50/50 dry and wet (notches); vibrato = wet only (pitch wobble).
        ParamDesc::choice("mode", "Mode", &["chorus", "vibrato"], 0),
        ParamDesc::number("level_db", "Level", -24.0, 12.0, 0.0, "dB"),
    ],
};

// Parameter indices in UNIVIBE_DESC.
const RATE: usize = 0;
const INTENSITY: usize = 1;
const MODE: usize = 2;
const LEVEL: usize = 3;

/// Centre frequencies of the four stages (different capacitors in the original).
const BASE_HZ: [f32; 4] = [150.0, 380.0, 950.0, 2400.0];
/// Fraction of the cycle spent rising (the lamp lights up quickly and fades slowly).
const RISE: f32 = 0.3;
/// Sweep of the stages in octaves at full intensity.
const OCTAVES: f32 = 3.0;

pub fn build_univibe(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = UNIVIBE_DESC.params;
    Ok(Box::new(Univibe {
        sr,
        phase: 0.0,
        inc: v.number(&p[RATE]) / sr,
        lamp: 0.0,
        lamp_coef: one_pole_coef(4.0, sr),
        stages: [AllPass::default(); 4],
        intensity: Smoothed::new(v.number(&p[INTENSITY]), 15.0, sr),
        wet_mix: Smoothed::new(wet_for(v.choice(&p[MODE])), 15.0, sr),
        level: Smoothed::new(db_to_lin(v.number(&p[LEVEL])), 15.0, sr),
    }))
}

/// Lamp brightness in [0, 1] over a cycle `p` in [0, 1): raised-cosine rise, then slower fall.
fn lamp_shape(p: f32) -> f32 {
    if p < RISE {
        0.5 - 0.5 * (std::f32::consts::PI * p / RISE).cos()
    } else {
        0.5 + 0.5 * (std::f32::consts::PI * (p - RISE) / (1.0 - RISE)).cos()
    }
}

fn wet_for(mode: usize) -> f32 {
    if mode == 1 {
        1.0
    } else {
        0.5
    }
}

struct Univibe {
    sr: f32,
    phase: f32,
    inc: f32,
    lamp: f32,
    lamp_coef: f32,
    stages: [AllPass; 4],
    intensity: Smoothed,
    wet_mix: Smoothed,
    level: Smoothed,
}

impl Block for Univibe {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let x = *s;
            // Asymmetric cycle in [0, 1]: raised-cosine rise, then slower fall.
            let p = self.phase;
            self.phase += self.inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            let raw = lamp_shape(p);
            // The filament has thermal inertia.
            self.lamp = raw + (self.lamp - raw) * self.lamp_coef;
            let shift = OCTAVES * self.intensity.next() * (self.lamp - 0.5);
            let factor = shift.exp2();

            let mut y = x;
            for (st, base) in self.stages.iter_mut().zip(BASE_HZ) {
                y = st.tick(y, AllPass::coef(base * factor, self.sr));
            }
            let w = self.wet_mix.next();
            *s = (x * (1.0 - w) + y * w) * self.level.next();
        }
        self.lamp = flush(self.lamp);
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            RATE => self.inc = value / self.sr,
            INTENSITY => self.intensity.target = value,
            MODE => self.wet_mix.target = wet_for(value as usize),
            LEVEL => self.level.target = db_to_lin(value),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.stages = [AllPass::default(); 4];
        self.lamp = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::phaser::testkit::*;

    const SR: f32 = 48000.0;

    #[test]
    fn finite_and_bounded() {
        check_finite_and_bounded("univibe", SR, 8.0);
        check_finite_and_bounded("univibe", 44100.0, 8.0);
    }

    #[test]
    fn live_params_do_not_jump() {
        check_live_no_jumps("univibe", SR, "", 0.5);
        check_live_no_jumps("univibe", SR, "mode = \"vibrato\"", 0.5);
    }

    #[test]
    fn vibrato_is_wet_only_and_chorus_has_notches() {
        let input = noise(48000, 0.3, 2);
        let mut v = make("univibe", "mode = \"vibrato\"", SR);
        let mut c = make("univibe", "mode = \"chorus\"", SR);
        let (mut vb, mut cb) = (input.clone(), input.clone());
        v.process(&mut vb);
        c.process(&mut cb);
        // All-pass chain: same energy as the input; chorus sums with the dry signal and loses energy.
        let ri = rms(&input[4800..]);
        assert!((rms(&vb[4800..]) / ri - 1.0).abs() < 0.1, "{}", rms(&vb[4800..]) / ri);
        assert!(rms(&cb[4800..]) < 0.95 * ri);
    }

    #[test]
    fn chorus_notch_moves_with_the_lamp() {
        let mut b = make("univibe", "rate_hz = 1\nintensity = 1\nmode = \"chorus\"", SR);
        let mut buf = sine(500.0, 0.5, 96000, SR);
        b.process(&mut buf);
        let levels: Vec<f32> = buf.chunks(480).skip(10).map(rms).collect();
        let min = levels.iter().cloned().fold(f32::MAX, f32::min);
        let max = levels.iter().cloned().fold(0.0, f32::max);
        assert!(min < 0.4 * max, "min {min}, max {max}");
    }

    #[test]
    fn lamp_rises_fast_and_falls_slowly() {
        let n = 1000;
        let shape: Vec<f32> = (0..n).map(|i| super::lamp_shape(i as f32 / n as f32)).collect();
        let peak = shape.iter().position(|v| *v >= 0.999).unwrap();
        assert!(peak < n / 2, "peak at {peak}");
        let up = shape.iter().position(|v| *v > 0.5).unwrap();
        let down = shape.iter().skip(peak).position(|v| *v < 0.5).unwrap() + peak;
        assert!(up < peak && down - peak > 0 && (n - peak) > 2 * peak, "up {up}, peak {peak}, down {down}");
        assert!(shape[0] < 0.01 && shape[n - 1] < 0.01);
    }

    #[test]
    fn level_applies() {
        let mut b = make("univibe", "intensity = 0\nmode = \"vibrato\"\nlevel_db = -6", SR);
        let mut buf = noise(24000, 0.3, 4);
        let r = rms(&buf);
        b.process(&mut buf);
        assert!((rms(&buf[4800..]) / r - 0.5).abs() < 0.08);
    }

    #[test]
    fn sample_rate_independent() {
        let run = |sr: f32| {
            let mut b = make("univibe", "rate_hz = 2", sr);
            let mut buf = noise((sr * 2.0) as usize, 0.5, 11);
            b.process(&mut buf);
            rms(&buf[buf.len() / 2..])
        };
        assert!((run(44100.0) / run(48000.0) - 1.0).abs() < 0.1);
    }
}
