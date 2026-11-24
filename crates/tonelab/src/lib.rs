//! Tone Lab in Rust (PLAN §14, §15): job folder, analysis, transcription, synthetic DI.
pub mod analyse;
pub mod dsp;
pub mod job;
pub mod parts;
pub mod pyin;
pub mod synth;
pub mod transcribe;
pub use job::*;
pub use transcribe::transcribe;
pub use synth::synth_di;
pub use analyse::{analyse, analyse_audio};
pub use parts::{pick_part, synth_part};
