//! Lettura di preset e pedali dal disco.
//! I preset v1 (`[[block]]`) vengono convertiti in grafi lineari.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use super::model::{EdgeSpec, GraphFile, NodeSpec, PedalFile, INPUT, OUTPUT};
use crate::dsp::{ParamValues, Preset, Value};

/// Legge un preset (v2 o v1: i v1 vengono convertiti in un grafo lineare).
pub fn read_graph(path: &Path) -> Result<GraphFile> {
    let text = std::fs::read_to_string(path).with_context(|| format!("impossibile leggere {}", path.display()))?;
    parse_graph(&text).with_context(|| format!("errore nel preset {}", path.display()))
}

/// Interpreta il testo di un preset: v2 (`[[node]]`) oppure v1 (`[[block]]`).
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

/// Converte un preset v1 in un grafo `input -> blocchi -> output`.
/// `output_db` diventa il volume dell'uscita; i nodi stanno a 220 px l'uno dall'altro.
pub fn graph_from_preset(preset: &Preset) -> GraphFile {
    let mut nodes = vec![NodeSpec { id: "in".into(), kind: INPUT.into(), pos: [0.0, 0.0], params: ParamValues::default() }];
    let mut counters: HashMap<&str, usize> = HashMap::new();
    for (i, b) in preset.blocks.iter().enumerate() {
        let n = counters.entry(b.kind.as_str()).or_insert(0);
        *n += 1;
        // Il tipo può essere sbagliato (lo segnalerà il compilatore): l'id deve comunque restare valido.
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

/// Legge tutti i pedali composti di una cartella: (id = nome del file senza estensione, pedale).
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
        let pedal: PedalFile = toml::from_str(&text).with_context(|| format!("errore nel pedale {}", path.display()))?;
        pedals.push((id, pedal));
    }
    pedals.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pedals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_diventa_un_grafo_lineare() {
        let g = parse_graph("name = \"X\"\noutput_db = -6\n[[block]]\ntype = \"gate\"\nthreshold_db = -50\n[[block]]\ntype = \"filter\"\n[[block]]\ntype = \"filter\"\n")
            .unwrap();
        let ids: Vec<_> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["in", "gate1", "filter1", "filter2", "out"]);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.nodes[1].pos, [220.0, 0.0]);
        assert_eq!(g.nodes[4].params.get("volume_db"), Some(&Value::Number(-6.0)));
    }

    #[test]
    fn v2_resta_v2() {
        let g = parse_graph(&std::fs::read_to_string("docs/esempi/crunch-grafo.toml").unwrap()).unwrap();
        assert_eq!(g.nodes.len(), 6);
    }

    #[test]
    fn v1_con_campo_sbagliato_e_un_errore() {
        assert!(parse_graph("nome = 3\n").is_err());
    }
}
