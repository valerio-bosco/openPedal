//! Executor of the compiled graph. Everything it needs (buffers, blocks) is
//! prepared by `compile`: `process` and `set_param` do not allocate.

use crate::dsp::{db_to_lin, Block, Smoothed};

/// Samples per node: longer buffers are processed in pieces.
pub const MAX_BLOCK: usize = 8192;

pub(super) struct OutGain(pub Smoothed);

pub(super) enum Op {
    /// The guitar signal.
    Input,
    /// Just sums its own inputs (input and output of composite pedals).
    Pass,
    Block(Box<dyn Block>),
    /// The output: sums the inputs and applies the volume.
    Output(OutGain),
}

pub(super) struct Step {
    pub op: Op,
    /// Indices of earlier steps (and thus of their buffers). A repeated index is summed twice.
    pub inputs: Box<[usize]>,
}

/// The graph ready for the audio thread, in topological order: step `i`
/// writes into buffer `i`; the last step is always the output.
pub struct Plan {
    steps: Vec<Step>,
    bufs: Vec<Vec<f32>>,
}

impl Plan {
    pub(super) fn new(steps: Vec<Step>) -> Plan {
        let bufs = steps.iter().map(|_| vec![0.0; MAX_BLOCK]).collect();
        Plan { steps, bufs }
    }

    /// Processes `buf` in place: the guitar goes in, the output sound comes out.
    pub fn process(&mut self, buf: &mut [f32]) {
        let Plan { steps, bufs } = self;
        for chunk in buf.chunks_mut(MAX_BLOCK) {
            let n = chunk.len();
            for (i, step) in steps.iter_mut().enumerate() {
                let (done, rest) = bufs.split_at_mut(i);
                let dst = &mut rest[0][..n];
                if let Op::Input = step.op {
                    dst.copy_from_slice(chunk);
                    continue;
                }
                match step.inputs.split_first() {
                    None => dst.fill(0.0),
                    Some((&first, others)) => {
                        dst.copy_from_slice(&done[first][..n]);
                        for &j in others {
                            for (d, s) in dst.iter_mut().zip(&done[j][..n]) {
                                *d += *s;
                            }
                        }
                    }
                }
                match &mut step.op {
                    Op::Block(b) => b.process(dst),
                    Op::Output(g) => {
                        let g = &mut g.0;
                        if g.value == g.target {
                            for s in dst.iter_mut() {
                                *s *= g.target;
                            }
                        } else {
                            for s in dst.iter_mut() {
                                *s *= g.next();
                            }
                        }
                    }
                    Op::Input | Op::Pass => {}
                }
            }
            if let Some(last) = bufs.last() {
                chunk.copy_from_slice(&last[..n]);
            }
        }
    }

    /// Changes a parameter live. `step` and `param` come from `NodeMap::resolve`.
    /// Out-of-range indices are ignored.
    pub fn set_param(&mut self, step: usize, param: usize, value: f32) {
        match self.steps.get_mut(step).map(|s| &mut s.op) {
            Some(Op::Block(b)) => b.set_param(param, value),
            // The output's only parameter: volume_db.
            Some(Op::Output(g)) if param == 0 => g.0.target = db_to_lin(value),
            _ => {}
        }
    }

    /// Number of steps (active nodes, including input, output and pedal junctions).
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}
