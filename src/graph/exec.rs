//! Esecutore del grafo compilato. Tutto ciò che serve (buffer, blocchi) è
//! preparato da `compile`: `process` e `set_param` non allocano.

use crate::dsp::{db_to_lin, Block, Smoothed};

/// Campioni per nodo: i buffer più lunghi si elaborano a pezzi.
pub const MAX_BLOCK: usize = 8192;

pub(super) struct OutGain(pub Smoothed);

pub(super) enum Op {
    /// Il segnale della chitarra.
    Input,
    /// Somma i propri ingressi e basta (ingresso e uscita dei pedali composti).
    Pass,
    Block(Box<dyn Block>),
    /// L'uscita: somma gli ingressi e applica il volume.
    Output(OutGain),
}

pub(super) struct Step {
    pub op: Op,
    /// Indici di passi precedenti (e quindi dei loro buffer). Un indice ripetuto si somma due volte.
    pub inputs: Box<[usize]>,
}

/// Il grafo pronto per il thread audio, in ordine topologico: il passo `i`
/// scrive nel buffer `i`; l'ultimo passo è sempre l'uscita.
pub struct Plan {
    steps: Vec<Step>,
    bufs: Vec<Vec<f32>>,
}

impl Plan {
    pub(super) fn new(steps: Vec<Step>) -> Plan {
        let bufs = steps.iter().map(|_| vec![0.0; MAX_BLOCK]).collect();
        Plan { steps, bufs }
    }

    /// Elabora `buf` sul posto: entra la chitarra, esce il suono dell'uscita.
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

    /// Cambia un parametro dal vivo. `step` e `param` arrivano da `NodeMap::resolve`.
    /// Indici fuori intervallo vengono ignorati.
    pub fn set_param(&mut self, step: usize, param: usize, value: f32) {
        match self.steps.get_mut(step).map(|s| &mut s.op) {
            Some(Op::Block(b)) => b.set_param(param, value),
            // Unico parametro dell'uscita: volume_db.
            Some(Op::Output(g)) if param == 0 => g.0.target = db_to_lin(value),
            _ => {}
        }
    }

    /// Numero di passi (nodi attivi, compresi ingresso, uscita e giunzioni dei pedali).
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}
