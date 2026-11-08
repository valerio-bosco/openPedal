//! Riverbero Freeverb mono (task D3, vedi docs/PIANO.md §7).

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const REVERB_DESC: BlockDesc = BlockDesc {
    kind: "reverb",
    label: "Riverbero",
    category: Category::Time,
    params: &[
        ParamDesc::number("room", "Stanza", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("damp", "Smorzamento", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 0.25, ""),
        ParamDesc::number("predelay_ms", "Pre-delay", 0.0, 100.0, 10.0, "ms"),
        ParamDesc::log("width_hz", "Tono", 2000.0, 12000.0, 8000.0, "Hz"),
    ],
};

pub fn build_reverb(_v: &ParamValues, _sr: f32) -> Result<Box<dyn Block>> {
    Ok(Box::new(Reverb {}))
}

// TODO(D3): comb, allpass, pre-delay (memorie allocate solo in build_reverb).
pub struct Reverb {}

impl Block for Reverb {
    fn process(&mut self, _buf: &mut [f32]) {}
}
