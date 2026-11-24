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
/// 3: pedal files v2 (`control`/`route`, `color`, `version`; `knob` is gone), `color` on catalog
/// items, the `analyze_pedal`/`analyze_node` requests and the `response` reply.
/// 4: Tone Lab (docs/PLAN.md §14.5): the `tonelab_*` client and server messages.
pub const PROTOCOL_VERSION: u32 = 4;

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
    /// Frequency response of a pedal file, which may be an unsaved draft, with the given control
    /// values (knob position 0..1, or option index of a switch/selector; missing ones take their
    /// default). The server replies with `response`, or `error` if the pedal is invalid.
    AnalyzePedal { pedal: PedalFile, values: BTreeMap<String, f32> },
    /// Frequency response of a node of the playing graph, with its current values.
    /// The server replies with `response`.
    AnalyzeNode { node: String },
    /// Call once at the start of a continuous gesture (a knob drag): the `set_param` messages
    /// that follow become a single undo step.
    EditBegin,
    /// Asks what Tone Lab can do on this machine. Replies with `tonelab_caps`.
    TonelabCaps,
    /// Lists the Tone Lab jobs. Replies with `tonelab_jobs`.
    TonelabJobs,
    /// Starts a job from a URL or from a path returned by `POST /tonelab/upload`: fetch, separate and
    /// analyse. Replies with `tonelab_progress` messages, then `tonelab_job`.
    TonelabStart { source: String },
    /// Reads a job from disk. Replies with `tonelab_job`.
    TonelabOpen { job: String },
    /// Picks a part of the song (a time range and a stereo channel), transcribes it and builds a
    /// synthetic DI. Replies with `tonelab_progress` messages, then `tonelab_job`.
    TonelabPick { job: String, start: f32, end: f32, channel: TonelabChannel, label: String },
    /// Records the live guitar input into `parts/<part>/di-user.wav`, then replies with `tonelab_job`.
    TonelabRecordDi { job: String, part: String, seconds: f32 },
    /// Fits a preset to a part. Replies with `tonelab_progress` messages, then `tonelab_fit_result`.
    TonelabFit { job: String, part: String },
    /// Saves the fitted preset of a part into the library under `name` (like `save_preset`).
    TonelabSavePreset { job: String, part: String, name: String },
    /// Stops the step running for a job (`tonelab_progress` with stage `cancelled`).
    TonelabCancel { job: String },
}

/// Which side of the stereo image a part is taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TonelabChannel {
    Mid,
    Left,
    Right,
}

/// What a `tonelab_progress` message is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TonelabStage {
    Fetch,
    Separate,
    Analyse,
    Parts,
    Synth,
    Record,
    Fit,
    Cancelled,
}

/// A job in the list.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TonelabJobSummary {
    pub id: String,
    pub title: String,
    pub created: String,
    pub duration_s: f32,
}

/// A stretch of the song with guitar in it.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TonelabSection {
    pub start: f32,
    pub end: f32,
    /// `centre`, `double`, `wide` or `none`.
    pub kind: String,
    pub guitar_db: f32,
    pub corr: f32,
}

/// The result of a fit.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TonelabFit {
    pub template: String,
    /// Spectral error on the fit window, in dB.
    pub error_db: f32,
    /// Error on the held-out 30 % (only when fitted on a real recording).
    pub holdout_error_db: Option<f32>,
    /// Error on another real DI given for validation.
    pub validate_error_db: Option<f32>,
}

/// A part of the song the user picked.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TonelabPart {
    pub id: String,
    pub label: String,
    pub start: f32,
    pub end: f32,
    pub channel: TonelabChannel,
    /// Number of transcribed notes.
    pub notes: u32,
    pub has_user_di: bool,
    pub has_synth_di: bool,
    pub fit: Option<TonelabFit>,
}

/// Everything the interface needs about one job.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TonelabJobInfo {
    pub id: String,
    pub title: String,
    /// `file` or `youtube`.
    pub source_kind: String,
    pub duration_s: f32,
    pub tempo_bpm: Option<f32>,
    pub key: Option<String>,
    pub sections: Vec<TonelabSection>,
    pub parts: Vec<TonelabPart>,
}

/// One template of a fit, as in the final table.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TonelabTemplateResult {
    pub template: String,
    pub error_db: f32,
    pub renders: u32,
    /// `screen` or `fit`.
    pub stage: String,
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
    /// Reply to `analyze_pedal` / `analyze_node`, sent only to the client that asked: the magnitude
    /// response, `db[i]` at `freq[i]` Hz (about 128 log-spaced points, 20 Hz to 20 kHz).
    Response { freq: Vec<f32>, db: Vec<f32> },
    /// Sent to the client that asked for `add_node`, right after the `state` that contains the node.
    Added { node: String },
    /// Reply to `tonelab_caps`.
    TonelabCaps {
        /// The `tonelab` tool was found and runs.
        available: bool,
        /// The YouTube link field can be shown.
        youtube: bool,
        /// `cuda` or `cpu`.
        device: String,
        /// Why `available` is false.
        reason: Option<String>,
    },
    /// Reply to `tonelab_jobs`.
    TonelabJobs { jobs: Vec<TonelabJobSummary> },
    /// Progress of a running step. `fraction` is 0..1, or null when unknown.
    TonelabProgress { job: String, stage: TonelabStage, message: String, fraction: Option<f32> },
    /// A job as it is on disk now.
    TonelabJob { job: TonelabJobInfo },
    /// A fit finished.
    TonelabFitResult { job: String, part: String, fit: TonelabFit, templates: Vec<TonelabTemplateResult> },
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
