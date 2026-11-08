//! DSP blocks, catalog and linear chain (v1 presets).
//!
//! Golden rule: `Block::process` and `Block::set_param` run on the realtime thread.
//! No allocations, no locks, no I/O. Everything that is needed (buffers,
//! coefficients) is prepared in the `build_*` functions, outside the audio thread.

// `Smoothed::next` is a per-sample tick, not an Iterator.
#![allow(clippy::should_implement_trait)]

mod cab;
mod compressor;
mod drive;
mod dynamics;
mod filter;
mod fuzz;
mod modulation;
mod octave;
pub mod oversample;
pub mod params;
mod reverb;
mod time;
mod tonestack;
pub mod wav;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub use params::{BlockDesc, ParamKind, ParamValues, Value};

/// A processing unit: works in-place on a block of mono samples.
pub trait Block: Send {
    fn process(&mut self, buf: &mut [f32]);

    /// Changes a parameter live. `index` is the position in `BlockDesc::params`,
    /// `value` is already clamped to the range (for choices: the option index).
    /// Called from the audio thread: no allocations; abrupt changes must be
    /// smoothed (about 10-20 ms) to avoid clicks.
    /// The default does nothing: the parameter changes on the next rebuild.
    fn set_param(&mut self, _index: usize, _value: f32) {}

    /// Resets the internal state (delay and reverb tails, envelopes...).
    #[allow(dead_code)] // planned for a future "mute all" command; implemented by the blocks
    fn reset(&mut self) {}
}

pub type BuildFn = fn(&ParamValues, f32) -> Result<Box<dyn Block>>;

/// A catalog entry: how a block is described and how it is built.
pub struct Entry {
    pub desc: &'static BlockDesc,
    pub build: BuildFn,
}

/// All built-in blocks. Registering a new block = adding a line here.
static CATALOG: &[Entry] = &[
    Entry { desc: &dynamics::GAIN_DESC, build: dynamics::build_gain },
    Entry { desc: &dynamics::GATE_DESC, build: dynamics::build_gate },
    Entry { desc: &compressor::COMPRESSOR_DESC, build: compressor::build_compressor },
    Entry { desc: &drive::DRIVE_DESC, build: drive::build_drive },
    Entry { desc: &fuzz::FUZZ_DESC, build: fuzz::build_fuzz },
    Entry { desc: &tonestack::TONESTACK_DESC, build: tonestack::build_tonestack },
    Entry { desc: &cab::CAB_DESC, build: cab::build_cab },
    Entry { desc: &filter::FILTER_DESC, build: filter::build_filter },
    Entry { desc: &modulation::TREMOLO_DESC, build: modulation::build_tremolo },
    Entry { desc: &octave::OCTAVE_DESC, build: octave::build_octave },
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

/// Validates the parameters and builds the `kind` block.
pub fn build_block(kind: &str, values: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let Some(entry) = find(kind) else {
        let valid: Vec<_> = CATALOG.iter().map(|e| e.desc.kind).collect();
        bail!("unknown block '{kind}' (valid blocks: {})", valid.join(", "));
    };
    values.validate(entry.desc)?;
    (entry.build)(values, sr).with_context(|| format!("block '{kind}'"))
}

/// A block of a v1 preset: `type` plus its parameters.
#[derive(Deserialize, Debug, Clone)]
pub struct BlockEntry {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub params: ParamValues,
}

/// v1 preset: name, output volume and ordered list of blocks.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default = "default_name")]
    pub name: String,
    /// Output volume in dB, applied after the whole chain.
    #[serde(default)]
    pub output_db: f32,
    #[serde(default, rename = "block")]
    pub blocks: Vec<BlockEntry>,
}

fn default_name() -> String {
    "unnamed".into()
}

/// The old linear chain: kept only as a reference in the tests
/// (v1 presets must sound identical when converted to a graph). Only built for tests, and for
/// other crates' tests through the `legacy-chain` feature.
#[cfg(any(test, feature = "legacy-chain"))]
pub struct Chain {
    blocks: Vec<Box<dyn Block>>,
    out_gain: f32,
}

#[cfg(any(test, feature = "legacy-chain"))]
impl Chain {
    /// Builds the chain (off the audio thread): it may read files, so it can fail.
    pub fn build(preset: &Preset, sr: f32) -> Result<Chain> {
        let blocks = preset
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| build_block(&b.kind, &b.params, sr).with_context(|| format!("block #{}", i + 1)))
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

/// Coefficient of a one-pole smoothing with time constant `ms`.
pub fn one_pole_coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * sr)).exp()
    }
}

/// Value that glides smoothly toward a target: for parameters changed live.
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
    fn bundled_v1_presets_are_valid() {
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/presets")).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            // v2 presets (graphs) are covered by graph::compile's tests
            if text.contains("[[node]]") {
                continue;
            }
            let p: Preset = toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut buf = sine(220.0, 0.5, 48000);
            Chain::build(&p, SR).unwrap().process(&mut buf);
            assert!(buf.iter().all(|s| s.is_finite()), "{}", path.display());
        }
    }

    #[test]
    fn wrong_parameter_is_an_error() {
        let p: Preset = toml::from_str("[[block]]\ntype = \"drive\"\ngain = 3\n").unwrap();
        let err = Chain::build(&p, SR).err().expect("the typo `gain` (instead of gain_db) must be reported");
        assert!(format!("{err:#}").contains("gain_db"), "{err:#}");
    }

    #[test]
    fn wrong_choice_is_an_error() {
        let p: Preset = toml::from_str("[[block]]\ntype = \"drive\"\nshape = \"tube\"\n").unwrap();
        assert!(Chain::build(&p, SR).is_err());
    }

    #[test]
    fn catalog_defaults_are_valid() {
        // Every block builds with defaults only and produces no NaN.
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
            assert!(buf.iter().all(|s| s.is_finite()), "{} after set_param", entry.desc.kind);
        }
    }

    #[test]
    fn lowpass_cuts_highs() {
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
    fn drive_stays_bounded() {
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
    fn gate_closes_on_noise_and_passes_notes() {
        let mut c = chain("[[block]]\ntype = \"gate\"\nthreshold_db = -50\n");
        let mut noise = sine(3000.0, 0.001, 24000); // -60 dB
        c.process(&mut noise);
        assert!(peak(&noise[12000..]) < 1e-4);
        let mut note = sine(220.0, 0.3, 4800);
        c.process(&mut note);
        assert!(peak(&note[480..]) > 0.29);
    }

    #[test]
    fn delay_repeats_after_the_right_time() {
        let mut c = chain("[[block]]\ntype = \"delay\"\ntime_ms = 100\nfeedback = 0\nmix = 0.5\n");
        let mut buf = vec![0.0; 9600];
        buf[0] = 1.0;
        c.process(&mut buf);
        let echo_at = buf[1..].iter().position(|s| s.abs() > 0.1).unwrap() + 1;
        assert_eq!(echo_at, 4800);
    }
}
