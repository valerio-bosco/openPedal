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

/// Bypass state of a step: `wet` (1 = processed, 0 = dry) glides towards `target`
/// at `inc` per sample, so switching is click-free.
pub(super) struct Bypass {
    pub wet: f32,
    pub target: f32,
    pub inc: f32,
    /// Step whose buffer is the dry signal (the input junction of a composite pedal).
    /// `None` for plain blocks: the dry signal is their own summed input.
    pub dry: Option<usize>,
}

impl Bypass {
    pub(super) fn new(bypassed: bool, dry: Option<usize>, sr: f32) -> Bypass {
        let w = if bypassed { 0.0 } else { 1.0 };
        Bypass { wet: w, target: w, inc: 1.0 / (BYPASS_SECONDS * sr).max(1.0), dry }
    }
}

/// Crossfade time between processed and dry.
const BYPASS_SECONDS: f32 = 0.010;

pub(super) struct Step {
    pub op: Op,
    /// `None` = cannot be bypassed (input, output, plain junctions).
    pub bypass: Option<Bypass>,
    /// Indices of earlier steps (and thus of their buffers). A repeated index is summed twice.
    pub inputs: Box<[usize]>,
}

/// `out` holds the processed signal and `dry` the unprocessed one. Result in `out`.
fn crossfade(by: &mut Bypass, out: &mut [f32], dry: &[f32]) {
    for (o, d) in out.iter_mut().zip(dry) {
        if by.wet < by.target {
            by.wet = (by.wet + by.inc).min(by.target);
        } else if by.wet > by.target {
            by.wet = (by.wet - by.inc).max(by.target);
        }
        *o = *d + (*o - *d) * by.wet;
    }
}

/// The graph ready for the audio thread, in topological order: step `i`
/// writes into buffer `i`; the last step is always the output.
pub struct Plan {
    steps: Vec<Step>,
    bufs: Vec<Vec<f32>>,
    /// Copy of a block's dry input while it crossfades.
    scratch: Vec<f32>,
}

impl Plan {
    pub(super) fn new(steps: Vec<Step>) -> Plan {
        let bufs = steps.iter().map(|_| vec![0.0; MAX_BLOCK]).collect();
        Plan { steps, bufs, scratch: vec![0.0; MAX_BLOCK] }
    }

    /// Processes `buf` in place: the guitar goes in, the output sound comes out.
    pub fn process(&mut self, buf: &mut [f32]) {
        let Plan { steps, bufs, scratch } = self;
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
                    Op::Block(b) => match &mut step.bypass {
                        None => b.process(dst),
                        Some(by) if by.wet == 1.0 && by.target == 1.0 => b.process(dst),
                        // Fully bypassed: the summed input is already the output.
                        Some(by) if by.wet == 0.0 && by.target == 0.0 => {}
                        Some(by) => {
                            let dry = &mut scratch[..n];
                            dry.copy_from_slice(dst);
                            b.process(dst);
                            crossfade(by, dst, dry);
                        }
                    },
                    Op::Pass => {
                        if let Some(by) = &mut step.bypass {
                            if let Some(d) = by.dry {
                                let dry = &done[d][..n];
                                if by.wet == 0.0 && by.target == 0.0 {
                                    dst.copy_from_slice(dry);
                                } else if by.wet != 1.0 || by.target != 1.0 {
                                    crossfade(by, dst, dry);
                                }
                            }
                        }
                    }
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
                    Op::Input => {}
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

    /// Bypasses (or re-enables) a step live, with a short crossfade. Steps that cannot
    /// be bypassed and out-of-range indices are ignored. `step` comes from `NodeMap::resolve_bypass`.
    pub fn set_bypass(&mut self, step: usize, bypass: bool) {
        if let Some(by) = self.steps.get_mut(step).and_then(|s| s.bypass.as_mut()) {
            by.target = if bypass { 0.0 } else { 1.0 };
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
