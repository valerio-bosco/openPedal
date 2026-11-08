use std::f32::consts::TAU;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{one_pole_coef, Block};

// Da migrare (D1): set_param.
pub const TREMOLO_DESC: BlockDesc = BlockDesc {
    kind: "tremolo",
    label: "Tremolo",
    category: Category::Modulation,
    params: &[
        ParamDesc::log("rate_hz", "Velocità", 0.1, 20.0, 5.0, "Hz"),
        // 0 = nessun effetto, 1 = il volume scende fino a zero.
        ParamDesc::number("depth", "Profondità", 0.0, 1.0, 0.5, ""),
        ParamDesc::choice("wave", "Onda", &["sine", "triangle", "square"], 0),
    ],
};

pub fn build_tremolo(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = TREMOLO_DESC.params;
    let params = TremoloParams { rate_hz: v.number(&p[0]), depth: v.number(&p[1]), wave: Wave::ALL[v.choice(&p[2])] };
    Ok(Box::new(Tremolo::new(&params, sr)))
}

#[derive(Debug, Clone, Copy)]
pub enum Wave {
    Sine,
    Triangle,
    Square,
}

impl Wave {
    /// Nello stesso ordine delle scelte in TREMOLO_DESC.
    pub const ALL: [Wave; 3] = [Wave::Sine, Wave::Triangle, Wave::Square];
}

/// Oscillatore lento (LFO), uscita in [-1, 1].
pub struct Lfo {
    phase: f32,
    inc: f32,
    wave: Wave,
}

impl Lfo {
    pub fn new(rate_hz: f32, wave: Wave, sr: f32) -> Self {
        Self { phase: 0.0, inc: rate_hz / sr, wave }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        let p = self.phase;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        match self.wave {
            Wave::Sine => (p * TAU).sin(),
            Wave::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            Wave::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TremoloParams {
    pub rate_hz: f32,
    /// 0 = nessun effetto, 1 = il volume scende fino a zero.
    pub depth: f32,
    pub wave: Wave,
}

impl Default for TremoloParams {
    fn default() -> Self {
        Self { rate_hz: 5.0, depth: 0.5, wave: Wave::Sine }
    }
}

pub struct Tremolo {
    lfo: Lfo,
    depth: f32,
    gain: f32,
    // Smussa l'onda quadra per non avere click.
    smooth: f32,
}

impl Tremolo {
    pub fn new(p: &TremoloParams, sr: f32) -> Self {
        Self {
            lfo: Lfo::new(p.rate_hz, p.wave, sr),
            depth: p.depth.clamp(0.0, 1.0),
            gain: 1.0,
            smooth: one_pole_coef(2.0, sr),
        }
    }
}

impl Block for Tremolo {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let target = 1.0 - self.depth * (0.5 + 0.5 * self.lfo.next());
            self.gain = target + (self.gain - target) * self.smooth;
            *s *= self.gain;
        }
    }
}
