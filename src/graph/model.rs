//! Graph types (v2 presets) and composite pedals: the format of the TOML files
//! and of the JSON messages to the interface. Contract: see docs/PLAN.md §5.3.

use serde::{Deserialize, Serialize};

use crate::dsp::params::{BlockDesc, Category, ParamDesc, ParamKind, ParamValues};

/// Type of the input node (the guitar): one per graph, with no inputs.
pub const INPUT: &str = "input";
/// Type of the output node ("Ampli" in the UI): one per graph; the sound is what arrives here.
pub const OUTPUT: &str = "output";
/// Type prefix of a node that uses a composite pedal: "pedal:<id>".
pub const PEDAL_PREFIX: &str = "pedal:";
/// Implicit nodes inside a composite pedal.
pub const PEDAL_IN: &str = "pedal_in";
pub const PEDAL_OUT: &str = "pedal_out";

pub const INPUT_DESC: BlockDesc = BlockDesc { kind: INPUT, label: "Chitarra", category: Category::Io, params: &[] };

pub const OUTPUT_DESC: BlockDesc = BlockDesc {
    kind: OUTPUT,
    label: "Ampli (uscita)",
    category: Category::Io,
    params: &[ParamDesc::number("volume_db", "Volume", -60.0, 12.0, 0.0, "dB")],
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    /// Unique identifier in the graph (letters, digits, `_`, `-`).
    pub id: String,
    /// Type: a catalog block, `input`, `output` or `pedal:<id>`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Position in the editor; ignored by the engine.
    #[serde(default)]
    pub pos: [f32; 2],
    #[serde(default)]
    pub params: ParamValues,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeSpec {
    pub from: String,
    pub to: String,
}

/// A v2 preset: a graph of connected nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphFile {
    pub name: String,
    #[serde(default = "version_2")]
    pub version: u32,
    #[serde(default, rename = "node")]
    pub nodes: Vec<NodeSpec>,
    #[serde(default, rename = "edge")]
    pub edges: Vec<EdgeSpec>,
}

fn version_2() -> u32 {
    2
}

/// Knob exposed by a composite pedal: a 0..1 value mapped onto an internal parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobSpec {
    pub label: String,
    /// "<internal node id>.<parameter id>", e.g. "c.high_hz".
    pub target: String,
    /// Internal parameter values when the knob is at 0 and at 1.
    pub min: f32,
    pub max: f32,
    /// Initial knob position, 0..1.
    #[serde(default = "half")]
    pub default: f32,
}

fn half() -> f32 {
    0.5
}

/// A composite pedal (file `pedals/<id>.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PedalFile {
    pub name: String,
    #[serde(default = "utility")]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "node")]
    pub nodes: Vec<NodeSpec>,
    #[serde(default, rename = "edge")]
    pub edges: Vec<EdgeSpec>,
    #[serde(default, rename = "knob")]
    pub knobs: Vec<KnobSpec>,
}

fn utility() -> String {
    "utility".into()
}

/// A catalog entry as the interface sees it (base blocks, input/output and pedals).
#[derive(Debug, Clone, Serialize)]
pub struct CatalogItem {
    pub kind: String,
    pub label: String,
    pub category: Category,
    pub params: Vec<ParamDesc>,
    /// true = composite pedal (`pedal:<id>`), editable and deletable from the interface.
    pub composite: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl CatalogItem {
    pub fn from_desc(desc: &BlockDesc) -> Self {
        Self {
            kind: desc.kind.into(),
            label: desc.label.into(),
            category: desc.category,
            params: desc.params.to_vec(),
            composite: false,
            description: String::new(),
        }
    }

    /// The knobs exposed by a pedal become 0..1 parameters, with the label as the id.
    pub fn from_pedal(id: &str, pedal: &PedalFile) -> Self {
        let params = pedal
            .knobs
            .iter()
            .map(|k| {
                // ParamDesc needs static strings: pedal labels live
                // as long as the catalog, which is reloaded rarely.
                let label: &'static str = Box::leak(k.label.clone().into_boxed_str());
                ParamDesc {
                    id: label,
                    label,
                    kind: ParamKind::Number { min: 0.0, max: 1.0, default: k.default, unit: "", log: false, step: 0.0 },
                }
            })
            .collect();
        Self {
            kind: format!("{PEDAL_PREFIX}{id}"),
            label: pedal.name.clone(),
            category: Category::parse(&pedal.category),
            params,
            composite: true,
            description: pedal.description.clone(),
        }
    }
}

/// Full catalog: special nodes, base blocks and composite pedals.
pub fn catalog_items(pedals: &[(String, PedalFile)]) -> Vec<CatalogItem> {
    let mut items = vec![CatalogItem::from_desc(&INPUT_DESC), CatalogItem::from_desc(&OUTPUT_DESC)];
    items.extend(crate::dsp::catalog().iter().map(|e| CatalogItem::from_desc(e.desc)));
    items.extend(pedals.iter().map(|(id, p)| CatalogItem::from_pedal(id, p)));
    items
}
