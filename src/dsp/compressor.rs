//! Compressore feed-forward (task D2, vedi docs/PIANO.md §7).

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const COMPRESSOR_DESC: BlockDesc = BlockDesc {
    kind: "compressor",
    label: "Compressore",
    category: Category::Dynamics,
    params: &[
        ParamDesc::number("threshold_db", "Soglia", -60.0, 0.0, -20.0, "dB"),
        ParamDesc::log("ratio", "Rapporto", 1.0, 20.0, 4.0, ":1"),
        ParamDesc::log("attack_ms", "Attacco", 0.1, 100.0, 5.0, "ms"),
        ParamDesc::log("release_ms", "Rilascio", 10.0, 1000.0, 120.0, "ms"),
        ParamDesc::number("knee_db", "Ginocchio", 0.0, 12.0, 6.0, "dB"),
        ParamDesc::number("makeup_db", "Guadagno", 0.0, 24.0, 0.0, "dB"),
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 1.0, ""),
    ],
};

pub fn build_compressor(_v: &ParamValues, _sr: f32) -> Result<Box<dyn Block>> {
    Ok(Box::new(Compressor {}))
}

// TODO(D2): stato del rilevatore, parametri, smoothing.
pub struct Compressor {}

impl Block for Compressor {
    fn process(&mut self, _buf: &mut [f32]) {}
}
