//! WebSocket messages between the interface and openPedal (contract: docs/PLAN.md §5.4).
//! The JSON examples in apps/desktop/ui/src/fixtures/ are generated from these types
//! (`openpedal dev-fixtures apps/desktop/ui/src/fixtures`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use openpedal_graph::model::{CatalogItem, GraphFile, PedalFile};

pub mod codegen;

/// Version of the WebSocket contract (docs/PLAN.md §5.4), sent in `state`.
/// Bumped on any breaking change; additive optional fields do not bump it.
/// 2: server-side chain edits and undo/redo, new `param` and `added` server messages,
/// `can_undo`/`can_redo`/`dirty` in `state`, and state broadcast to every client.
pub const PROTOCOL_VERSION: u32 = 2;

/// From the client (interface) to the server.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// The server replies with `state`.
    Hello,
    /// Live change of a parameter, without rebuilding the graph.
    /// For choices, `value` is the option index.
    SetParam { node: String, param: String, value: f32 },
    /// Live bypass of a node (click-free, no graph rebuild). Not allowed on `input`/`output`.
    SetBypass { node: String, bypass: bool },
    /// New graph (structure changed): recompiled and swapped in with a fade.
    SetGraph { graph: GraphFile },
    LoadPreset { name: String },
    SavePreset { name: String, graph: GraphFile },
    /// Deletes presets/<name>.toml; the graph currently playing does not change.
    DeletePreset { name: String },
    /// Writes pedals/<id>.toml; the server then sends `state` back with the updated catalog.
    SavePedal { id: String, pedal: PedalFile },
    DeletePedal { id: String },
    /// Inserts a new pedal of type `kind` into a linear chain, after the node `after`
    /// (default: just before the output). The server picks a unique id. Replies with `state`
    /// then `added`; `error` if the graph is not a simple chain.
    AddNode {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
    },
    /// Removes a pedal from a linear chain (not `input`/`output`) and reconnects its neighbours.
    RemoveNode { node: String },
    /// Moves a pedal of a linear chain to position `to` among the pedals (0 = right after the
    /// input; clamped). The input stays first and the output last.
    MoveNode { node: String, to: u32 },
    /// Undoes the last step of the shared history (replies with `state`).
    Undo,
    /// Redoes the last undone step.
    Redo,
    /// Call once at the start of a continuous gesture (a knob drag): the `set_param` messages
    /// that follow become a single undo step.
    EditBegin,
}

/// From the server to the client.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    State(StateMsg),
    Meters(MetersMsg),
    /// A `set_param` made by another client, forwarded without rebuilding anything
    /// (never sent back to the client that made it). For choices `value` is the option index.
    Param { node: String, param: String, value: f32 },
    /// Sent to the client that asked for `add_node`, right after the `state` that contains the node.
    Added { node: String },
    Error {
        /// In English, to show to the user.
        message: String,
        /// The node the error refers to, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct StateMsg {
    /// `PROTOCOL_VERSION` of this server.
    pub protocol: u32,
    pub catalog: Vec<CatalogItem>,
    pub graph: GraphFile,
    /// Names of the available presets (files in presets/ without extension).
    pub presets: Vec<String>,
    /// Content of the composite pedals (id -> pedal), so they can be opened and edited.
    pub pedals: BTreeMap<String, PedalFile>,
    pub audio: AudioInfo,
    /// The shared history has something to undo / redo.
    pub can_undo: bool,
    pub can_redo: bool,
    /// The graph differs from the last loaded or saved preset.
    pub dirty: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub buffer: u32,
    /// false = no audio card connected (editor-only mode).
    pub running: bool,
}

/// Sent about 20 times per second.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MetersMsg {
    pub in_db: f32,
    pub out_db: f32,
    /// Load of the audio thread, 0..1 (1 = out of time).
    pub dsp: f32,
    pub xruns: u64,
}
