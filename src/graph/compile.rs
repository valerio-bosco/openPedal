//! Compilatore: espande i pedali composti, controlla il grafo, lo ordina e
//! costruisce il `Plan`. Gira nel thread di controllo (può allocare e leggere file).

// L'API di traduzione dei parametri la userà l'integrazione (I2, `EngineHandle`).
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use anyhow::{bail, Context, Result};

use super::exec::{Op, OutGain, Plan, Step};
use super::model::*;
use crate::dsp::{self, db_to_lin, BlockDesc, Entry, ParamKind, ParamValues, Smoothed, Value};

/// Errore del grafo, con il nodo (di primo livello) a cui si riferisce, se c'è:
/// l'interfaccia lo può evidenziare (`anyhow::Error::downcast_ref::<GraphError>()`).
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

/// Un cambio di parametro già tradotto in indici del `Plan`, pronto per il thread audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamChange {
    pub step: usize,
    pub param: usize,
    pub value: f32,
}

/// Dove va a finire una manopola esposta: il parametro di un blocco, attraverso
/// uno o più livelli di mappatura 0..1 -> min..max (più livelli se i pedali sono annidati).
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
    /// Nodo che corrisponde a un solo blocco (compresi `input` e `output`).
    Block { flat: usize, desc: &'static BlockDesc },
    Pedal { knobs: BTreeMap<String, Binding> },
}

/// Traduce (id del nodo, id del parametro) negli indici per `Plan::set_param`.
/// La tiene il thread di controllo; è valida solo per il `Plan` con cui è stata creata.
pub struct NodeMap {
    slots: HashMap<String, Slot>,
    /// Nodo piatto -> passo del piano (None = scartato perché non porta all'uscita).
    flat_to_step: Vec<Option<usize>>,
}

impl NodeMap {
    /// `Ok(None)`: il nodo esiste ma non è nel piano (non è collegato tra chitarra e uscita),
    /// quindi non c'è niente da cambiare. Errore: nodo o parametro inesistente, o parametro
    /// che richiede di ricostruire il grafo (un file).
    /// Per i pedali composti `param` è l'etichetta della manopola e `value` va da 0 a 1.
    pub fn resolve(&self, node: &str, param: &str, value: f32) -> Result<Option<ParamChange>> {
        let Some(slot) = self.slots.get(node) else {
            return Err(err(Some(node), format!("il nodo '{node}' non esiste")));
        };
        let (flat, index, value) = match slot {
            Slot::Block { flat, desc } => {
                let Some(index) = desc.param_index(param) else {
                    let valid: Vec<_> = desc.params.iter().map(|p| p.id).collect();
                    let msg = format!("il nodo '{node}' ({}) non ha il parametro '{param}' (validi: {})", desc.kind, valid.join(", "));
                    return Err(err(Some(node), msg));
                };
                let pd = &desc.params[index];
                if let ParamKind::File { .. } = pd.kind {
                    return Err(err(Some(node), format!("'{node}.{param}' si cambia ricostruendo il grafo")));
                }
                (*flat, index, pd.clamp(value))
            }
            Slot::Pedal { knobs } => {
                let Some(b) = knobs.get(param) else {
                    let valid: Vec<_> = knobs.keys().map(String::as_str).collect();
                    let msg = format!("il pedale '{node}' non ha la manopola '{param}' (manopole: {})", valid.join(", "));
                    return Err(err(Some(node), msg));
                };
                (b.flat, b.param, b.desc.params[b.param].clamp(b.apply(value)))
            }
        };
        Ok(self.flat_to_step[flat].map(|step| ParamChange { step, param: index, value }))
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
    /// Percorso leggibile: "speaker/c" per un nodo dentro un pedale.
    path: String,
    kind: FlatKind,
    params: ParamValues,
}

/// Come ci si collega a un nodo di un livello: i cavi entrano in `entry` e partono da `exit`.
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
        self.nodes.push(FlatNode { path, kind, params });
        self.nodes.len() - 1
    }

    /// Espande un livello (il grafo principale o il corpo di un pedale).
    /// `implicit` = giunzioni `pedal_in`/`pedal_out`, solo dentro i pedali.
    /// `top` = nodo di primo livello da segnalare negli errori.
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
                return Err(err(node, "un nodo ha l'id vuoto"));
            }
            if map.contains_key(&spec.id) {
                return Err(err(node, format!("l'id '{}' è usato da più nodi (o è riservato)", spec.id)));
            }
            kinds.insert(&spec.id, &spec.kind);
            let path = if prefix.is_empty() { spec.id.clone() } else { format!("{prefix}/{}", spec.id) };
            let exp = match spec.kind.as_str() {
                INPUT | OUTPUT if top.is_some() || implicit.is_some() => {
                    return Err(err(node, format!("'{}' non può stare dentro un pedale composto", spec.kind)));
                }
                INPUT => self.leaf(path, FlatKind::Input, &INPUT_DESC, spec)?,
                OUTPUT => self.leaf(path, FlatKind::Output, &OUTPUT_DESC, spec)?,
                kind if kind.starts_with(PEDAL_PREFIX) => {
                    self.pedal(&kind[PEDAL_PREFIX.len()..], spec, &path, node.unwrap_or(&spec.id), stack)?
                }
                kind => {
                    let Some(entry) = dsp::find(kind) else {
                        let valid: Vec<_> = dsp::catalog().iter().map(|e| e.desc.kind).collect();
                        let msg = format!("il nodo '{}' ha un tipo sconosciuto '{kind}' (tipi validi: {})", spec.id, valid.join(", "));
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
            let missing = |id: &str| err(node, format!("il cavo {} -> {} usa il nodo '{id}', che non esiste", e.from, e.to));
            let from = map.get(&e.from).ok_or_else(|| missing(&e.from))?;
            let to = map.get(&e.to).ok_or_else(|| missing(&e.to))?;
            let from_is_sink = e.from == PEDAL_OUT || kinds.get(e.from.as_str()) == Some(&OUTPUT);
            let to_is_source = e.to == PEDAL_IN || kinds.get(e.to.as_str()) == Some(&INPUT);
            if from_is_sink {
                return Err(err(node, format!("dall'uscita '{}' non possono partire cavi", e.from)));
            }
            if to_is_source {
                return Err(err(node, format!("nell'ingresso '{}' non possono arrivare cavi", e.to)));
            }
            if !seen.insert((e.from.as_str(), e.to.as_str())) {
                return Err(err(node, format!("il cavo {} -> {} è ripetuto", e.from, e.to)));
            }
            self.edges.push((from.exit, to.entry));
        }
        Ok(map)
    }

    fn leaf(&mut self, path: String, kind: FlatKind, desc: &'static BlockDesc, spec: &NodeSpec) -> Result<Expanded> {
        spec.params.validate(desc).map_err(|e| err(Some(&spec.id), format!("{e:#}")))?;
        let i = self.push(path, kind, spec.params.clone());
        Ok(Expanded { entry: i, exit: i, knobs: BTreeMap::new() })
    }

    /// Espande un nodo `pedal:<id>`: le manopole impostano i parametri interni.
    fn pedal(&mut self, id: &str, spec: &NodeSpec, path: &str, top: &str, stack: &mut Vec<String>) -> Result<Expanded> {
        if stack.iter().any(|s| s == id) {
            let chain: Vec<_> = stack.iter().map(String::as_str).chain([id]).collect();
            return Err(err(Some(top), format!("il pedale '{id}' contiene se stesso: {}", chain.join(" -> "))));
        }
        let pedals = self.pedals;
        let Some((_, pedal)) = pedals.iter().find(|(pid, _)| pid == id) else {
            return Err(err(Some(top), format!("il pedale '{id}' non esiste (manca pedals/{id}.toml)")));
        };

        for name in spec.params.0.keys() {
            if !pedal.knobs.iter().any(|k| &k.label == name) {
                let valid: Vec<_> = pedal.knobs.iter().map(|k| k.label.as_str()).collect();
                let msg = format!("il pedale '{id}' non ha la manopola '{name}' (manopole: {})", valid.join(", "));
                return Err(err(Some(top), msg));
            }
        }

        // Le manopole scrivono il proprio valore nel parametro interno prima di costruire.
        let mut inner = pedal.nodes.clone();
        for k in &pedal.knobs {
            let (target, param) = k.target.split_once('.').ok_or_else(|| {
                err(Some(top), format!("pedale '{id}', manopola '{}': il target '{}' deve essere 'nodo.parametro'", k.label, k.target))
            })?;
            let Some(node) = inner.iter_mut().find(|n| n.id == target) else {
                return Err(err(Some(top), format!("pedale '{id}', manopola '{}': il nodo '{target}' non esiste", k.label)));
            };
            let v = match spec.params.get(&k.label) {
                Some(Value::Number(v)) => *v,
                Some(Value::Text(_)) => return Err(err(Some(top), format!("la manopola '{}' deve essere un numero", k.label))),
                None => k.default,
            };
            node.params.set(param, Value::Number(k.min + (k.max - k.min) * v.clamp(0.0, 1.0)));
        }

        let pin = self.push(format!("{path}/{PEDAL_IN}"), FlatKind::Pass, ParamValues::default());
        let pout = self.push(format!("{path}/{PEDAL_OUT}"), FlatKind::Pass, ParamValues::default());
        stack.push(id.to_string());
        let map = self.body(&inner, &pedal.edges, path, Some(top), stack, Some((pin, pout)));
        stack.pop();
        let map = map.with_context(|| format!("pedale '{id}'"))?;

        let mut knobs = BTreeMap::new();
        for k in &pedal.knobs {
            let (target, param) = k.target.split_once('.').expect("già controllato");
            let exp = &map[target];
            let level = (k.min, k.max);
            let binding = match &self.nodes[exp.entry].kind {
                FlatKind::Block(entry) => {
                    let Some(index) = entry.desc.param_index(param) else {
                        let msg = format!("pedale '{id}', manopola '{}': '{target}' ({}) non ha il parametro '{param}'", k.label, entry.desc.kind);
                        return Err(err(Some(top), msg));
                    };
                    if !matches!(entry.desc.params[index].kind, ParamKind::Number { .. }) {
                        let msg = format!("pedale '{id}', manopola '{}': '{}' non è numerico", k.label, k.target);
                        return Err(err(Some(top), msg));
                    }
                    Binding { flat: exp.entry, desc: entry.desc, param: index, levels: vec![level] }
                }
                // Target = un altro pedale: `param` è una sua manopola, già verificata in `body`.
                _ => {
                    let Some(inner_binding) = exp.knobs.get(param) else {
                        return Err(err(Some(top), format!("pedale '{id}', manopola '{}': '{target}' non ha la manopola '{param}'", k.label)));
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

/// Compila un grafo (con i pedali composti disponibili) in un `Plan` e nella `NodeMap`
/// per tradurre i cambi di parametro. Può leggere file (cab) e fallire: non va chiamata
/// dal thread audio.
pub fn compile(graph: &GraphFile, pedals: &[(String, PedalFile)], sr: f32) -> Result<(Plan, NodeMap)> {
    let mut ex = Expander { pedals, nodes: Vec::new(), edges: Vec::new() };
    let top = ex.body(&graph.nodes, &graph.edges, "", None, &mut Vec::new(), None)?;
    let Expander { nodes, edges, .. } = ex;

    let inputs: Vec<_> = (0..nodes.len()).filter(|&i| matches!(nodes[i].kind, FlatKind::Input)).collect();
    let outputs: Vec<_> = (0..nodes.len()).filter(|&i| matches!(nodes[i].kind, FlatKind::Output)).collect();
    match (inputs.len(), outputs.len()) {
        (1, 1) => {}
        (0, _) => bail!("manca il nodo 'input' (la chitarra)"),
        (_, 0) => bail!("manca il nodo 'output' (l'ampli)"),
        (n, 1) if n > 1 => bail!("il grafo ha {n} nodi 'input': ne serve uno solo"),
        (_, n) => bail!("il grafo ha {n} nodi 'output': ne serve uno solo"),
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

    // Ordinamento topologico (Kahn); ciò che resta fuori fa parte di un ciclo o ne dipende.
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

    // Si tiene ciò che è raggiungibile dalla chitarra e porta all'uscita (l'uscita resta sempre).
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
                    .map_err(|e| err(Some(&top_of(&node.path)), format!("nodo '{}': {e:#}", node.path)))?;
                Op::Block(block)
            }
        };
        let inputs = pred[i].iter().filter(|&&p| keep(p)).map(|&p| flat_to_step[p].expect("ordine topologico")).collect();
        flat_to_step[i] = Some(steps.len());
        steps.push(Step { op, inputs });
    }
    debug_assert!(matches!(steps.last().map(|s| &s.op), Some(Op::Output(_))));

    let mut slots = HashMap::new();
    for spec in &graph.nodes {
        let exp = &top[&spec.id];
        let slot = match &nodes[exp.entry].kind {
            FlatKind::Input => Slot::Block { flat: exp.entry, desc: &INPUT_DESC },
            FlatKind::Output => Slot::Block { flat: exp.entry, desc: &OUTPUT_DESC },
            FlatKind::Block(entry) => Slot::Block { flat: exp.entry, desc: entry.desc },
            FlatKind::Pass => Slot::Pedal { knobs: exp.knobs.clone() },
        };
        slots.insert(spec.id.clone(), slot);
    }
    Ok((Plan::new(steps), NodeMap { slots, flat_to_step }))
}

fn top_of(path: &str) -> String {
    path.split('/').next().unwrap_or(path).to_string()
}

/// Nodi raggiungibili da `start` seguendo `next`.
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

/// Trova un ciclo tra i nodi che l'ordinamento non ha potuto sistemare, per descriverlo.
fn cycle_error(nodes: &[FlatNode], pred: &[Vec<usize>], left: &[usize]) -> anyhow::Error {
    // Ogni nodo rimasto ha un predecessore rimasto: risalendo si finisce in un ciclo.
    let start = (0..nodes.len()).find(|&i| left[i] > 0).expect("c'è un nodo rimasto");
    let mut path = vec![start];
    let cycle = loop {
        let cur = *path.last().unwrap();
        let prev = *pred[cur].iter().find(|&&p| left[p] > 0).expect("predecessore rimasto");
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
    err(Some(&top_of(&names[0])), format!("il grafo contiene un ciclo: {}", names.join(" -> ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{Chain, Preset};
    use crate::graph::load::{parse_graph, read_pedals};
    use std::path::Path;

    const SR: f32 = 48000.0;

    /// Segnale deterministico: un seno più un po' di rumore.
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
    fn i_preset_v1_suonano_come_la_vecchia_catena() {
        let input = signal(48000);
        let mut count = 0;
        for entry in std::fs::read_dir("presets").unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let preset: Preset = toml::from_str(&text).unwrap();
            let mut chain = Chain::build(&preset, SR).unwrap();
            let mut old = input.clone();
            for block in old.chunks_mut(64) {
                chain.process(block);
            }
            let (mut plan, _) = compile(&crate::graph::load::read_graph(&path).unwrap(), &[], SR).unwrap();
            let new = run(&mut plan, &input);
            let same = old.iter().zip(&new).all(|(a, b)| a.to_bits() == b.to_bits());
            assert!(same, "{} suona diverso", path.display());
            count += 1;
        }
        assert!(count >= 5);
    }

    #[test]
    fn il_grafo_di_esempio_suona() {
        let pedals = read_pedals(Path::new("pedals")).unwrap();
        let g = crate::graph::load::read_graph(Path::new("docs/esempi/crunch-grafo.toml")).unwrap();
        let (mut plan, _) = compile(&g, &pedals, SR).unwrap();
        let out = run(&mut plan, &signal(24000));
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(peak(&out) > 0.01);
    }

    #[test]
    fn fan_in_somma_e_fan_out_copia() {
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
    fn un_ciclo_e_rifiutato() {
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
        assert!(msg.contains("ciclo") && (msg.contains("a -> b -> a") || msg.contains("b -> a -> b")), "{msg}");
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
    fn un_pedale_annidato_viene_espanso_e_le_knob_funzionano_dal_vivo() {
        let (mut plan, map) = build(USE_OUTER, &nested_pedals()).unwrap();
        let input = signal(9600);
        // Master = 1 -> Level interno 0 -> -20 dB.
        let out = run(&mut plan, &input);
        let expected = crate::dsp::db_to_lin(-20.0);
        assert!((out[5000] - input[5000] * expected).abs() < 1e-5);

        // Dal vivo: Master = 0 -> Level interno 1 -> 0 dB.
        let change = map.resolve("p", "Master", 0.0).unwrap().unwrap();
        assert_eq!(change.value, 0.0);
        plan.set_param(change.step, change.param, change.value);
        let input = signal(48000);
        let out = run(&mut plan, &input);
        assert!((out[47000] - input[47000]).abs() < 1e-3);
    }

    #[test]
    fn la_manopola_senza_valore_usa_il_default() {
        let g = USE_OUTER.replace("params = { Master = 1 }\n", "");
        let (mut plan, _) = build(&g, &nested_pedals()).unwrap();
        let input = signal(9600);
        let out = run(&mut plan, &input);
        // Master = 0 (default dell'esterno) -> Level 1 -> 0 dB.
        assert!((out[5000] - input[5000]).abs() < 1e-5);
    }

    #[test]
    fn un_pedale_ricorsivo_e_rifiutato() {
        let a = pedal("name = \"A\"\n[[node]]\nid = \"x\"\ntype = \"pedal:b\"\n[[edge]]\nfrom = \"pedal_in\"\nto = \"x\"\n[[edge]]\nfrom = \"x\"\nto = \"pedal_out\"\n");
        let b = pedal("name = \"B\"\n[[node]]\nid = \"y\"\ntype = \"pedal:a\"\n[[edge]]\nfrom = \"pedal_in\"\nto = \"y\"\n[[edge]]\nfrom = \"y\"\nto = \"pedal_out\"\n");
        let g = USE_OUTER.replace("pedal:outer", "pedal:a").replace("params = { Master = 1 }\n", "");
        let e = build(&g, &[("a".into(), a), ("b".into(), b)]).err().unwrap();
        assert!(format!("{e:#}").contains("a -> b -> a"), "{e:#}");
    }

    #[test]
    fn la_knob_della_cassa_mappa_su_high_hz() {
        let pedals = read_pedals(Path::new("pedals")).unwrap();
        let g = crate::graph::load::read_graph(Path::new("docs/esempi/crunch-grafo.toml")).unwrap();
        let (_, map) = compile(&g, &pedals, SR).unwrap();
        let c = map.resolve("speaker", "Brillantezza", 0.5).unwrap().unwrap();
        let index = dsp::find("cab").unwrap().desc.param_index("high_hz").unwrap();
        assert_eq!(c.param, index);
        assert!((c.value - 5250.0).abs() < 0.01);
        assert!(map.resolve("speaker", "Inesistente", 0.5).is_err());
        assert!(map.resolve("nessuno", "x", 0.5).is_err());
        // Un blocco normale: parametro per id, valore limitato all'intervallo.
        let c = map.resolve("dist", "gain_db", 1000.0).unwrap().unwrap();
        assert!(c.value < 1000.0);
    }

    #[test]
    fn il_volume_dell_uscita_cambia_dal_vivo_e_i_nodi_scollegati_si_ignorano() {
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
    fn errori_chiari() {
        let base = "name = \"p\"\n[[node]]\nid = \"in\"\ntype = \"input\"\n[[node]]\nid = \"out\"\ntype = \"output\"\n";
        let msg = |extra: &str| format!("{:#}", build(&format!("{base}{extra}"), &[]).err().expect(extra));
        assert!(msg("[[node]]\nid = \"in2\"\ntype = \"input\"\n").contains("2 nodi 'input'"));
        assert!(msg("[[node]]\nid = \"x\"\ntype = \"tubo\"\n").contains("tipo sconosciuto"));
        assert!(msg("[[edge]]\nfrom = \"in\"\nto = \"fantasma\"\n").contains("fantasma"));
        assert!(msg("[[node]]\nid = \"in\"\ntype = \"gain\"\n").contains("più nodi"));
        assert!(msg("[[node]]\nid = \"x\"\ntype = \"gain\"\nparams = { gain = 3 }\n").contains("db"));
        assert!(msg("[[node]]\nid = \"x\"\ntype = \"pedal:nonesiste\"\n").contains("nonesiste"));
        assert!(msg("[[edge]]\nfrom = \"out\"\nto = \"in\"\n").contains("non possono"));
        let senza_uscita = "name = \"p\"\n[[node]]\nid = \"in\"\ntype = \"input\"\n";
        assert!(build(senza_uscita, &[]).is_err());
    }

    #[test]
    fn senza_percorso_non_esce_suono() {
        let g = "name = \"p\"\n[[node]]\nid = \"in\"\ntype = \"input\"\n[[node]]\nid = \"out\"\ntype = \"output\"\n";
        let (mut plan, _) = build(g, &[]).unwrap();
        let out = run(&mut plan, &signal(480));
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn buffer_lunghi_si_elaborano_a_pezzi() {
        let g = "name = \"p\"\n[[block]]\ntype = \"gain\"\ndb = -6\n";
        let (mut plan, _) = build(g, &[]).unwrap();
        let input = signal(20000);
        let mut out = input.clone();
        plan.process(&mut out);
        let g6 = crate::dsp::db_to_lin(-6.0);
        assert!((out[19999] - input[19999] * g6).abs() < 1e-5);
    }
}
