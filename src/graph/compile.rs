//! Compiler: expands composite pedals, checks the graph, sorts it and
//! builds the `Plan`. Runs on the control thread (it can allocate and read files).

// The parameter translation API will be used by the integration (I2, `EngineHandle`).
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use anyhow::{bail, Context, Result};

use super::exec::{Bypass, Op, OutGain, Plan, Step};
use super::model::*;
use crate::dsp::{self, db_to_lin, BlockDesc, Entry, ParamKind, ParamValues, Smoothed, Value};

/// Graph error, with the (top-level) node it refers to, if any:
/// the interface can highlight it (`anyhow::Error::downcast_ref::<GraphError>()`).
#[derive(Debug, Clone)]
pub struct GraphError {
    pub message: String,
    pub node: Option<String>,
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GraphError {}

fn err(node: Option<&str>, message: impl Into<String>) -> anyhow::Error {
    GraphError { message: message.into(), node: node.map(String::from) }.into()
}

/// A parameter change already translated into `Plan` indices, ready for the audio thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamChange {
    pub step: usize,
    pub param: usize,
    pub value: f32,
}

/// Where an exposed knob ends up: the parameter of a block, through
/// one or more 0..1 -> min..max mapping levels (more levels if pedals are nested).
#[derive(Debug, Clone)]
struct Binding {
    flat: usize,
    desc: &'static BlockDesc,
    param: usize,
    levels: Vec<(f32, f32)>,
}

impl Binding {
    fn apply(&self, value: f32) -> f32 {
        self.levels.iter().fold(value, |v, &(min, max)| min + (max - min) * v.clamp(0.0, 1.0))
    }
}

enum Slot {
    /// Node that corresponds to a single block (including `input` and `output`).
    Block { flat: usize, desc: &'static BlockDesc },
    /// `exit` is the flat index of the pedal's output junction (where its bypass lives).
    Pedal { knobs: BTreeMap<String, Binding>, exit: usize },
}

/// Translates (node id, parameter id) into indices for `Plan::set_param`.
/// Kept by the control thread; only valid for the `Plan` it was created with.
pub struct NodeMap {
    slots: HashMap<String, Slot>,
    /// Flat node -> plan step (None = discarded because it does not lead to the output).
    flat_to_step: Vec<Option<usize>>,
}

impl NodeMap {
    /// `Ok(None)`: the node exists but is not in the plan (it is not wired between guitar and output),
    /// so there is nothing to change. Error: node or parameter does not exist, or a parameter
    /// that requires rebuilding the graph (a file).
    /// For composite pedals `param` is the knob label and `value` goes from 0 to 1.
    pub fn resolve(&self, node: &str, param: &str, value: f32) -> Result<Option<ParamChange>> {
        let Some(slot) = self.slots.get(node) else {
            return Err(err(Some(node), format!("node '{node}' does not exist")));
        };
        let (flat, index, value) = match slot {
            Slot::Block { flat, desc } => {
                let Some(index) = desc.param_index(param) else {
                    let valid: Vec<_> = desc.params.iter().map(|p| p.id).collect();
                    let msg = format!("node '{node}' ({}) has no parameter '{param}' (valid: {})", desc.kind, valid.join(", "));
                    return Err(err(Some(node), msg));
                };
                let pd = &desc.params[index];
                if let ParamKind::File { .. } = pd.kind {
                    return Err(err(Some(node), format!("'{node}.{param}' can only be changed by rebuilding the graph")));
                }
                (*flat, index, pd.clamp(value))
            }
            Slot::Pedal { knobs, .. } => {
                let Some(b) = knobs.get(param) else {
                    let valid: Vec<_> = knobs.keys().map(String::as_str).collect();
                    let msg = format!("pedal '{node}' has no knob '{param}' (knobs: {})", valid.join(", "));
                    return Err(err(Some(node), msg));
                };
                (b.flat, b.param, b.desc.params[b.param].clamp(b.apply(value)))
            }
        };
        Ok(self.flat_to_step[flat].map(|step| ParamChange { step, param: index, value }))
    }
}

impl NodeMap {
    /// Plan step to bypass for `node`. `Ok(None)`: the node is not in the plan (not wired
    /// between guitar and output). Error: unknown node, or `input`/`output`.
    pub fn resolve_bypass(&self, node: &str) -> Result<Option<usize>> {
        let Some(slot) = self.slots.get(node) else {
            return Err(err(Some(node), format!("node '{node}' does not exist")));
        };
        let flat = match slot {
            Slot::Block { desc, .. } if desc.kind == INPUT || desc.kind == OUTPUT => {
                return Err(err(Some(node), format!("node '{node}' ({}) cannot be bypassed", desc.kind)));
            }
            Slot::Block { flat, .. } => *flat,
            Slot::Pedal { exit, .. } => *exit,
        };
        Ok(self.flat_to_step[flat])
    }
}

// ---------- espansione ----------

enum FlatKind {
    Input,
    Output,
    Pass,
    Block(&'static Entry),
}

struct FlatNode {
    /// Readable path: "speaker/c" for a node inside a pedal.
    path: String,
    kind: FlatKind,
    params: ParamValues,
    bypass: bool,
    /// For a pedal's output junction: its input junction, the dry signal when bypassed.
    dry: Option<usize>,
}

/// How a level is wired to its nodes: cables enter at `entry` and leave from `exit`.
struct Expanded {
    entry: usize,
    exit: usize,
    knobs: BTreeMap<String, Binding>,
}

struct Expander<'a> {
    pedals: &'a [(String, PedalFile)],
    nodes: Vec<FlatNode>,
    edges: Vec<(usize, usize)>,
}

impl Expander<'_> {
    fn push(&mut self, path: String, kind: FlatKind, params: ParamValues) -> usize {
        self.nodes.push(FlatNode { path, kind, params, bypass: false, dry: None });
        self.nodes.len() - 1
    }

    /// Expands a level (the main graph or the body of a pedal).
    /// `implicit` = `pedal_in`/`pedal_out` junctions, only inside pedals.
    /// `top` = top-level node to report in errors.
    fn body(
        &mut self,
        nodes: &[NodeSpec],
        edges: &[EdgeSpec],
        prefix: &str,
        top: Option<&str>,
        stack: &mut Vec<String>,
        implicit: Option<(usize, usize)>,
    ) -> Result<HashMap<String, Expanded>> {
        let mut map: HashMap<String, Expanded> = HashMap::new();
        let mut kinds: HashMap<&str, &str> = HashMap::new();
        if let Some((i, o)) = implicit {
            map.insert(PEDAL_IN.into(), Expanded { entry: i, exit: i, knobs: BTreeMap::new() });
            map.insert(PEDAL_OUT.into(), Expanded { entry: o, exit: o, knobs: BTreeMap::new() });
        }
        for spec in nodes {
            let node = top.or(Some(spec.id.as_str()));
            if spec.id.is_empty() {
                return Err(err(node, "a node has an empty id"));
            }
            if map.contains_key(&spec.id) {
                return Err(err(node, format!("the id '{}' is used by more than one node (or is reserved)", spec.id)));
            }
            kinds.insert(&spec.id, &spec.kind);
            let path = if prefix.is_empty() { spec.id.clone() } else { format!("{prefix}/{}", spec.id) };
            let exp = match spec.kind.as_str() {
                INPUT | OUTPUT if top.is_some() || implicit.is_some() => {
                    return Err(err(node, format!("'{}' cannot be inside a composite pedal", spec.kind)));
                }
                INPUT => self.leaf(path, FlatKind::Input, &INPUT_DESC, spec)?,
                OUTPUT => self.leaf(path, FlatKind::Output, &OUTPUT_DESC, spec)?,
                kind if kind.starts_with(PEDAL_PREFIX) => {
                    self.pedal(&kind[PEDAL_PREFIX.len()..], spec, &path, node.unwrap_or(&spec.id), stack)?
                }
                kind => {
                    let Some(entry) = dsp::find(kind) else {
                        let valid: Vec<_> = dsp::catalog().iter().map(|e| e.desc.kind).collect();
                        let msg = format!("node '{}' has an unknown type '{kind}' (valid types: {})", spec.id, valid.join(", "));
                        return Err(err(node, msg));
                    };
                    self.leaf(path, FlatKind::Block(entry), entry.desc, spec)?
                }
            };
            map.insert(spec.id.clone(), exp);
        }

        let mut seen = HashSet::new();
        for e in edges {
            let node = top.or(Some(e.from.as_str()));
            let missing = |id: &str| err(node, format!("cable {} -> {} uses node '{id}', which does not exist", e.from, e.to));
            let from = map.get(&e.from).ok_or_else(|| missing(&e.from))?;
            let to = map.get(&e.to).ok_or_else(|| missing(&e.to))?;
            let from_is_sink = e.from == PEDAL_OUT || kinds.get(e.from.as_str()) == Some(&OUTPUT);
            let to_is_source = e.to == PEDAL_IN || kinds.get(e.to.as_str()) == Some(&INPUT);
            if from_is_sink {
                return Err(err(node, format!("cables cannot leave output '{}'", e.from)));
            }
            if to_is_source {
                return Err(err(node, format!("cables cannot arrive at input '{}'", e.to)));
            }
            if !seen.insert((e.from.as_str(), e.to.as_str())) {
                return Err(err(node, format!("cable {} -> {} is repeated", e.from, e.to)));
            }
            self.edges.push((from.exit, to.entry));
        }
        Ok(map)
    }

    fn leaf(&mut self, path: String, kind: FlatKind, desc: &'static BlockDesc, spec: &NodeSpec) -> Result<Expanded> {
        spec.params.validate(desc).map_err(|e| err(Some(&spec.id), format!("{e:#}")))?;
        if spec.bypass && matches!(kind, FlatKind::Input | FlatKind::Output) {
            return Err(err(Some(&spec.id), format!("node '{}' ({}) cannot be bypassed", spec.id, desc.kind)));
        }
        let i = self.push(path, kind, spec.params.clone());
        self.nodes[i].bypass = spec.bypass;
        Ok(Expanded { entry: i, exit: i, knobs: BTreeMap::new() })
    }

    /// Expands a `pedal:<id>` node: the knobs set the internal parameters.
    fn pedal(&mut self, id: &str, spec: &NodeSpec, path: &str, top: &str, stack: &mut Vec<String>) -> Result<Expanded> {
        if stack.iter().any(|s| s == id) {
            let chain: Vec<_> = stack.iter().map(String::as_str).chain([id]).collect();
            return Err(err(Some(top), format!("pedal '{id}' contains itself: {}", chain.join(" -> "))));
        }
        let pedals = self.pedals;
        let Some((_, pedal)) = pedals.iter().find(|(pid, _)| pid == id) else {
            return Err(err(Some(top), format!("pedal '{id}' does not exist (missing pedals/{id}.toml)")));
        };

        for name in spec.params.0.keys() {
            if !pedal.knobs.iter().any(|k| &k.label == name) {
                let valid: Vec<_> = pedal.knobs.iter().map(|k| k.label.as_str()).collect();
                let msg = format!("pedal '{id}' has no knob '{name}' (knobs: {})", valid.join(", "));
                return Err(err(Some(top), msg));
            }
        }

        // The knobs write their value into the internal parameter before building.
        let mut inner = pedal.nodes.clone();
        for k in &pedal.knobs {
            let (target, param) = k.target.split_once('.').ok_or_else(|| {
                err(Some(top), format!("pedal '{id}', knob '{}': target '{}' must be 'node.parameter'", k.label, k.target))
            })?;
            let Some(node) = inner.iter_mut().find(|n| n.id == target) else {
                return Err(err(Some(top), format!("pedal '{id}', knob '{}': node '{target}' does not exist", k.label)));
            };
            let v = match spec.params.get(&k.label) {
                Some(Value::Number(v)) => *v,
                Some(Value::Text(_)) => return Err(err(Some(top), format!("the knob '{}' must be a number", k.label))),
                None => k.default,
            };
            node.params.set(param, Value::Number(k.min + (k.max - k.min) * v.clamp(0.0, 1.0)));
        }

        let pin = self.push(format!("{path}/{PEDAL_IN}"), FlatKind::Pass, ParamValues::default());
        let pout = self.push(format!("{path}/{PEDAL_OUT}"), FlatKind::Pass, ParamValues::default());
        stack.push(id.to_string());
        let map = self.body(&inner, &pedal.edges, path, Some(top), stack, Some((pin, pout)));
        stack.pop();
        let map = map.with_context(|| format!("pedal '{id}'"))?;
        self.nodes[pout].bypass = spec.bypass;
        self.nodes[pout].dry = Some(pin);

        let mut knobs = BTreeMap::new();
        for k in &pedal.knobs {
            let (target, param) = k.target.split_once('.').expect("already checked");
            let exp = &map[target];
            let level = (k.min, k.max);
            let binding = match &self.nodes[exp.entry].kind {
                FlatKind::Block(entry) => {
                    let Some(index) = entry.desc.param_index(param) else {
                        let msg = format!("pedal '{id}', knob '{}': '{target}' ({}) has no parameter '{param}'", k.label, entry.desc.kind);
                        return Err(err(Some(top), msg));
                    };
                    if !matches!(entry.desc.params[index].kind, ParamKind::Number { .. }) {
                        let msg = format!("pedal '{id}', knob '{}': '{}' is not numeric", k.label, k.target);
                        return Err(err(Some(top), msg));
                    }
                    Binding { flat: exp.entry, desc: entry.desc, param: index, levels: vec![level] }
                }
                // Target = another pedal: `param` is one of its knobs, already checked in `body`.
                _ => {
                    let Some(inner_binding) = exp.knobs.get(param) else {
                        return Err(err(Some(top), format!("pedal '{id}', knob '{}': '{target}' has no knob '{param}'", k.label)));
                    };
                    let mut levels = vec![level];
                    levels.extend(&inner_binding.levels);
                    Binding { levels, ..inner_binding.clone() }
                }
            };
            knobs.insert(k.label.clone(), binding);
        }
        Ok(Expanded { entry: pin, exit: pout, knobs })
    }
}

// ---------- compilazione ----------

/// Compiles a graph (with the available composite pedals) into a `Plan` and a `NodeMap`
/// that translates parameter changes. It can read files (cab) and fail: it must not be called
/// from the audio thread.
pub fn compile(graph: &GraphFile, pedals: &[(String, PedalFile)], sr: f32) -> Result<(Plan, NodeMap)> {
    let mut ex = Expander { pedals, nodes: Vec::new(), edges: Vec::new() };
    let top = ex.body(&graph.nodes, &graph.edges, "", None, &mut Vec::new(), None)?;
    let Expander { nodes, edges, .. } = ex;

    let inputs: Vec<_> = (0..nodes.len()).filter(|&i| matches!(nodes[i].kind, FlatKind::Input)).collect();
    let outputs: Vec<_> = (0..nodes.len()).filter(|&i| matches!(nodes[i].kind, FlatKind::Output)).collect();
    match (inputs.len(), outputs.len()) {
        (1, 1) => {}
        (0, _) => bail!("the 'input' node (the guitar) is missing"),
        (_, 0) => bail!("the 'output' node (the amp) is missing"),
        (n, 1) if n > 1 => bail!("the graph has {n} 'input' nodes: only one is needed"),
        (_, n) => bail!("the graph has {n} 'output' nodes: only one is needed"),
    }
    let (input, output) = (inputs[0], outputs[0]);

    let n = nodes.len();
    let mut succ = vec![Vec::new(); n];
    let mut pred = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in &edges {
        succ[a].push(b);
        pred[b].push(a);
        indeg[b] += 1;
    }

    // Topological sort (Kahn); whatever is left over is part of a cycle or depends on one.
    let mut order = Vec::with_capacity(n);
    let mut left = indeg.clone();
    let mut ready: Vec<usize> = (0..n).filter(|&i| left[i] == 0).collect();
    ready.reverse();
    while let Some(i) = ready.pop() {
        order.push(i);
        for &j in &succ[i] {
            left[j] -= 1;
            if left[j] == 0 {
                ready.push(j);
            }
        }
    }
    if order.len() < n {
        return Err(cycle_error(&nodes, &pred, &left));
    }

    // Keep what is reachable from the guitar and leads to the output (the output is always kept).
    let from_input = reach(input, &succ);
    let to_output = reach(output, &pred);
    let keep = |i: usize| i == output || (from_input[i] && to_output[i]);

    let mut flat_to_step = vec![None; n];
    let mut steps: Vec<Step> = Vec::new();
    for &i in order.iter().filter(|&&i| keep(i)) {
        let node = &nodes[i];
        let op = match &node.kind {
            FlatKind::Input => Op::Input,
            FlatKind::Pass => Op::Pass,
            FlatKind::Output => {
                let db = node.params.number(&OUTPUT_DESC.params[0]);
                Op::Output(OutGain(Smoothed::new(db_to_lin(db), 15.0, sr)))
            }
            FlatKind::Block(entry) => {
                let block = dsp::build_block(entry.desc.kind, &node.params, sr)
                    .map_err(|e| err(Some(&top_of(&node.path)), format!("node '{}': {e:#}", node.path)))?;
                Op::Block(block)
            }
        };
        let inputs = pred[i].iter().filter(|&&p| keep(p)).map(|&p| flat_to_step[p].expect("ordine topologico")).collect();
        let bypass = match &node.kind {
            FlatKind::Block(_) => Some(Bypass::new(node.bypass, None, sr)),
            // A pedal's output junction: the dry signal is its input junction, if that comes earlier.
            FlatKind::Pass => node.dry.and_then(|d| flat_to_step[d]).map(|d| Bypass::new(node.bypass, Some(d), sr)),
            _ => None,
        };
        flat_to_step[i] = Some(steps.len());
        steps.push(Step { op, inputs, bypass });
    }
    debug_assert!(matches!(steps.last().map(|s| &s.op), Some(Op::Output(_))));

    let mut slots = HashMap::new();
    for spec in &graph.nodes {
        let exp = &top[&spec.id];
        let slot = match &nodes[exp.entry].kind {
            FlatKind::Input => Slot::Block { flat: exp.entry, desc: &INPUT_DESC },
            FlatKind::Output => Slot::Block { flat: exp.entry, desc: &OUTPUT_DESC },
            FlatKind::Block(entry) => Slot::Block { flat: exp.entry, desc: entry.desc },
            FlatKind::Pass => Slot::Pedal { knobs: exp.knobs.clone(), exit: exp.exit },
        };
        slots.insert(spec.id.clone(), slot);
    }
    Ok((Plan::new(steps), NodeMap { slots, flat_to_step }))
}

fn top_of(path: &str) -> String {
    path.split('/').next().unwrap_or(path).to_string()
}

/// Nodes reachable from `start` by following `next`.
fn reach(start: usize, next: &[Vec<usize>]) -> Vec<bool> {
    let mut seen = vec![false; next.len()];
    let mut stack = vec![start];
    seen[start] = true;
    while let Some(i) = stack.pop() {
        for &j in &next[i] {
            if !seen[j] {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    seen
}

/// Finds a cycle among the nodes the sort could not place, in order to describe it.
fn cycle_error(nodes: &[FlatNode], pred: &[Vec<usize>], left: &[usize]) -> anyhow::Error {
    // Every remaining node has a remaining predecessor: walking back always ends in a cycle.
    let start = (0..nodes.len()).find(|&i| left[i] > 0).expect("a remaining node exists");
    let mut path = vec![start];
    let cycle = loop {
        let cur = *path.last().unwrap();
        let prev = *pred[cur].iter().find(|&&p| left[p] > 0).expect("remaining predecessor");
        if let Some(pos) = path.iter().position(|&p| p == prev) {
            let mut c = path[pos..].to_vec();
            c.reverse();
            break c;
        }
        path.push(prev);
    };
    let mut names: Vec<_> = cycle.iter().filter(|&&i| !matches!(nodes[i].kind, FlatKind::Pass)).map(|&i| nodes[i].path.clone()).collect();
    if names.is_empty() {
        names = cycle.iter().map(|&i| nodes[i].path.clone()).collect();
    }
    names.push(names[0].clone());
    err(Some(&top_of(&names[0])), format!("the graph contains a cycle: {}", names.join(" -> ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{Chain, Preset};
    use crate::graph::load::{parse_graph, read_pedals};
    use std::path::Path;

    const SR: f32 = 48000.0;

    /// Deterministic signal: a sine plus a bit of noise.
    fn signal(n: usize) -> Vec<f32> {
        let mut seed = 12345u32;
        (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                0.4 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() + 0.05 * noise
            })
            .collect()
    }

    fn run(plan: &mut Plan, input: &[f32]) -> Vec<f32> {
        let mut out = input.to_vec();
        for block in out.chunks_mut(64) {
            plan.process(block);
        }
        out
    }

    fn pedal(text: &str) -> PedalFile {
        toml::from_str(text).unwrap()
    }

    fn build(graph: &str, pedals: &[(String, PedalFile)]) -> Result<(Plan, NodeMap)> {
        compile(&parse_graph(graph).unwrap(), pedals, SR)
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn v1_presets_sound_like_the_old_chain() {
        let input = signal(48000);
        let mut count = 0;
        for entry in std::fs::read_dir("presets").unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            if text.contains("[[node]]") {
                continue; // already a graph: nothing to compare with
            }
            let preset: Preset = toml::from_str(&text).unwrap();
            let mut chain = Chain::build(&preset, SR).unwrap();
            let mut old = input.clone();
            for block in old.chunks_mut(64) {
                chain.process(block);
            }
            let (mut plan, _) = compile(&crate::graph::load::read_graph(&path).unwrap(), &[], SR).unwrap();
            let new = run(&mut plan, &input);
            let same = old.iter().zip(&new).all(|(a, b)| a.to_bits() == b.to_bits());
            assert!(same, "{} sounds different", path.display());
            count += 1;
        }
        assert!(count >= 5);
    }

    #[test]
    fn example_graph_makes_sound() {
        let pedals = read_pedals(Path::new("pedals")).unwrap();
        let g = crate::graph::load::read_graph(Path::new("docs/examples/crunch-graph.toml")).unwrap();
        let (mut plan, _) = compile(&g, &pedals, SR).unwrap();
        let out = run(&mut plan, &signal(24000));
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(peak(&out) > 0.01);
    }

    #[test]
    fn fan_in_sums_and_fan_out_copies() {
        let g = r#"
name = "p"
[[node]]
id = "in"
type = "input"
[[node]]
id = "a"
type = "gain"
params = { db = -6 }
[[node]]
id = "b"
type = "gain"
params = { db = 0 }
[[node]]
id = "out"
type = "output"
[[edge]]
from = "in"
to = "a"
[[edge]]
from = "in"
to = "b"
[[edge]]
from = "a"
to = "out"
[[edge]]
from = "b"
to = "out"
"#;
        let (mut plan, _) = build(g, &[]).unwrap();
        let input = signal(4800);
        let out = run(&mut plan, &input);
        let g6 = crate::dsp::db_to_lin(-6.0);
        for (x, y) in input.iter().zip(&out) {
            assert!((y - x * (g6 + 1.0)).abs() < 1e-5);
        }
    }

    #[test]
    fn a_cycle_is_rejected() {
        let g = r#"
name = "p"
[[node]]
id = "in"
type = "input"
[[node]]
id = "a"
type = "gain"
[[node]]
id = "b"
type = "gain"
[[node]]
id = "out"
type = "output"
[[edge]]
from = "in"
to = "a"
[[edge]]
from = "a"
to = "b"
[[edge]]
from = "b"
to = "a"
[[edge]]
from = "b"
to = "out"
"#;
        let e = build(g, &[]).err().unwrap();
        let msg = format!("{e:#}");
        assert!(msg.contains("cycle") && (msg.contains("a -> b -> a") || msg.contains("b -> a -> b")), "{msg}");
        let ge = e.downcast_ref::<GraphError>().unwrap();
        assert!(ge.node.is_some());
    }

    const INNER: &str = r#"
name = "Interno"
[[node]]
id = "g"
type = "gain"
[[edge]]
from = "pedal_in"
to = "g"
[[edge]]
from = "g"
to = "pedal_out"
[[knob]]
label = "Level"
target = "g.db"
min = -20
max = 0
default = 0.5
"#;

    const OUTER: &str = r#"
name = "Esterno"
[[node]]
id = "i"
type = "pedal:inner"
[[edge]]
from = "pedal_in"
to = "i"
[[edge]]
from = "i"
to = "pedal_out"
[[knob]]
label = "Master"
target = "i.Level"
min = 1
max = 0
default = 0
"#;

    fn nested_pedals() -> Vec<(String, PedalFile)> {
        vec![("inner".into(), pedal(INNER)), ("outer".into(), pedal(OUTER))]
    }

    const USE_OUTER: &str = r#"
name = "p"
[[node]]
id = "in"
type = "input"
[[node]]
id = "p"
type = "pedal:outer"
params = { Master = 1 }
[[node]]
id = "out"
type = "output"
[[edge]]
from = "in"
to = "p"
[[edge]]
from = "p"
to = "out"
"#;

    #[test]
    fn nested_pedal_is_expanded_and_knobs_work_live() {
        let (mut plan, map) = build(USE_OUTER, &nested_pedals()).unwrap();
        let input = signal(9600);
        // Master = 1 -> internal Level 0 -> -20 dB.
        let out = run(&mut plan, &input);
        let expected = crate::dsp::db_to_lin(-20.0);
        assert!((out[5000] - input[5000] * expected).abs() < 1e-5);

        // Live: Master = 0 -> internal Level 1 -> 0 dB.
        let change = map.resolve("p", "Master", 0.0).unwrap().unwrap();
        assert_eq!(change.value, 0.0);
        plan.set_param(change.step, change.param, change.value);
        let input = signal(48000);
        let out = run(&mut plan, &input);
        assert!((out[47000] - input[47000]).abs() < 1e-3);
    }

    #[test]
    fn knob_without_value_uses_default() {
        let g = USE_OUTER.replace("params = { Master = 1 }\n", "");
        let (mut plan, _) = build(&g, &nested_pedals()).unwrap();
        let input = signal(9600);
        let out = run(&mut plan, &input);
        // Master = 0 (outer default) -> Level 1 -> 0 dB.
        assert!((out[5000] - input[5000]).abs() < 1e-5);
    }

    #[test]
    fn recursive_pedal_is_rejected() {
        let a = pedal("name = \"A\"\n[[node]]\nid = \"x\"\ntype = \"pedal:b\"\n[[edge]]\nfrom = \"pedal_in\"\nto = \"x\"\n[[edge]]\nfrom = \"x\"\nto = \"pedal_out\"\n");
        let b = pedal("name = \"B\"\n[[node]]\nid = \"y\"\ntype = \"pedal:a\"\n[[edge]]\nfrom = \"pedal_in\"\nto = \"y\"\n[[edge]]\nfrom = \"y\"\nto = \"pedal_out\"\n");
        let g = USE_OUTER.replace("pedal:outer", "pedal:a").replace("params = { Master = 1 }\n", "");
        let e = build(&g, &[("a".into(), a), ("b".into(), b)]).err().unwrap();
        assert!(format!("{e:#}").contains("a -> b -> a"), "{e:#}");
    }

    #[test]
    fn cab_knob_maps_to_high_hz() {
        let pedals = read_pedals(Path::new("pedals")).unwrap();
        let g = crate::graph::load::read_graph(Path::new("docs/examples/crunch-graph.toml")).unwrap();
        let (_, map) = compile(&g, &pedals, SR).unwrap();
        let c = map.resolve("speaker", "Brightness", 0.5).unwrap().unwrap();
        let index = dsp::find("cab").unwrap().desc.param_index("high_hz").unwrap();
        assert_eq!(c.param, index);
        assert!((c.value - 5250.0).abs() < 0.01);
        assert!(map.resolve("speaker", "Missing", 0.5).is_err());
        assert!(map.resolve("nobody", "x", 0.5).is_err());
        // A normal block: parameter by id, value clamped to the range.
        let c = map.resolve("dist", "gain_db", 1000.0).unwrap().unwrap();
        assert!(c.value < 1000.0);
    }

    #[test]
    fn output_volume_changes_live_and_unconnected_nodes_are_ignored() {
        let g = r#"
name = "p"
[[node]]
id = "in"
type = "input"
[[node]]
id = "solo"
type = "gain"
[[node]]
id = "out"
type = "output"
[[edge]]
from = "in"
to = "out"
"#;
        let (mut plan, map) = build(g, &[]).unwrap();
        assert_eq!(map.resolve("solo", "db", 3.0).unwrap(), None);
        let c = map.resolve("out", "volume_db", -6.0).unwrap().unwrap();
        plan.set_param(c.step, c.param, c.value);
        let out = run(&mut plan, &vec![0.5; 48000]);
        assert!((out[47999] - 0.5 * crate::dsp::db_to_lin(-6.0)).abs() < 1e-4);
    }

    #[test]
    fn clear_errors() {
        let base = "name = \"p\"\n[[node]]\nid = \"in\"\ntype = \"input\"\n[[node]]\nid = \"out\"\ntype = \"output\"\n";
        let msg = |extra: &str| format!("{:#}", build(&format!("{base}{extra}"), &[]).err().expect(extra));
        assert!(msg("[[node]]\nid = \"in2\"\ntype = \"input\"\n").contains("2 'input' nodes"));
        assert!(msg("[[node]]\nid = \"x\"\ntype = \"tubo\"\n").contains("unknown type"));
        assert!(msg("[[edge]]\nfrom = \"in\"\nto = \"ghost\"\n").contains("ghost"));
        assert!(msg("[[node]]\nid = \"in\"\ntype = \"gain\"\n").contains("more than one node"));
        assert!(msg("[[node]]\nid = \"x\"\ntype = \"gain\"\nparams = { gain = 3 }\n").contains("db"));
        assert!(msg("[[node]]\nid = \"x\"\ntype = \"pedal:nonexistent\"\n").contains("nonexistent"));
        assert!(msg("[[edge]]\nfrom = \"out\"\nto = \"in\"\n").contains("cannot"));
        let senza_uscita = "name = \"p\"\n[[node]]\nid = \"in\"\ntype = \"input\"\n";
        assert!(build(senza_uscita, &[]).is_err());
    }

    #[test]
    fn no_path_means_no_sound() {
        let g = "name = \"p\"\n[[node]]\nid = \"in\"\ntype = \"input\"\n[[node]]\nid = \"out\"\ntype = \"output\"\n";
        let (mut plan, _) = build(g, &[]).unwrap();
        let out = run(&mut plan, &signal(480));
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn long_buffers_are_processed_in_pieces() {
        let g = "name = \"p\"\n[[block]]\ntype = \"gain\"\ndb = -6\n";
        let (mut plan, _) = build(g, &[]).unwrap();
        let input = signal(20000);
        let mut out = input.clone();
        plan.process(&mut out);
        let g6 = crate::dsp::db_to_lin(-6.0);
        assert!((out[19999] - input[19999] * g6).abs() < 1e-5);
    }

    #[test]
    fn every_bundled_preset_compiles_and_plays() {
        let pedals = crate::graph::load::read_pedals(Path::new("pedals")).unwrap();
        for entry in std::fs::read_dir("presets").unwrap() {
            let path = entry.unwrap().path();
            let graph = crate::graph::load::read_graph(&path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            let (mut plan, _) = compile(&graph, &pedals, SR).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            let mut buf = signal(48000);
            for block in buf.chunks_mut(64) {
                plan.process(block);
            }
            assert!(buf.iter().all(|s| s.is_finite()), "{}", path.display());
            assert!(buf.iter().any(|s| s.abs() > 1e-4), "{} is silent", path.display());
        }
    }

    // ---------- bypass ----------

    fn gain_graph(extra: &str) -> String {
        format!(
            r#"
name = "p"
[[node]]
id = "in"
type = "input"
[[node]]
id = "g"
type = "gain"
params = {{ db = -12 }}
{extra}
[[node]]
id = "out"
type = "output"
[[edge]]
from = "in"
to = "g"
[[edge]]
from = "g"
to = "out"
"#
        )
    }

    fn sine(n: usize) -> Vec<f32> {
        (0..n).map(|i| 0.4 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin()).collect()
    }

    #[test]
    fn bypassed_node_passes_its_input_through() {
        let input = signal(4800);
        let (mut normal, _) = build(&gain_graph(""), &[]).unwrap();
        let (mut bypassed, _) = build(&gain_graph("bypass = true"), &[]).unwrap();
        let processed = run(&mut normal, &input);
        assert!(peak(&processed) < 0.6 * peak(&input), "the gain must lower the level");
        let out = run(&mut bypassed, &input);
        assert!(out.iter().zip(&input).all(|(a, b)| a.to_bits() == b.to_bits()), "bypassed node must be transparent");
    }

    #[test]
    fn bypassed_pedal_passes_its_input_through() {
        let p = pedal("name = \"P\"\n[[node]]\nid = \"g\"\ntype = \"gain\"\nparams = { db = -12 }\n[[edge]]\nfrom = \"pedal_in\"\nto = \"g\"\n[[edge]]\nfrom = \"g\"\nto = \"pedal_out\"\n");
        let pedals = vec![("p".to_string(), p)];
        let graph = |extra: &str| gain_graph(extra).replace("type = \"gain\"\nparams = { db = -12 }", "type = \"pedal:p\"");
        let input = signal(4800);
        let (mut normal, _) = build(&graph(""), &pedals).unwrap();
        assert!(peak(&run(&mut normal, &input)) < 0.6 * peak(&input));
        let (mut bypassed, _) = build(&graph("bypass = true"), &pedals).unwrap();
        let out = run(&mut bypassed, &input);
        assert!(out.iter().zip(&input).all(|(a, b)| a.to_bits() == b.to_bits()));
        // live toggle
        let (mut plan, map) = build(&graph(""), &pedals).unwrap();
        plan.set_bypass(map.resolve_bypass("g").unwrap().unwrap(), true);
        run(&mut plan, &input);
        let out = run(&mut plan, &input);
        assert!(out.iter().zip(&input).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn live_bypass_crossfades_without_clicks() {
        let input = sine(48000);
        let (mut plan, map) = build(&gain_graph(""), &[]).unwrap();
        let step = map.resolve_bypass("g").unwrap().unwrap();
        let mut out = Vec::new();
        for (k, block) in input.chunks(64).enumerate() {
            // toggle at 0.2 s (on) and 0.6 s (off), in the middle of the signal
            if k * 64 == 9600 {
                plan.set_bypass(step, true);
            }
            if k * 64 == 28800 {
                plan.set_bypass(step, false);
            }
            let mut b = block.to_vec();
            plan.process(&mut b);
            out.extend(b);
        }
        let jump = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        // The sine alone moves at most ~0.0115 per sample; an abrupt switch would jump ~0.3.
        assert!(jump < 0.02, "max sample-to-sample jump {jump}");
        // Settled: processed before, dry in the middle, processed again at the end.
        assert!((peak(&out[4800..9600]) - 0.1).abs() < 0.01);
        assert!((peak(&out[20000..28000]) - 0.4).abs() < 0.01);
        assert!((peak(&out[40000..]) - 0.1).abs() < 0.01);
    }

    #[test]
    fn input_and_output_cannot_be_bypassed() {
        let (_, map) = build(&gain_graph(""), &[]).unwrap();
        for node in ["in", "out"] {
            let e = map.resolve_bypass(node).unwrap_err();
            assert!(format!("{e:#}").contains("cannot be bypassed"), "{e:#}");
        }
        assert!(map.resolve_bypass("nope").is_err());
        let g = gain_graph("").replace("id = \"out\"\ntype = \"output\"", "id = \"out\"\ntype = \"output\"\nbypass = true");
        let e = build(&g, &[]).err().expect("bypass on output must fail");
        assert_eq!(e.downcast_ref::<GraphError>().unwrap().node.as_deref(), Some("out"));
    }
}
