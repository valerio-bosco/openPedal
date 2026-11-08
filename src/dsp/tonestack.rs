//! Tone stack da ampli: bassi, medi, alti interagenti (task D4, vedi docs/PIANO.md §7).

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const TONESTACK_DESC: BlockDesc = BlockDesc {
    kind: "tonestack",
    label: "Tone stack",
    category: Category::Amp,
    params: &[
        ParamDesc::number("bass", "Bassi", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("mid", "Medi", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("treble", "Alti", 0.0, 1.0, 0.5, ""),
        ParamDesc::choice("model", "Modello", &["fender", "marshall", "vox"], 0),
    ],
};

pub fn build_tonestack(_v: &ParamValues, _sr: f32) -> Result<Box<dyn Block>> {
    Ok(Box::new(ToneStack {}))
}

// TODO(D4): filtro del 3° ordine (modello di Yeh), coefficienti ricalcolati in set_param.
pub struct ToneStack {}

impl Block for ToneStack {
    fn process(&mut self, _buf: &mut [f32]) {}
}
