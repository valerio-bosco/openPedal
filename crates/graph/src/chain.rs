//! Edits of a linear chain `input -> pedal -> ... -> output` (docs/PLAN.md §10.1).
//! Pure functions on `GraphFile`; the server owns history and broadcasting.

use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};

use crate::model::{EdgeSpec, GraphFile, NodeSpec, INPUT, OUTPUT, PEDAL_PREFIX};

/// Horizontal distance between nodes and row used when a chain is laid out.
const SPACING: f32 = 220.0;
const ROW: f32 = 120.0;

const NOT_LINEAR: &str = "this graph has parallel routing: edit it on the desktop";

/// The nodes in signal order when the graph is a simple line from the only `input` to
/// the `output` covering every node; `None` otherwise (branches, cycles, loose nodes).
pub fn linear_order(g: &GraphFile) -> Option<Vec<NodeSpec>> {
    let by_id: HashMap<&str, &NodeSpec> = g.nodes.iter().rev().map(|n| (n.id.as_str(), n)).collect();
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut indeg: HashMap<&str, usize> = HashMap::new();
    for e in &g.edges {
        if by_id.contains_key(e.from.as_str()) && by_id.contains_key(e.to.as_str()) {
            next.entry(&e.from).or_default().push(&e.to);
            *indeg.entry(&e.to).or_default() += 1;
        }
    }
    let mut inputs = g.nodes.iter().filter(|n| n.kind == INPUT);
    let input = inputs.next()?;
    if inputs.next().is_some() || next.values().any(|v| v.len() > 1) || indeg.values().any(|d| *d > 1) {
        return None;
    }
    let mut order = Vec::with_capacity(g.nodes.len());
    let mut seen = HashSet::new();
    let mut cur = Some(input.id.as_str());
    while let Some(id) = cur {
        if !seen.insert(id) {
            break;
        }
        let Some(n) = by_id.get(id) else { break };
        order.push((*n).clone());
        cur = next.get(id).and_then(|v| v.first().copied());
    }
    (order.len() == g.nodes.len() && order.last().is_some_and(|n| n.kind == OUTPUT)).then_some(order)
}

/// Rebuilds node positions and edges for a linear order.
pub fn rewrite(g: &GraphFile, mut order: Vec<NodeSpec>) -> GraphFile {
    for (i, n) in order.iter_mut().enumerate() {
        n.pos = [i as f32 * SPACING, ROW];
    }
    let edges = order.windows(2).map(|w| EdgeSpec { from: w[0].id.clone(), to: w[1].id.clone() }).collect();
    GraphFile { name: g.name.clone(), version: g.version, nodes: order, edges }
}

fn order_of(g: &GraphFile) -> Result<Vec<NodeSpec>> {
    linear_order(g).ok_or_else(|| anyhow!(NOT_LINEAR))
}

/// A node id not used in `g`, derived from the type (`pedal:cab-4x12` -> `cab-4x12`, then `cab-4x122`...).
pub fn unique_id(g: &GraphFile, kind: &str) -> String {
    let base = kind.strip_prefix(PEDAL_PREFIX).unwrap_or(kind);
    let mut id = base.to_string();
    let mut n = 2;
    while g.nodes.iter().any(|x| x.id == id) {
        id = format!("{base}{n}");
        n += 1;
    }
    id
}

/// Inserts a new node of type `kind` after `after` (default: before the output).
/// Returns the new graph and the id of the node.
pub fn add_node(g: &GraphFile, kind: &str, after: Option<&str>) -> Result<(GraphFile, String)> {
    if kind == INPUT || kind == OUTPUT {
        bail!("cannot add another '{kind}' node");
    }
    let mut order = order_of(g)?;
    let at = match after {
        None => order.len() - 1,
        Some(id) => {
            let i = order.iter().position(|n| n.id == id).ok_or_else(|| anyhow!("node '{id}' does not exist"))?;
            if order[i].kind == OUTPUT {
                bail!("cannot add a pedal after the output");
            }
            i + 1
        }
    };
    let id = unique_id(g, kind);
    order.insert(at, NodeSpec { id: id.clone(), kind: kind.to_string(), pos: [0.0, ROW], params: Default::default(), bypass: false });
    Ok((rewrite(g, order), id))
}

fn pedal_index(order: &[NodeSpec], id: &str) -> Result<usize> {
    let i = order.iter().position(|n| n.id == id).ok_or_else(|| anyhow!("node '{id}' does not exist"))?;
    if order[i].kind == INPUT || order[i].kind == OUTPUT {
        bail!("the {} node cannot be moved or removed", order[i].kind);
    }
    Ok(i)
}

pub fn remove_node(g: &GraphFile, id: &str) -> Result<GraphFile> {
    let mut order = order_of(g)?;
    let i = pedal_index(&order, id)?;
    order.remove(i);
    Ok(rewrite(g, order))
}

/// Moves a pedal to position `to` among the pedals (0 = right after the input, clamped).
pub fn move_node(g: &GraphFile, id: &str, to: usize) -> Result<GraphFile> {
    let mut order = order_of(g)?;
    let i = pedal_index(&order, id)?;
    let j = to.min(order.len() - 3) + 1;
    let node = order.remove(i);
    order.insert(j, node);
    Ok(rewrite(g, order))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str) -> NodeSpec {
        NodeSpec { id: id.into(), kind: kind.into(), pos: [0.0, 0.0], params: Default::default(), bypass: false }
    }

    fn chain(pedals: &[(&str, &str)]) -> GraphFile {
        let g = GraphFile { name: "t".into(), version: 2, nodes: vec![], edges: vec![] };
        let mut order = vec![node("in", INPUT)];
        order.extend(pedals.iter().map(|(i, k)| node(i, k)));
        order.push(node("out", OUTPUT));
        rewrite(&g, order)
    }

    fn ids(g: &GraphFile) -> Vec<&str> {
        g.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    fn branched() -> GraphFile {
        let mut g = chain(&[("a", "drive")]);
        g.edges.push(EdgeSpec { from: "in".into(), to: "out".into() });
        g
    }

    #[test]
    fn rewrite_lays_out_and_connects() {
        let g = chain(&[("a", "drive"), ("b", "delay")]);
        assert_eq!(g.nodes[2].pos, [440.0, 120.0]);
        assert_eq!(g.edges.len(), 3);
        assert_eq!((g.edges[1].from.as_str(), g.edges[1].to.as_str()), ("a", "b"));
        let order: Vec<_> = linear_order(&g).unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(order, ["in", "a", "b", "out"]);
    }

    #[test]
    fn linear_detection() {
        assert!(linear_order(&chain(&[])).is_some());
        assert!(linear_order(&chain(&[("a", "drive")])).is_some());
        assert!(linear_order(&branched()).is_none());
        let mut loose = chain(&[("a", "drive")]);
        loose.nodes.push(node("x", "delay"));
        assert!(linear_order(&loose).is_none());
        let mut g = chain(&[("a", "drive")]);
        g.edges.pop();
        assert!(linear_order(&g).is_none(), "does not reach the output");
    }

    #[test]
    fn add_defaults_to_before_the_output_and_picks_unique_ids() {
        let g = chain(&[("drive", "drive")]);
        let (g, id) = add_node(&g, "drive", None).unwrap();
        assert_eq!(id, "drive2");
        assert_eq!(ids(&g), ["in", "drive", "drive2", "out"]);
        let (g, id) = add_node(&g, "pedal:cab-4x12", Some("in")).unwrap();
        assert_eq!(id, "cab-4x12");
        assert_eq!(ids(&g), ["in", "cab-4x12", "drive", "drive2", "out"]);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.nodes[1].pos, [220.0, 120.0]);
    }

    #[test]
    fn add_errors() {
        let g = chain(&[("a", "drive")]);
        assert!(add_node(&g, "delay", Some("nope")).is_err());
        assert!(add_node(&g, "delay", Some("out")).is_err());
        assert!(add_node(&g, "input", None).is_err());
        assert!(add_node(&g, "output", None).is_err());
        let e = add_node(&branched(), "delay", None).unwrap_err();
        assert!(e.to_string().contains("parallel"));
    }

    #[test]
    fn remove_reconnects_and_protects_io() {
        let g = chain(&[("a", "drive"), ("b", "delay")]);
        let g = remove_node(&g, "a").unwrap();
        assert_eq!(ids(&g), ["in", "b", "out"]);
        assert_eq!(g.edges.len(), 2);
        assert!(remove_node(&g, "in").is_err());
        assert!(remove_node(&g, "out").is_err());
        assert!(remove_node(&g, "zzz").is_err());
        assert!(remove_node(&branched(), "a").is_err());
    }

    #[test]
    fn move_among_pedals_clamps_and_keeps_io() {
        let g = chain(&[("a", "drive"), ("b", "delay"), ("c", "chorus")]);
        assert_eq!(ids(&move_node(&g, "c", 0).unwrap()), ["in", "c", "a", "b", "out"]);
        assert_eq!(ids(&move_node(&g, "a", 1).unwrap()), ["in", "b", "a", "c", "out"]);
        assert_eq!(ids(&move_node(&g, "a", 99).unwrap()), ["in", "b", "c", "a", "out"]);
        assert_eq!(ids(&move_node(&g, "b", 1).unwrap()), ["in", "a", "b", "c", "out"]);
        assert!(move_node(&g, "in", 1).is_err());
        assert!(move_node(&g, "out", 0).is_err());
        assert!(move_node(&branched(), "a", 0).is_err());
    }

    #[test]
    fn params_and_bypass_survive_edits() {
        let mut g = chain(&[("a", "drive"), ("b", "delay")]);
        g.nodes[1].bypass = true;
        g.nodes[1].params.set("gain_db", openpedal_dsp::params::Value::Number(7.0));
        let g = move_node(&g, "a", 1).unwrap();
        let a = g.nodes.iter().find(|n| n.id == "a").unwrap();
        assert!(a.bypass && a.params.get("gain_db").is_some());
    }
}
