//! Audio backends: who calls `AudioCore::process` with real samples.
//!
//! - `jack` (Linux): a JACK client, served by PipeWire.
//! - iOS (and any host that owns the audio callback): no backend at all; the host calls
//!   `openpedal_process` through the C ABI in `crate::ffi`.
//! - Windows/macOS (cpal): task E1.

use std::any::Any;

use anyhow::Result;

use crate::engine::AudioCore;

#[cfg(target_os = "linux")]
pub mod jack;

/// An audio API that drives the engine's real-time callback.
pub trait AudioBackend {
    /// Sample rate the device runs at (needed to compile the first plan).
    fn sample_rate(&self) -> f32;

    /// Starts the audio stream, which takes ownership of `core` and calls `AudioCore::process`
    /// from the audio thread. The returned value keeps the stream alive: dropping it stops the audio.
    fn start(self: Box<Self>, core: AudioCore) -> Result<Box<dyn Any + Send>>;
}
