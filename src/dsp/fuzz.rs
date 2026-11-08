//! Gated, sputtery transistor fuzz in the spirit of the Z.Vex Fuzz Factory
//! (task P1, see docs/PLAN.md §9).

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const FUZZ_DESC: BlockDesc = BlockDesc {
    kind: "fuzz",
    label: "Fuzz",
    category: Category::Drive,
    params: &[
        ParamDesc::number("drive", "Drive", 0.0, 1.0, 0.7, ""),
        // Starves the transistors: small signals stop conducting, notes decay into "velcro" and cut off.
        ParamDesc::number("gate", "Gate", 0.0, 1.0, 0.3, ""),
        // Low = smooth, compressed saturation; high = harder, spikier clipping.
        ParamDesc::number("comp", "Comp", 0.0, 1.0, 0.5, ""),
        // Asymmetry of the clipping (even harmonics).
        ParamDesc::number("bias", "Bias", 0.0, 1.0, 0.4, ""),
        ParamDesc::log("tone_hz", "Tone", 1000.0, 12000.0, 6000.0, "Hz"),
        ParamDesc::number("level_db", "Level", -40.0, 6.0, -12.0, "dB"),
    ],
};

pub fn build_fuzz(_v: &ParamValues, _sr: f32) -> Result<Box<dyn Block>> {
    Ok(Box::new(Fuzz {}))
}

// TODO(P1): model, oversampling, smoothing.
pub struct Fuzz {}

impl Block for Fuzz {
    fn process(&mut self, _buf: &mut [f32]) {}
}
