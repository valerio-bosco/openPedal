//! WebSocket messages between the interface and openPedal (contract: docs/PLAN.md §5.4).
//! The JSON examples in ui/src/fixtures/ are generated from these types
//! (`openpedal dev-fixtures ui/src/fixtures`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph::model::{CatalogItem, GraphFile, PedalFile};

/// From the client (interface) to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// The server replies with `state`.
    Hello,
    /// Live change of a parameter, without rebuilding the graph.
    /// For choices, `value` is the option index.
    SetParam { node: String, param: String, value: f32 },
    /// New graph (structure changed): recompiled and swapped in with a fade.
    SetGraph { graph: GraphFile },
    LoadPreset { name: String },
    SavePreset { name: String, graph: GraphFile },
    /// Deletes presets/<name>.toml; the graph currently playing does not change.
    DeletePreset { name: String },
    /// Writes pedals/<id>.toml; the server then sends `state` back with the updated catalog.
    SavePedal { id: String, pedal: PedalFile },
    DeletePedal { id: String },
}

/// From the server to the client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    State(StateMsg),
    Meters(MetersMsg),
    Error {
        /// In English, to show to the user.
        message: String,
        /// The node the error refers to, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct StateMsg {
    pub catalog: Vec<CatalogItem>,
    pub graph: GraphFile,
    /// Names of the available presets (files in presets/ without extension).
    pub presets: Vec<String>,
    /// Content of the composite pedals (id -> pedal), so they can be opened and edited.
    pub pedals: BTreeMap<String, PedalFile>,
    pub audio: AudioInfo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub buffer: u32,
    /// false = no audio card connected (editor-only mode).
    pub running: bool,
}

/// Sent about 20 times per second.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MetersMsg {
    pub in_db: f32,
    pub out_db: f32,
    /// Load of the audio thread, 0..1 (1 = out of time).
    pub dsp: f32,
    pub xruns: u64,
}
