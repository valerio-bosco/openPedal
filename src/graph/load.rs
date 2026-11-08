//! Reading presets and pedals from disk.
//! v1 presets (`[[block]]`) are converted into linear graphs.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use super::model::{EdgeSpec, GraphFile, NodeSpec, PedalFile, INPUT, OUTPUT};
use crate::dsp::{ParamValues, Preset, Value};

/// Reads a preset (v2 or v1: v1 presets are converted into a linear graph).
pub fn read_graph(path: &Path) -> Result<GraphFile> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    parse_graph(&text).with_context(|| format!("error in preset {}", path.display()))
}

/// Parses the text of a preset: v2 (`[[node]]`) or v1 (`[[block]]`).
pub fn parse_graph(text: &str) -> Result<GraphFile> {
    let mut table: toml::Table = text.parse()?;
    let version = table.get("version").and_then(|v| v.as_integer());
    let is_v2 = table.contains_key("node") || table.contains_key("edge") || version == Some(2);
    if is_v2 {
        return Ok(toml::from_str(text)?);
    }
    if version == Some(1) {
        table.remove("version");
    }
    let preset: Preset = table.try_into()?;
    Ok(graph_from_preset(&preset))
}

/// Converts a v1 preset into an `input -> blocks -> output` graph.
/// `output_db` becomes the output volume; nodes are placed 220 px apart.
pub fn graph_from_preset(preset: &Preset) -> GraphFile {
    let mut nodes = vec![NodeSpec { id: "in".into(), kind: INPUT.into(), pos: [0.0, 0.0], params: ParamValues::default() }];
    let mut counters: HashMap<&str, usize> = HashMap::new();
    for (i, b) in preset.blocks.iter().enumerate() {
        let n = counters.entry(b.kind.as_str()).or_insert(0);
        *n += 1;
        // The type may be wrong (the compiler will report it): the id must still be valid.
        let base: String = b.kind.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        nodes.push(NodeSpec {
            id: format!("{base}{n}"),
            kind: b.kind.clone(),
            pos: [220.0 * (i + 1) as f32, 0.0],
            params: b.params.clone(),
        });
    }
    let mut out_params = ParamValues::default();
    if preset.output_db != 0.0 {
        out_params.set("volume_db", Value::Number(preset.output_db));
    }
    nodes.push(NodeSpec {
        id: "out".into(),
        kind: OUTPUT.into(),
        pos: [220.0 * (preset.blocks.len() + 1) as f32, 0.0],
        params: out_params,
    });
    let edges = nodes.windows(2).map(|w| EdgeSpec { from: w[0].id.clone(), to: w[1].id.clone() }).collect();
    GraphFile { name: preset.name.clone(), version: 2, nodes, edges }
}

/// Reads all composite pedals in a folder: (id = file name without extension, pedal).
pub fn read_pedals(dir: &Path) -> Result<Vec<(String, PedalFile)>> {
    let mut pedals = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(pedals) };
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        let text = std::fs::read_to_string(&path)?;
        let pedal: PedalFile = toml::from_str(&text).with_context(|| format!("error in pedal {}", path.display()))?;
        pedals.push((id, pedal));
    }
    pedals.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pedals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_becomes_a_linear_graph() {
        let g = parse_graph("name = \"X\"\noutput_db = -6\n[[block]]\ntype = \"gate\"\nthreshold_db = -50\n[[block]]\ntype = \"filter\"\n[[block]]\ntype = \"filter\"\n")
            .unwrap();
        let ids: Vec<_> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["in", "gate1", "filter1", "filter2", "out"]);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.nodes[1].pos, [220.0, 0.0]);
        assert_eq!(g.nodes[4].params.get("volume_db"), Some(&Value::Number(-6.0)));
    }

    #[test]
    fn v2_stays_v2() {
        let g = parse_graph(&std::fs::read_to_string("docs/examples/crunch-graph.toml").unwrap()).unwrap();
        assert_eq!(g.nodes.len(), 6);
    }

    #[test]
    fn v1_with_wrong_field_is_an_error() {
        assert!(parse_graph("name = 3\n").is_err());
    }
}
