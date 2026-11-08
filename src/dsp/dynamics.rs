use serde::Deserialize;

use super::{db_to_lin, one_pole_coef, Block};

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct GainParams {
    pub db: f32,
}

impl Default for GainParams {
    fn default() -> Self {
        Self { db: 0.0 }
    }
}

pub struct Gain {
    gain: f32,
}

impl Gain {
    pub fn new(p: &GainParams) -> Self {
        Self { gain: db_to_lin(p.db) }
    }
}

impl Block for Gain {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            *s *= self.gain;
        }
    }
}

/// Noise gate: chiude il segnale sotto soglia, per eliminare il fruscio
/// quando non suoni (indispensabile prima di un drive).
#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct GateParams {
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
}

impl Default for GateParams {
    fn default() -> Self {
        Self { threshold_db: -60.0, attack_ms: 0.5, hold_ms: 40.0, release_ms: 80.0 }
    }
}

pub struct Gate {
    open_thr: f32,
    close_thr: f32,
    env: f32,
    env_coef: f32,
    gain: f32,
    attack: f32,
    release: f32,
    hold_samples: u32,
    hold_left: u32,
}

impl Gate {
    pub fn new(p: &GateParams, sr: f32) -> Self {
        Self {
            open_thr: db_to_lin(p.threshold_db),
            // Isteresi di 6 dB: evita che il gate "balbetti" attorno alla soglia.
            close_thr: db_to_lin(p.threshold_db - 6.0),
            env: 0.0,
            env_coef: one_pole_coef(10.0, sr),
            gain: 0.0,
            attack: one_pole_coef(p.attack_ms, sr),
            release: one_pole_coef(p.release_ms, sr),
            hold_samples: (p.hold_ms * 0.001 * sr) as u32,
            hold_left: 0,
        }
    }
}

impl Block for Gate {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let x = s.abs();
            self.env = if x > self.env { x } else { self.env * self.env_coef };

            if self.env > self.open_thr {
                self.hold_left = self.hold_samples;
            } else if self.env < self.close_thr && self.hold_left > 0 {
                self.hold_left -= 1;
            }
            let open = self.hold_left > 0;

            let (target, coef) = if open { (1.0, self.attack) } else { (0.0, self.release) };
            self.gain = target + (self.gain - target) * coef;
            *s *= self.gain;
        }
    }
}
