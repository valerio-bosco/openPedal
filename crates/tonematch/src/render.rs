//! In-process rendering, identical to `openpedal render` (blocks of 64, output clamped).

use anyhow::{Context, Result};
use openpedal_graph::model::{GraphFile, PedalFile};

/// Samples per block, as in live use.
pub const BLOCK: usize = 64;

/// Compiles the graph and runs the DI through it; returns the wet signal.
pub fn render(graph: &GraphFile, pedals: &[(String, PedalFile)], di: &[f32], sample_rate: f32) -> Result<Vec<f32>> {
    let (mut plan, _) = openpedal_graph::compile(graph, pedals, sample_rate).with_context(|| format!("preset \"{}\"", graph.name))?;
    let mut wet = di.to_vec();
    for block in wet.chunks_mut(BLOCK) {
        plan.process(block);
        for s in block.iter_mut() {
            *s = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        }
    }
    Ok(wet)
}
