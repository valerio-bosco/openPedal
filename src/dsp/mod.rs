//! Blocchi DSP e catena di effetti.
//!
//! Regola d'oro: `Block::process` gira nel thread realtime. Niente allocazioni,
//! niente lock, niente I/O. Tutto ciò che serve (buffer, coefficienti) si prepara
//! in `build`, fuori dal thread audio.

mod cab;
mod drive;
mod dynamics;
mod filter;
mod modulation;
mod time;

use anyhow::Result;
use serde::Deserialize;

pub use cab::CabParams;
pub use drive::DriveParams;
pub use dynamics::{GainParams, GateParams};
pub use filter::FilterParams;
pub use modulation::TremoloParams;
pub use time::{ChorusParams, DelayParams};

/// Un'unità di elaborazione: lavora in-place su un blocco di campioni mono.
pub trait Block: Send {
    fn process(&mut self, buf: &mut [f32]);
}

/// Descrizione di un blocco come appare nel preset (`type = "..."`).
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BlockSpec {
    Gain(GainParams),
    Gate(GateParams),
    Drive(DriveParams),
    Filter(FilterParams),
    Delay(DelayParams),
    Chorus(ChorusParams),
    Tremolo(TremoloParams),
    Cab(CabParams),
}

impl BlockSpec {
    fn build(&self, sr: f32) -> Result<Box<dyn Block>> {
        Ok(match self {
            BlockSpec::Gain(p) => Box::new(dynamics::Gain::new(p)),
            BlockSpec::Gate(p) => Box::new(dynamics::Gate::new(p, sr)),
            BlockSpec::Drive(p) => Box::new(drive::Drive::new(p, sr)),
            BlockSpec::Filter(p) => Box::new(filter::Biquad::new(p, sr)),
            BlockSpec::Delay(p) => Box::new(time::Delay::new(p, sr)),
            BlockSpec::Chorus(p) => Box::new(time::Chorus::new(p, sr)),
            BlockSpec::Tremolo(p) => Box::new(modulation::Tremolo::new(p, sr)),
            BlockSpec::Cab(p) => Box::new(cab::Cab::new(p, sr)?),
        })
    }
}

/// Un preset: nome, volume finale e lista ordinata di blocchi.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default = "default_name")]
    pub name: String,
    /// Volume finale in dB, applicato dopo tutta la catena.
    #[serde(default)]
    pub output_db: f32,
    #[serde(default, rename = "block")]
    pub blocks: Vec<BlockSpec>,
}

fn default_name() -> String {
    "senza nome".into()
}

/// La catena pronta per il thread audio.
pub struct Chain {
    blocks: Vec<Box<dyn Block>>,
    out_gain: f32,
}

impl Chain {
    /// Prepara la catena (fuori dal thread audio): può leggere file, quindi può fallire.
    pub fn build(preset: &Preset, sr: f32) -> Result<Chain> {
        Ok(Chain {
            blocks: preset.blocks.iter().map(|b| b.build(sr)).collect::<Result<_>>()?,
            out_gain: db_to_lin(preset.output_db),
        })
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for block in &mut self.blocks {
            block.process(buf);
        }
        for s in buf.iter_mut() {
            *s *= self.out_gain;
        }
    }
}

pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-9).log10()
}

/// Coefficiente di uno smoothing a un polo con costante di tempo `ms`.
pub fn one_pole_coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * sr)).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    fn chain(toml_text: &str) -> Chain {
        Chain::build(&toml::from_str(toml_text).unwrap(), SR).unwrap()
    }

    #[test]
    fn i_preset_inclusi_sono_validi() {
        for entry in std::fs::read_dir("presets").unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let p: Preset = toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut buf = sine(220.0, 0.5, 48000);
            Chain::build(&p, SR).unwrap().process(&mut buf);
            assert!(buf.iter().all(|s| s.is_finite()), "{}", path.display());
        }
    }

    #[test]
    fn un_parametro_sbagliato_e_un_errore() {
        let r: Result<Preset, _> = toml::from_str("[[block]]\ntype = \"drive\"\ngain = 3\n");
        assert!(r.is_err(), "il refuso `gain` (invece di gain_db) deve essere segnalato");
    }

    #[test]
    fn lowpass_taglia_gli_alti() {
        let mut c = chain("[[block]]\ntype = \"filter\"\nkind = \"lowpass\"\nfreq = 1000\n");
        let mut low = sine(100.0, 1.0, 9600);
        let mut high = sine(10000.0, 1.0, 9600);
        c.process(&mut low);
        let mut c = chain("[[block]]\ntype = \"filter\"\nkind = \"lowpass\"\nfreq = 1000\n");
        c.process(&mut high);
        assert!(peak(&low[4800..]) > 0.95);
        assert!(peak(&high[4800..]) < 0.02);
    }

    #[test]
    fn drive_resta_limitato() {
        for shape in ["soft", "hard", "asym", "fuzz", "fold"] {
            let mut c = chain(&format!(
                "[[block]]\ntype = \"drive\"\nshape = \"{shape}\"\ngain_db = 40\nlevel_db = 0\n"
            ));
            let mut buf = sine(110.0, 1.0, 4800);
            c.process(&mut buf);
            assert!(peak(&buf) < 2.5, "{shape}: {}", peak(&buf));
        }
    }

    #[test]
    fn gate_chiude_il_fruscio_e_lascia_passare_le_note() {
        let mut c = chain("[[block]]\ntype = \"gate\"\nthreshold_db = -50\n");
        let mut noise = sine(3000.0, 0.001, 24000); // -60 dB
        c.process(&mut noise);
        assert!(peak(&noise[12000..]) < 1e-4);
        let mut note = sine(220.0, 0.3, 4800);
        c.process(&mut note);
        assert!(peak(&note[480..]) > 0.29);
    }

    #[test]
    fn delay_ripete_dopo_il_tempo_giusto() {
        let mut c = chain("[[block]]\ntype = \"delay\"\ntime_ms = 100\nfeedback = 0\nmix = 0.5\n");
        let mut buf = vec![0.0; 9600];
        buf[0] = 1.0;
        c.process(&mut buf);
        let echo_at = buf[1..].iter().position(|s| s.abs() > 0.1).unwrap() + 1;
        assert_eq!(echo_at, 4800);
    }
}
