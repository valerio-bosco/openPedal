//! The only place that touches the crates `openpedal-separate` and `openpedal-tonelab` (§15
//! "Wave 1 APIs"), behind the feature `native-tonelab`. The functions are stubs until the two
//! branches are merged: the merge replaces their bodies with the calls named in the comments.

use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Result};
use openpedal_protocol::TonelabChannel;

use crate::native_backend::{JobOps, Mix, Separator, Stems};

fn not_wired<T>() -> Result<T> {
    bail!("native Tone Lab is not wired yet: openpedal-separate and openpedal-tonelab are not merged")
}

/// `openpedal_separate::DemucsCpp::load(weights)` as an `Arc<dyn Separator>`.
pub fn load_separator(_weights: &Path) -> Result<Arc<dyn Separator>> {
    not_wired()
}

/// The `openpedal_tonelab` job functions.
pub struct TonelabJobOps;

impl JobOps for TonelabJobOps {
    /// `Job::create`-like: writes `job.json` into `dir` (title, "file" kind, duration from `mix.wav`).
    fn init_job(&self, _dir: &Path, _title: &str) -> Result<()> {
        not_wired()
    }
    /// `Job::open(dir)` + reading `mix.wav` between `from` and `to`.
    fn read_mix(&self, _dir: &Path, _from: f32, _to: Option<f32>) -> Result<(Mix, f32)> {
        not_wired()
    }
    /// Reads the stem offset the excerpt writer recorded (`stems/excerpt.json`), if any.
    fn stems_range(&self, _dir: &Path) -> Option<(f32, f32)> {
        None
    }
    /// `Job::write_stems(&Stems)` (+ the offset of the excerpt).
    fn write_stems(&self, _dir: &Path, _stems: &Stems, _offset_s: f32) -> Result<()> {
        not_wired()
    }
    /// `analysis.json` exists.
    fn has_analysis(&self, dir: &Path) -> bool {
        dir.join("analysis.json").is_file()
    }
    /// `openpedal_tonelab::analyse(&job)`.
    fn analyse(&self, _dir: &Path) -> Result<()> {
        not_wired()
    }
    /// `openpedal_tonelab::pick_part(&job, start, end, channel, label)`, returns `Part.id`.
    fn pick_part(&self, _dir: &Path, _start: f32, _end: f32, _channel: TonelabChannel, _label: &str) -> Result<String> {
        not_wired()
    }
    /// `transcribe` is inside `pick_part`; this is `synth_di(&notes, sr, match_di)` written to `di-synth.wav`.
    fn synth_di(&self, _dir: &Path, _part: &str, _match_di: Option<&Path>) -> Result<()> {
        not_wired()
    }
}
