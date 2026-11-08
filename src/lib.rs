//! openPedal core: DSP blocks, graph compiler, audio engine, web server and the C ABI.
//!
//! The desktop binary (`src/main.rs`) is a thin CLI over this library; on iOS the Swift
//! shell links the static library and talks to it through `ffi` (`openpedal.h`).

// `Smoothed::next` is a per-sample tick, not an Iterator.
#![allow(clippy::should_implement_trait)]

pub mod audio;
pub mod dsp;
pub mod engine;
pub mod ffi;
pub mod graph;
pub mod live;
pub mod server;
pub mod wav;
