use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, one_pole_coef, Block, Smoothed};

// ---- Gain: esempio di riferimento di un blocco migrato -------------------------

pub const GAIN_DESC: BlockDesc = BlockDesc {
    kind: "gain",
    label: "Volume",
    category: Category::Utility,
    params: &[ParamDesc::number("db", "Volume", -60.0, 24.0, 0.0, "dB")],
};

pub fn build_gain(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = GAIN_DESC.params;
    Ok(Box::new(Gain { gain: Smoothed::new(db_to_lin(v.number(&p[0])), 15.0, sr) }))
}

pub struct Gain {
    gain: Smoothed,
}

impl Block for Gain {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            *s *= self.gain.next();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.gain.target = db_to_lin(value);
        }
    }
}

// ---- Gate (da migrare: D1) ---------------------------------------------------

pub const GATE_DESC: BlockDesc = BlockDesc {
    kind: "gate",
    label: "Noise gate",
    category: Category::Dynamics,
    params: &[
        ParamDesc::number("threshold_db", "Soglia", -90.0, 0.0, -60.0, "dB"),
        ParamDesc::log("attack_ms", "Attacco", 0.1, 50.0, 0.5, "ms"),
        ParamDesc::log("hold_ms", "Tenuta", 1.0, 500.0, 40.0, "ms"),
        ParamDesc::log("release_ms", "Rilascio", 5.0, 2000.0, 80.0, "ms"),
    ],
};

pub fn build_gate(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = GATE_DESC.params;
    let params = GateParams {
        threshold_db: v.number(&p[0]),
        attack_ms: v.number(&p[1]),
        hold_ms: v.number(&p[2]),
        release_ms: v.number(&p[3]),
    };
    Ok(Box::new(Gate::new(&params, sr)))
}

/// Noise gate: chiude il segnale sotto soglia, per eliminare il fruscio
/// quando non suoni (indispensabile prima di un drive).
#[derive(Debug, Clone)]
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
