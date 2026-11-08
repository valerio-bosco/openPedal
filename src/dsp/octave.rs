//! Analog-style octaver: one octave down by frequency division, one octave up by
//! rectification (task P2, see docs/PLAN.md §9).

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const OCTAVE_DESC: BlockDesc = BlockDesc {
    kind: "octave",
    label: "Octave",
    category: Category::Modulation,
    params: &[
        ParamDesc::number("dry", "Dry", 0.0, 1.0, 1.0, ""),
        ParamDesc::number("down", "Down", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("up", "Up", 0.0, 1.0, 0.0, ""),
        // Low-pass on the octave voices.
        ParamDesc::log("tone_hz", "Tone", 300.0, 8000.0, 2000.0, "Hz"),
        // Low-pass on the tracking signal: lower = steadier tracking, less range.
        ParamDesc::log("track_hz", "Tracking", 200.0, 2000.0, 800.0, "Hz"),
    ],
};

pub fn build_octave(_v: &ParamValues, _sr: f32) -> Result<Box<dyn Block>> {
    Ok(Box::new(Octave {}))
}

// TODO(P2): tracking filter, flip-flop divider, rectifier, mix.
pub struct Octave {}

impl Block for Octave {
    fn process(&mut self, _buf: &mut [f32]) {}
}
