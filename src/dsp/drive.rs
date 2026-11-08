use std::f32::consts::FRAC_PI_2;

use serde::Deserialize;

use super::{db_to_lin, Block};

/// Curva di saturazione (waveshaper).
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// tanh: saturazione morbida, tipo overdrive.
    Soft,
    /// Taglio netto: distorsione aggressiva, tipo diodi.
    Hard,
    /// Asimmetrica: armoniche pari, carattere "valvolare".
    Asym,
    /// Fuzz: compressione estrema e sustain lungo.
    Fuzz,
    /// Wavefolder: l'onda si ripiega su se stessa, suono sintetico.
    Fold,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct DriveParams {
    pub shape: Shape,
    /// Guadagno prima della curva: più è alto, più distorce.
    pub gain_db: f32,
    /// Volume dopo la curva.
    pub level_db: f32,
    /// Asimmetria, usata dalla curva `asym`.
    pub bias: f32,
}

impl Default for DriveParams {
    fn default() -> Self {
        Self { shape: Shape::Soft, gain_db: 12.0, level_db: -6.0, bias: 0.2 }
    }
}

pub struct Drive {
    shape: Shape,
    pre: f32,
    post: f32,
    bias: f32,
    bias_offset: f32,
    // DC blocker: le curve asimmetriche spostano il segnale dallo zero.
    dc_x: f32,
    dc_y: f32,
    dc_r: f32,
}

impl Drive {
    pub fn new(p: &DriveParams, sr: f32) -> Self {
        Self {
            shape: p.shape,
            pre: db_to_lin(p.gain_db),
            post: db_to_lin(p.level_db),
            bias: p.bias,
            bias_offset: p.bias.tanh(),
            dc_x: 0.0,
            dc_y: 0.0,
            dc_r: 1.0 - (2.0 * std::f32::consts::PI * 10.0 / sr),
        }
    }

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
            let y = self.shape(*s * self.pre);
            let out = y - self.dc_x + self.dc_r * self.dc_y;
            self.dc_x = y;
            self.dc_y = out;
            *s = out * self.post;
        }
    }
}
