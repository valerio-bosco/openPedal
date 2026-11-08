//! Blocchi DSP, catalogo e catena lineare (preset v1).
//!
//! Regola d'oro: `Block::process` e `Block::set_param` girano nel thread realtime.
//! Niente allocazioni, niente lock, niente I/O. Tutto ciò che serve (buffer,
//! coefficienti) si prepara nella funzione `build_*`, fuori dal thread audio.

mod cab;
mod compressor;
mod drive;
mod dynamics;
mod filter;
mod modulation;
pub mod oversample;
pub mod params;
mod reverb;
mod time;
mod tonestack;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub use params::{BlockDesc, Category, ParamDesc, ParamKind, ParamValues, Value};

/// Un'unità di elaborazione: lavora in-place su un blocco di campioni mono.
pub trait Block: Send {
    fn process(&mut self, buf: &mut [f32]);

    /// Cambia un parametro dal vivo. `index` è la posizione in `BlockDesc::params`,
    /// `value` è già limitato all'intervallo (per le scelte: l'indice dell'opzione).
    /// Chiamato dal thread audio: niente allocazioni; le variazioni brusche vanno
    /// smussate (circa 10-20 ms) per non sentire click.
    /// Il default non fa niente: il parametro cambierà alla prossima ricostruzione.
    fn set_param(&mut self, _index: usize, _value: f32) {}

    /// Azzera lo stato interno (code di delay e riverbero, inviluppi...).
    fn reset(&mut self) {}
}

pub type BuildFn = fn(&ParamValues, f32) -> Result<Box<dyn Block>>;

/// Una voce del catalogo: come si descrive un blocco e come lo si costruisce.
pub struct Entry {
    pub desc: &'static BlockDesc,
    pub build: BuildFn,
}

/// Tutti i blocchi base. Registrare un blocco nuovo = aggiungere una riga qui.
static CATALOG: &[Entry] = &[
    Entry { desc: &dynamics::GAIN_DESC, build: dynamics::build_gain },
    Entry { desc: &dynamics::GATE_DESC, build: dynamics::build_gate },
    Entry { desc: &compressor::COMPRESSOR_DESC, build: compressor::build_compressor },
    Entry { desc: &drive::DRIVE_DESC, build: drive::build_drive },
    Entry { desc: &tonestack::TONESTACK_DESC, build: tonestack::build_tonestack },
    Entry { desc: &cab::CAB_DESC, build: cab::build_cab },
    Entry { desc: &filter::FILTER_DESC, build: filter::build_filter },
    Entry { desc: &modulation::TREMOLO_DESC, build: modulation::build_tremolo },
    Entry { desc: &time::CHORUS_DESC, build: time::build_chorus },
    Entry { desc: &time::DELAY_DESC, build: time::build_delay },
    Entry { desc: &reverb::REVERB_DESC, build: reverb::build_reverb },
];

pub fn catalog() -> &'static [Entry] {
    CATALOG
}

pub fn find(kind: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.desc.kind == kind)
}

/// Valida i parametri e costruisce il blocco `kind`.
pub fn build_block(kind: &str, values: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let Some(entry) = find(kind) else {
        let valid: Vec<_> = CATALOG.iter().map(|e| e.desc.kind).collect();
        bail!("blocco sconosciuto '{kind}' (blocchi validi: {})", valid.join(", "));
    };
    values.validate(entry.desc)?;
    (entry.build)(values, sr).with_context(|| format!("blocco '{kind}'"))
}

/// Un blocco di un preset v1: `type` più i suoi parametri.
#[derive(Deserialize, Debug, Clone)]
pub struct BlockEntry {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub params: ParamValues,
}

/// Preset v1: nome, volume finale e lista ordinata di blocchi.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default = "default_name")]
    pub name: String,
    /// Volume finale in dB, applicato dopo tutta la catena.
    #[serde(default)]
    pub output_db: f32,
    #[serde(default, rename = "block")]
    pub blocks: Vec<BlockEntry>,
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
        let blocks = preset
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| build_block(&b.kind, &b.params, sr).with_context(|| format!("blocco n. {}", i + 1)))
            .collect::<Result<_>>()?;
        Ok(Chain { blocks, out_gain: db_to_lin(preset.output_db) })
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

/// Valore che si muove dolcemente verso un obiettivo: per i parametri cambiati dal vivo.
#[derive(Debug, Clone, Copy)]
pub struct Smoothed {
    pub value: f32,
    pub target: f32,
    coef: f32,
}

impl Smoothed {
    pub fn new(value: f32, ms: f32, sr: f32) -> Self {
        Self { value, target: value, coef: one_pole_coef(ms, sr) }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        self.value = self.target + (self.value - self.target) * self.coef;
        self.value
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
        let p: Preset = toml::from_str("[[block]]\ntype = \"drive\"\ngain = 3\n").unwrap();
        let err = Chain::build(&p, SR).err().expect("il refuso `gain` (invece di gain_db) deve essere segnalato");
        assert!(format!("{err:#}").contains("gain_db"), "{err:#}");
    }

    #[test]
    fn una_scelta_sbagliata_e_un_errore() {
        let p: Preset = toml::from_str("[[block]]\ntype = \"drive\"\nshape = \"tubo\"\n").unwrap();
        assert!(Chain::build(&p, SR).is_err());
    }

    #[test]
    fn i_default_del_catalogo_sono_validi() {
        // Ogni blocco si costruisce con i soli default e non produce NaN.
        for entry in catalog() {
            let mut block = (entry.build)(&ParamValues::default(), SR).unwrap();
            let mut buf = sine(220.0, 0.5, 4800);
            block.process(&mut buf);
            assert!(buf.iter().all(|s| s.is_finite()), "{}", entry.desc.kind);
            for (i, p) in entry.desc.params.iter().enumerate() {
                let v = match p.kind {
                    ParamKind::Number { max, .. } => max,
                    ParamKind::Choice { options, .. } => (options.len() - 1) as f32,
                    ParamKind::File { .. } => continue,
                };
                block.set_param(i, v);
            }
            block.process(&mut buf);
            assert!(buf.iter().all(|s| s.is_finite()), "{} dopo set_param", entry.desc.kind);
        }
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
