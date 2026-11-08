use std::f32::consts::TAU;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{one_pole_coef, Block};

pub const TREMOLO_DESC: BlockDesc = BlockDesc {
    kind: "tremolo",
    label: "Tremolo",
    category: Category::Modulation,
    params: &[
        ParamDesc::log("rate_hz", "Rate", 0.1, 20.0, 5.0, "Hz"),
        // 0 = no effect, 1 = the volume drops to zero.
        ParamDesc::number("depth", "Depth", 0.0, 1.0, 0.5, ""),
        ParamDesc::choice("wave", "Wave", &["sine", "triangle", "square"], 0),
    ],
};

// Parameter indices in TREMOLO_DESC.
const RATE: usize = 0;
const DEPTH: usize = 1;
const WAVE: usize = 2;

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
    /// Same order as the choices in TREMOLO_DESC.
    pub const ALL: [Wave; 3] = [Wave::Sine, Wave::Triangle, Wave::Square];
}

/// Slow oscillator (LFO), output in [-1, 1].
pub struct Lfo {
    phase: f32,
    inc: f32,
    wave: Wave,
}

impl Lfo {
    pub fn new(rate_hz: f32, wave: Wave, sr: f32) -> Self {
        Self { phase: 0.0, inc: rate_hz / sr, wave }
    }

    /// Changes the rate without restarting the phase.
    pub fn set_rate(&mut self, rate_hz: f32, sr: f32) {
        self.inc = rate_hz / sr;
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
    /// 0 = no effect, 1 = the volume drops to zero.
    pub depth: f32,
    pub wave: Wave,
}

impl Default for TremoloParams {
    fn default() -> Self {
        Self { rate_hz: 5.0, depth: 0.5, wave: Wave::Sine }
    }
}

pub struct Tremolo {
    sr: f32,
    lfo: Lfo,
    depth: f32,
    gain: f32,
    // Smooths the square wave to avoid clicks.
    smooth: f32,
}

impl Tremolo {
    pub fn new(p: &TremoloParams, sr: f32) -> Self {
        Self {
            sr,
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

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            RATE => self.lfo.set_rate(value, self.sr),
            DEPTH => self.depth = value.clamp(0.0, 1.0),
            WAVE => self.lfo.wave = Wave::ALL[(value as usize).min(Wave::ALL.len() - 1)],
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    #[test]
    fn set_param_changes_depth_live() {
        // With depth 0 the volume stays at 1; raising it, the tremolo drops almost to zero.
        let mut t = Tremolo::new(&TremoloParams { depth: 0.0, ..Default::default() }, SR);
        let mut buf = vec![1.0; 4800];
        t.process(&mut buf);
        assert!(buf.iter().all(|&s| s > 0.999));

        t.set_param(DEPTH, 1.0);
        let mut buf = vec![1.0; 9600];
        t.process(&mut buf);
        let min = buf.iter().fold(f32::MAX, |m, &s| m.min(s));
        assert!(min < 0.1, "minimum {min}");
    }
}
