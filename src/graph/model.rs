//! Tipi del grafo (preset v2) e dei pedali composti: il formato dei file TOML
//! e dei messaggi JSON verso l'interfaccia. Contratto: vedi docs/PLAN.md §5.3.

use serde::{Deserialize, Serialize};

use crate::dsp::params::{BlockDesc, Category, ParamDesc, ParamKind, ParamValues};

/// Tipo del nodo di ingresso (la chitarra): uno per grafo, senza ingressi.
pub const INPUT: &str = "input";
/// Tipo del nodo di uscita ("Ampli"): uno per grafo; il suono è ciò che arriva qui.
pub const OUTPUT: &str = "output";
/// Prefisso del tipo di un nodo che usa un pedale composto: "pedal:<id>".
pub const PEDAL_PREFIX: &str = "pedal:";
/// Nodi impliciti dentro un pedale composto.
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
    /// Identificatore unico nel grafo (lettere, cifre, `_`, `-`).
    pub id: String,
    /// Tipo: un blocco del catalogo, `input`, `output` oppure `pedal:<id>`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Posizione nell'editor; ignorata dal motore.
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

/// Un preset v2: grafo di nodi collegati.
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

/// Manopola esposta da un pedale composto: valore 0..1 mappato su un parametro interno.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobSpec {
    pub label: String,
    /// "<id nodo interno>.<id parametro>", es. "c.high_hz".
    pub target: String,
    /// Valori del parametro interno quando la manopola è a 0 e a 1.
    pub min: f32,
    pub max: f32,
    /// Posizione iniziale della manopola, 0..1.
    #[serde(default = "half")]
    pub default: f32,
}

fn half() -> f32 {
    0.5
}

/// Un pedale composto (file `pedals/<id>.toml`).
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

/// Una voce del catalogo come la vede l'interfaccia (blocchi base, io e pedali).
#[derive(Debug, Clone, Serialize)]
pub struct CatalogItem {
    pub kind: String,
    pub label: String,
    pub category: Category,
    pub params: Vec<ParamDesc>,
    /// true = pedale composto (`pedal:<id>`), modificabile e cancellabile dall'interfaccia.
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

    /// Le manopole esposte di un pedale diventano parametri 0..1 con l'etichetta come id.
    pub fn from_pedal(id: &str, pedal: &PedalFile) -> Self {
        let params = pedal
            .knobs
            .iter()
            .map(|k| {
                // I ParamDesc vogliono stringhe statiche: le etichette dei pedali vivono
                // quanto il catalogo, che si ricarica raramente.
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

/// Catalogo completo: nodi speciali, blocchi base e pedali composti.
pub fn catalog_items(pedals: &[(String, PedalFile)]) -> Vec<CatalogItem> {
    let mut items = vec![CatalogItem::from_desc(&INPUT_DESC), CatalogItem::from_desc(&OUTPUT_DESC)];
    items.extend(crate::dsp::catalog().iter().map(|e| CatalogItem::from_desc(e.desc)));
    items.extend(pedals.iter().map(|(id, p)| CatalogItem::from_pedal(id, p)));
    items
}
