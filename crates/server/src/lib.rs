//! HTTP + WebSocket server for the interface (task B1, see docs/PLAN.md §7).
//!
//! The server does not know about the audio engine: it only talks to an `EngineHandle`
//! (defined in `openpedal-engine`). So it can be developed and tested with a fake engine
//! (MockEngine), and the integration plugs in the real one.

mod app;
mod assets;
pub mod mock;
#[cfg(feature = "native-tonelab")]
mod native_adapter;
mod native_backend;
mod tonelab;
mod tonelab_backend;
pub mod tonelab_mock;

pub use tonelab::{set_native_tonelab, NativeConfig};
pub use app::{run, ServerOptions};
pub use openpedal_engine::EngineHandle;
