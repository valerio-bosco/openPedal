//! Il motore audio vero dietro l'interfaccia: implementa `EngineHandle` usando
//! il grafo compilato (`graph::compile`) e il motore JACK (`engine::Engine`).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::dsp::params::{ParamKind, Value};
use crate::dsp::{self, lin_to_db};
use crate::engine::{Engine, Meters};
use crate::graph::model::{self, CatalogItem, GraphFile, PedalFile, OUTPUT, OUTPUT_DESC, PEDAL_PREFIX};
use crate::graph::{self, NodeMap};
use crate::server::protocol::{AudioInfo, MetersMsg};
use crate::server::EngineHandle;

struct Inner {
    engine: Engine,
    map: NodeMap,
    graph: GraphFile,
    pedals: Vec<(String, PedalFile)>,
    /// Un parametro che non si può cambiare dal vivo è stato modificato:
    /// il grafo va ricostruito (lo fa il thread di servizio, al massimo ogni 100 ms).
    rebuild: bool,
}

impl Inner {
    fn rebuild_now(&mut self, sr: f32) -> Result<()> {
        self.rebuild = false;
        let (plan, map) = graph::compile(&self.graph, &self.pedals, sr)?;
        self.engine.swap_plan(plan)?;
        self.map = map;
        Ok(())
    }
}

/// true se il parametro (di un blocco o, attraverso le manopole, di un pedale composto)
/// non può cambiare dal vivo e richiede di ricostruire il blocco.
fn needs_rebuild(kind: &str, param: &str, pedals: &[(String, PedalFile)], depth: usize) -> bool {
    if let Some(id) = kind.strip_prefix(PEDAL_PREFIX) {
        let Some((_, pedal)) = pedals.iter().find(|(p, _)| p == id) else { return false };
        let Some(knob) = pedal.knobs.iter().find(|k| k.label == param) else { return false };
        let Some((node, inner_param)) = knob.target.split_once('.') else { return false };
        let Some(spec) = pedal.nodes.iter().find(|n| n.id == node) else { return false };
        return depth < 8 && needs_rebuild(&spec.kind, inner_param, pedals, depth + 1);
    }
    // La cassa procedurale rigenera l'IR: solo il livello cambia dal vivo.
    kind == "cab" && param != "level_db"
}

pub struct LiveEngine {
    inner: Mutex<Inner>,
    meters: Arc<Meters>,
    sample_rate: f32,
    pedals_dir: PathBuf,
}

impl LiveEngine {
    /// Prende in carico un motore già avviato con il piano compilato da `graph`
    /// (`map` è la sua NodeMap) e avvia il thread che libera i piani dismessi.
    pub fn new(engine: Engine, map: NodeMap, graph: GraphFile, pedals: Vec<(String, PedalFile)>, pedals_dir: PathBuf) -> Arc<Self> {
        let live = Arc::new(Self {
            meters: engine.meters.clone(),
            sample_rate: engine.sample_rate,
            inner: Mutex::new(Inner { engine, map, graph, pedals, rebuild: false }),
            pedals_dir,
        });

        // I piani vecchi tornano dal thread audio e vanno liberati qui, non là.
        let weak = Arc::downgrade(&live);
        thread::spawn(move || {
            while let Some(live) = weak.upgrade() {
                let mut inner = live.lock();
                inner.engine.collect_garbage();
                if inner.rebuild {
                    if let Err(e) = inner.rebuild_now(live.sample_rate) {
                        eprintln!("ricostruzione del grafo fallita: {e:#}");
                    }
                }
                drop(inner);
                drop(live);
                thread::sleep(Duration::from_millis(100));
            }
        });
        live
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Un panic in un altro thread non deve spegnere l'audio dell'interfaccia.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl EngineHandle for LiveEngine {
    fn catalog(&self) -> Vec<CatalogItem> {
        model::catalog_items(&self.lock().pedals)
    }

    fn graph(&self) -> GraphFile {
        self.lock().graph.clone()
    }

    fn audio(&self) -> AudioInfo {
        AudioInfo {
            sample_rate: self.sample_rate as u32,
            buffer: self.meters.buffer_size.load(Ordering::Relaxed),
            running: true,
        }
    }

    fn meters(&self) -> MetersMsg {
        let (inp, out, load) = self.meters.take();
        MetersMsg { in_db: lin_to_db(inp), out_db: lin_to_db(out), dsp: load, xruns: self.meters.xruns.load(Ordering::Relaxed) as u64 }
    }

    fn set_graph(&self, graph: GraphFile) -> Result<()> {
        let mut inner = self.lock();
        let (plan, map) = graph::compile(&graph, &inner.pedals, self.sample_rate)?;
        inner.engine.swap_plan(plan)?;
        inner.map = map;
        inner.graph = graph;
        inner.rebuild = false;
        Ok(())
    }

    fn set_param(&self, node: &str, param: &str, value: f32) -> Result<()> {
        let mut inner = self.lock();
        // Ok(None): il nodo esiste ma non è collegato; il valore va solo salvato nel grafo.
        if let Some(change) = inner.map.resolve(node, param, value)? {
            inner.engine.set_param(change)?;
        }
        let spec = inner.graph.nodes.iter_mut().find(|n| n.id == node).ok_or_else(|| anyhow!("il nodo '{node}' non esiste"))?;
        let stored = if spec.kind.starts_with(PEDAL_PREFIX) {
            Value::Number(value.clamp(0.0, 1.0))
        } else {
            let desc = if spec.kind == OUTPUT { &OUTPUT_DESC } else { dsp::find(&spec.kind).map(|e| e.desc).ok_or_else(|| anyhow!("tipo '{}' sconosciuto", spec.kind))? };
            let p = desc.params.iter().find(|p| p.id == param).ok_or_else(|| anyhow!("'{}' non ha il parametro '{param}'", spec.kind))?;
            match p.kind {
                // Nei file le scelte si salvano per nome, come le scrive una persona.
                ParamKind::Choice { options, .. } => Value::Text(options[p.clamp(value) as usize].to_string()),
                _ => Value::Number(p.clamp(value)),
            }
        };
        spec.params.set(param, stored);
        let kind = spec.kind.clone();
        if needs_rebuild(&kind, param, &inner.pedals, 0) {
            inner.rebuild = true;
        }
        Ok(())
    }

    fn reload_pedals(&self) -> Result<()> {
        let pedals = graph::load::read_pedals(&self.pedals_dir)?;
        let mut inner = self.lock();
        // Un pedale modificato può cambiare il grafo che sta suonando: lo si ricompila.
        let old = std::mem::replace(&mut inner.pedals, pedals);
        if let Err(e) = inner.rebuild_now(self.sample_rate) {
            inner.pedals = old;
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pedals() -> Vec<(String, PedalFile)> {
        graph::load::read_pedals(std::path::Path::new("pedals")).unwrap()
    }

    #[test]
    fn la_cassa_procedurale_si_ricostruisce() {
        let p = pedals();
        assert!(needs_rebuild("cab", "high_hz", &p, 0));
        assert!(!needs_rebuild("cab", "level_db", &p, 0));
        assert!(!needs_rebuild("drive", "gain_db", &p, 0));
        // la manopola "Brillantezza" del pedale punta a c.high_hz di una cassa
        assert!(needs_rebuild("pedal:cassa-4x12", "Brillantezza", &p, 0));
        assert!(!needs_rebuild("pedal:inesistente", "x", &p, 0));
    }
}
