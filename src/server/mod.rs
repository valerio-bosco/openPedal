//! HTTP + WebSocket server for the interface (task B1, see docs/PLAN.md §7).
//!
//! The server does not know about the audio engine: it only talks to an `EngineHandle`.
//! So it can be developed and tested with a fake engine (MockEngine), and the integration
//! plugs in the real one.

mod app;
mod assets;
mod files;
pub mod mock;
pub mod protocol;

pub use app::{run, ServerOptions};

use anyhow::Result;

use crate::graph::model::{CatalogItem, GraphFile};
use protocol::{AudioInfo, MetersMsg};

/// What the server can ask of the engine. All methods are called from
/// normal threads (never from the audio one) and must respond quickly.
/// The server itself reads and writes the preset and pedal files.
pub trait EngineHandle: Send + Sync + 'static {
    /// Loaded base blocks, special nodes and composite pedals.
    fn catalog(&self) -> Vec<CatalogItem>;
    /// The graph currently playing.
    fn graph(&self) -> GraphFile;
    fn audio(&self) -> AudioInfo;
    fn meters(&self) -> MetersMsg;
    /// Compiles and starts a graph. On error the old one stays;
    /// the message is in English and, if possible, names the offending node.
    fn set_graph(&self, graph: GraphFile) -> Result<()>;
    /// Live change of a parameter (also updates the graph returned by `graph`).
    fn set_param(&self, node: &str, param: &str, value: f32) -> Result<()>;
    /// Live bypass of a node (also updates the graph returned by `graph`).
    /// Error for `input`/`output` and for unknown nodes.
    fn set_bypass(&self, node: &str, bypass: bool) -> Result<()>;
    /// To be called after writing or deleting files in pedals/.
    fn reload_pedals(&self) -> Result<()>;
}
