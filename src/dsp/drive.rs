//! Drive: saturazione tramite waveshaper. Esempio di riferimento di un blocco
//! migrato al catalogo, con parametri modificabili dal vivo.

use std::f32::consts::FRAC_PI_2;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, Block, Smoothed};

pub const DRIVE_DESC: BlockDesc = BlockDesc {
    kind: "drive",
    label: "Drive",
    category: Category::Drive,
    params: &[
        // soft: tanh, tipo overdrive · hard: taglio netto, tipo diodi · asym: armoniche pari,
        // "valvolare" · fuzz: compressione estrema · fold: l'onda si ripiega, suono sintetico
        ParamDesc::choice("shape", "Curva", &["soft", "hard", "asym", "fuzz", "fold"], 0),
        ParamDesc::number("gain_db", "Gain", 0.0, 60.0, 12.0, "dB"),
        ParamDesc::number("level_db", "Livello", -40.0, 12.0, -6.0, "dB"),
        ParamDesc::number("bias", "Asimmetria", 0.0, 1.0, 0.2, ""),
        // eco = 1x, alta = 4x di oversampling (meno aliasing). Implementato da D4.
        ParamDesc::choice("quality", "Qualità", &["eco", "alta"], 1),
    ],
};

const SHAPE: usize = 0;
const GAIN: usize = 1;
const LEVEL: usize = 2;
const BIAS: usize = 3;

pub fn build_drive(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = DRIVE_DESC.params;
    let bias = v.number(&p[BIAS]);
    Ok(Box::new(Drive {
        shape: Shape::from_index(v.choice(&p[SHAPE])),
        pre: Smoothed::new(db_to_lin(v.number(&p[GAIN])), 15.0, sr),
        post: Smoothed::new(db_to_lin(v.number(&p[LEVEL])), 15.0, sr),
        bias,
        bias_offset: bias.tanh(),
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
    // DC blocker: le curve asimmetriche spostano il segnale dallo zero.
    dc_x: f32,
    dc_y: f32,
    dc_r: f32,
}

impl Drive {
    #[inline]
    fn shape(&self, x: f32) -> f32 {
        match self.shape {
            Shape::Soft => x.tanh(),
            Shape::Hard => x.clamp(-1.0, 1.0),
            Shape::Asym => (x + self.bias).tanh() - self.bias_offset,
            Shape::Fuzz => x.signum() * (1.0 - (-x.abs() * 2.0).exp()),
            Shape::Fold => (x * FRAC_PI_2).sin(),
        }
    }
}

impl Block for Drive {
    fn process(&mut self, buf: &mut [f32]) {
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
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.dc_x = 0.0;
        self.dc_y = 0.0;
    }
}
