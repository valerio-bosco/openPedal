//! Tone Lab in Rust (PLAN §14, §15): job folder, analysis, transcription, synthetic DI.
pub mod dsp;
pub mod job;
pub mod pyin;
pub mod transcribe;
pub use job::*;
pub use transcribe::transcribe;
