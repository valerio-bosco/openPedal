//! Drive: saturation via a waveshaper. Reference example of a block
//! migrated to the catalog, with parameters that can be changed live.

use std::f32::consts::FRAC_PI_2;

use anyhow::Result;

use super::oversample::Oversampler4;
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, Block, Smoothed};

pub const DRIVE_DESC: BlockDesc = BlockDesc {
    kind: "drive",
    label: "Drive",
    category: Category::Drive,
    params: &[
        // soft: tanh, overdrive-like · hard: hard clip, diode-like · asym: even harmonics,
        // "tube-like" · fuzz: extreme compression · fold: the wave folds back, synthetic sound
        ParamDesc::choice("shape", "Shape", &["soft", "hard", "asym", "fuzz", "fold"], 0),
        ParamDesc::number("gain_db", "Gain", 0.0, 60.0, 12.0, "dB"),
        ParamDesc::number("level_db", "Level", -40.0, 12.0, -6.0, "dB"),
        ParamDesc::number("bias", "Bias", 0.0, 1.0, 0.2, ""),
        // eco = 1x, alta = 4x oversampling (less aliasing, ~0.55 ms of latency).
        ParamDesc::choice("quality", "Quality", &["eco", "alta"], 1),
    ],
};

const SHAPE: usize = 0;
const GAIN: usize = 1;
const LEVEL: usize = 2;
const BIAS: usize = 3;
const QUALITY: usize = 4;

pub fn build_drive(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = DRIVE_DESC.params;
    let bias = v.number(&p[BIAS]);
    Ok(Box::new(Drive {
        shape: Shape::from_index(v.choice(&p[SHAPE])),
        pre: Smoothed::new(db_to_lin(v.number(&p[GAIN])), 15.0, sr),
        post: Smoothed::new(db_to_lin(v.number(&p[LEVEL])), 15.0, sr),
        bias,
        bias_offset: bias.tanh(),
        os: Oversampler4::new(),
        hq: v.choice(&p[QUALITY]) == 1,
        dc_x: 0.0,
        dc_y: 0.0,
        dc_r: 1.0 - (2.0 * std::f32::consts::PI * 10.0 / sr),
    }))
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    Soft,
    Hard,
    Asym,
    Fuzz,
    Fold,
}

impl Shape {
    fn from_index(i: usize) -> Shape {
        [Shape::Soft, Shape::Hard, Shape::Asym, Shape::Fuzz, Shape::Fold][i.min(4)]
    }
}

pub struct Drive {
    shape: Shape,
    pre: Smoothed,
    post: Smoothed,
    bias: f32,
    bias_offset: f32,
    os: Oversampler4,
    hq: bool,
    // DC blocker: asymmetric curves shift the signal away from zero.
    dc_x: f32,
    dc_y: f32,
    dc_r: f32,
}

impl Drive {
    #[inline]
    fn shape(&self, x: f32) -> f32 {
        shape_fn(self.shape, self.bias, self.bias_offset, x)
    }
}

#[inline]
fn shape_fn(shape: Shape, bias: f32, bias_offset: f32, x: f32) -> f32 {
    match shape {
        Shape::Soft => x.tanh(),
        Shape::Hard => x.clamp(-1.0, 1.0),
        Shape::Asym => (x + bias).tanh() - bias_offset,
        Shape::Fuzz => x.signum() * (1.0 - (-x.abs() * 2.0).exp()),
        Shape::Fold => (x * FRAC_PI_2).sin(),
    }
}

impl Block for Drive {
    fn process(&mut self, buf: &mut [f32]) {
        if self.hq {
            // gain at the base rate, curve at 4x, then back to the base rate
            for s in buf.iter_mut() {
                *s *= self.pre.next();
            }
            let (shape, bias, off) = (self.shape, self.bias, self.bias_offset);
            self.os.process(buf, |x| shape_fn(shape, bias, off, x));
            for s in buf.iter_mut() {
                let y = *s;
                let out = y - self.dc_x + self.dc_r * self.dc_y;
                self.dc_x = y;
                self.dc_y = out;
                *s = out * self.post.next();
            }
            return;
        }
        for s in buf {
            let pre = self.pre.next();
            let y = self.shape(*s * pre);
            let out = y - self.dc_x + self.dc_r * self.dc_y;
            self.dc_x = y;
            self.dc_y = out;
            *s = out * self.post.next();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            SHAPE => self.shape = Shape::from_index(value as usize),
            GAIN => self.pre.target = db_to_lin(value),
            LEVEL => self.post.target = db_to_lin(value),
            BIAS => {
                self.bias = value;
                self.bias_offset = value.tanh();
            }
            QUALITY => {
                let hq = value >= 0.5;
                if hq && !self.hq {
                    self.os.reset();
                }
                self.hq = hq;
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.os.reset();
        self.dc_x = 0.0;
        self.dc_y = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::Value;

    const SR: f32 = 48000.0;

    /// Energy (sum of squared DFT coefficients) below `fmax`, excluding DC.
    fn energy_below(buf: &[f32], fmax: f32) -> f64 {
        let n = buf.len();
        let kmax = (fmax / SR * n as f32) as usize;
        (1..kmax)
            .map(|k| {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, &x) in buf.iter().enumerate() {
                    let w = std::f64::consts::TAU * (k * i % n) as f64 / n as f64;
                    re += x as f64 * w.cos();
                    im -= x as f64 * w.sin();
                }
                (re * re + im * im) / (n * n) as f64
            })
            .sum()
    }

    fn render(shape: usize, quality: usize) -> Vec<f32> {
        let mut v = ParamValues::default();
        v.set("shape", Value::Number(shape as f32));
        v.set("gain_db", Value::Number(30.0));
        v.set("quality", Value::Number(quality as f32));
        let mut d = build_drive(&v, SR).unwrap();
        let n = 4800;
        // 5 kHz = exactly 500 periods in 4800 samples (no spectral leakage)
        let mut buf: Vec<f32> = (0..2 * n).map(|i| 0.5 * (std::f32::consts::TAU * 5000.0 * i as f32 / SR).sin()).collect();
        d.process(&mut buf);
        buf[n..].to_vec()
    }

    #[test]
    fn oversampling_reduces_aliasing() {
        for (shape, name) in [(0, "soft"), (1, "hard"), (4, "fold")] {
            let eco = energy_below(&render(shape, 0), 4000.0);
            let alta = energy_below(&render(shape, 1), 4000.0);
            let db = 10.0 * (eco / alta.max(1e-30)).log10();
            println!("aliasing {name}: eco/alta = {db:.1} dB");
            assert!(db >= 20.0, "{name}: only {db:.1} dB");
        }
    }

    #[test]
    fn quality_switch_live_is_finite() {
        let mut d = build_drive(&ParamValues::default(), SR).unwrap();
        let mut buf = vec![0.3f32; 512];
        d.process(&mut buf);
        d.set_param(QUALITY, 0.0);
        d.process(&mut buf);
        d.set_param(QUALITY, 1.0);
        d.process(&mut buf);
        assert!(buf.iter().all(|x| x.is_finite()));
    }
}
