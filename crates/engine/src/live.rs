//! The real audio engine behind the interface: implements `EngineHandle` using
//! the compiled graph (`graph::compile`) and the audio engine (`engine::Engine`, any backend).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};

use openpedal_dsp::params::{ParamKind, Value};
use openpedal_dsp::{self as dsp, lin_to_db};
use crate::engine::{Engine, Meters};
use openpedal_graph::model::{self, CatalogItem, GraphFile, PedalFile, OUTPUT, OUTPUT_DESC, PEDAL_PREFIX};
use openpedal_graph::{self as graph, NodeMap};
use openpedal_protocol::{AudioInfo, MetersMsg};
use crate::EngineHandle;

struct Inner {
    engine: Engine,
    map: NodeMap,
    graph: GraphFile,
    pedals: Vec<(String, PedalFile)>,
    /// A parameter that cannot be changed live has been modified:
    /// the graph must be rebuilt (the service thread does it, at most every 100 ms).
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

/// true if the parameter (of a block, or through the knobs, of a composite pedal)
/// cannot change live and requires rebuilding the block.
fn needs_rebuild(kind: &str, param: &str, pedals: &[(String, PedalFile)], depth: usize) -> bool {
    if let Some(id) = kind.strip_prefix(PEDAL_PREFIX) {
        let Some((_, pedal)) = pedals.iter().find(|(p, _)| p == id) else { return false };
        let Some(knob) = pedal.knobs.iter().find(|k| k.label == param) else { return false };
        let Some((node, inner_param)) = knob.target.split_once('.') else { return false };
        let Some(spec) = pedal.nodes.iter().find(|n| n.id == node) else { return false };
        return depth < 8 && needs_rebuild(&spec.kind, inner_param, pedals, depth + 1);
    }
    // The procedural cabinet regenerates the IR: only the level changes live.
    kind == "cab" && param != "level_db"
}

pub struct LiveEngine {
    inner: Mutex<Inner>,
    meters: Arc<Meters>,
    sample_rate: f32,
    pedals_dir: PathBuf,
}

impl LiveEngine {
    /// Takes over an engine already started with the plan compiled from `graph`
    /// (`map` is its NodeMap) and starts the thread that frees the retired plans.
    pub fn new(engine: Engine, map: NodeMap, graph: GraphFile, pedals: Vec<(String, PedalFile)>, pedals_dir: PathBuf) -> Arc<Self> {
        let live = Arc::new(Self {
            meters: engine.meters.clone(),
            sample_rate: engine.sample_rate,
            inner: Mutex::new(Inner { engine, map, graph, pedals, rebuild: false }),
            pedals_dir,
        });

        // Old plans come back from the audio thread and must be freed here, not there.
        let weak = Arc::downgrade(&live);
        thread::spawn(move || {
            while let Some(live) = weak.upgrade() {
                let mut inner = live.lock();
                inner.engine.collect_garbage();
                if inner.rebuild {
                    if let Err(e) = inner.rebuild_now(live.sample_rate) {
                        eprintln!("graph rebuild failed: {e:#}");
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
        // A panic in another thread must not silence the interface's audio.
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

    fn set_graph_name(&self, name: &str) {
        self.lock().graph.name = name.to_string();
    }

    fn set_param(&self, node: &str, param: &str, value: f32) -> Result<()> {
        let mut inner = self.lock();
        // No changes: the node exists but is not connected; the value is only saved in the graph.
        // A control can drive several parameters (one change per route).
        for change in inner.map.resolve_changes(node, param, value)? {
            inner.engine.set_param(change)?;
        }
        let spec = inner.graph.nodes.iter_mut().find(|n| n.id == node).ok_or_else(|| anyhow!("node '{node}' does not exist"))?;
        let stored = if spec.kind.starts_with(PEDAL_PREFIX) {
            Value::Number(value.clamp(0.0, 1.0))
        } else {
            let desc = if spec.kind == OUTPUT { &OUTPUT_DESC } else { dsp::find(&spec.kind).map(|e| e.desc).ok_or_else(|| anyhow!("unknown type '{}'", spec.kind))? };
            let p = desc.params.iter().find(|p| p.id == param).ok_or_else(|| anyhow!("'{}' has no parameter '{param}'", spec.kind))?;
            match p.kind {
                // In files, choices are saved by name, as a person would write them.
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

    fn set_bypass(&self, node: &str, bypass: bool) -> Result<()> {
        let mut inner = self.lock();
        if !inner.graph.nodes.iter().any(|n| n.id == node) {
            return Err(anyhow!("node '{node}' does not exist"));
        }
        // Ok(None): the node is not connected; the flag is only saved in the graph.
        if let Some(step) = inner.map.resolve_bypass(node)? {
            inner.engine.set_bypass(step, bypass)?;
        }
        if let Some(spec) = inner.graph.nodes.iter_mut().find(|n| n.id == node) {
            spec.bypass = bypass;
        }
        Ok(())
    }

    fn reload_pedals(&self) -> Result<()> {
        let pedals = graph::load::read_pedals(&self.pedals_dir)?;
        let mut inner = self.lock();
        // A modified pedal can change the graph that is playing: it gets recompiled.
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
        graph::load::read_pedals(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/pedals"))).unwrap()
    }

    #[test]
    fn procedural_cab_is_rebuilt() {
        let p = pedals();
        assert!(needs_rebuild("cab", "high_hz", &p, 0));
        assert!(!needs_rebuild("cab", "level_db", &p, 0));
        assert!(!needs_rebuild("drive", "gain_db", &p, 0));
        // the pedal's "Brightness" knob points to c.high_hz of a cabinet
        assert!(needs_rebuild("pedal:cab-4x12", "Brightness", &p, 0));
        assert!(!needs_rebuild("pedal:missing", "x", &p, 0));
    }
}
