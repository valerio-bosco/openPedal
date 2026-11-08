//! In-memory fake engine for developing and trying the interface without audio.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{bail, Context, Result};

use super::protocol::{AudioInfo, MetersMsg};
use super::EngineHandle;
use crate::dsp::params::{ParamKind, Value};
use crate::graph::load::read_pedals;
use crate::graph::model::{catalog_items, CatalogItem, EdgeSpec, GraphFile, NodeSpec, INPUT, OUTPUT};

struct Inner {
    graph: GraphFile,
    catalog: Vec<CatalogItem>,
}

pub struct MockEngine {
    pedals_dir: PathBuf,
    inner: Mutex<Inner>,
    start: Instant,
}

/// Minimal graph: the guitar wired directly to the output.
pub fn default_graph() -> GraphFile {
    let node = |id: &str, kind: &str, x: f32| NodeSpec { id: id.into(), kind: kind.into(), pos: [x, 0.0], params: Default::default() };
    GraphFile {
        name: "Nuovo".into(),
        version: 2,
        nodes: vec![node("in", INPUT, 0.0), node("out", OUTPUT, 220.0)],
        edges: vec![EdgeSpec { from: "in".into(), to: "out".into() }],
    }
}

impl MockEngine {
    pub fn new(pedals_dir: impl Into<PathBuf>, graph: GraphFile) -> Self {
        let pedals_dir = pedals_dir.into();
        let catalog = Self::load_catalog(&pedals_dir).unwrap_or_else(|_| catalog_items(&[]));
        Self { pedals_dir, inner: Mutex::new(Inner { graph, catalog }), start: Instant::now() }
    }

    fn load_catalog(dir: &Path) -> Result<Vec<CatalogItem>> {
        Ok(catalog_items(&read_pedals(dir)?))
    }
}

impl EngineHandle for MockEngine {
    fn catalog(&self) -> Vec<CatalogItem> {
        self.inner.lock().unwrap().catalog.clone()
    }

    fn graph(&self) -> GraphFile {
        self.inner.lock().unwrap().graph.clone()
    }

    fn audio(&self) -> AudioInfo {
        AudioInfo { sample_rate: 48000, buffer: 96, running: false }
    }

    fn meters(&self) -> MetersMsg {
        // Two slow, out-of-phase waves: enough to see the meters move.
        let t = self.start.elapsed().as_secs_f32();
        let in_db = -24.0 + 14.0 * (t * 1.7).sin().abs();
        let out_db = in_db + 4.0 * (t * 0.6).sin() - 2.0;
        MetersMsg { in_db, out_db, dsp: 0.05 + 0.03 * (t * 0.9).sin().abs(), xruns: 0 }
    }

    fn set_graph(&self, graph: GraphFile) -> Result<()> {
        let count = |kind: &str| graph.nodes.iter().filter(|n| n.kind == kind).count();
        if count(INPUT) != 1 || count(OUTPUT) != 1 {
            bail!("the graph must have exactly one Guitar node and one Amp node");
        }
        for e in &graph.edges {
            for id in [&e.from, &e.to] {
                if !graph.nodes.iter().any(|n| &n.id == id) {
                    bail!("the cable {} -> {} names a node that does not exist: {id}", e.from, e.to);
                }
            }
        }
        self.inner.lock().unwrap().graph = graph;
        Ok(())
    }

    fn set_param(&self, node: &str, param: &str, value: f32) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let Inner { graph, catalog } = &mut *inner;
        let spec = graph.nodes.iter_mut().find(|n| n.id == node).with_context(|| format!("node \"{node}\" does not exist"))?;
        let item = catalog.iter().find(|c| c.kind == spec.kind).with_context(|| format!("unknown node type: {}", spec.kind))?;
        let desc = item.params.iter().find(|p| p.id == param).with_context(|| format!("node \"{node}\" has no parameter \"{param}\""))?;
        // As in files: choices are saved as text, the rest as numbers.
        let stored = match desc.kind {
            ParamKind::Choice { options, .. } => match options.get(value.round().max(0.0) as usize) {
                Some(name) => Value::Text((*name).into()),
                None => bail!("choice {value} out of range for \"{param}\""),
            },
            _ => Value::Number(value),
        };
        spec.params.set(param, stored);
        Ok(())
    }

    fn reload_pedals(&self) -> Result<()> {
        let catalog = Self::load_catalog(&self.pedals_dir)?;
        self.inner.lock().unwrap().catalog = catalog;
        Ok(())
    }
}
