//! The only place that touches the crates `openpedal-separate` and `openpedal-tonelab` (§15),
//! behind the feature `native-tonelab`.
//!
//! Stems of an excerpt live in `stems/*.wav` with `stems/excerpt.json` = `{"offset_s", "duration_s"}`
//! (where they start in the mix). Everything the adapter writes outside `stems/` (`analysis.json`,
//! `parts.json`) uses times of the whole song, like the Python tool.

use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use openpedal_protocol::TonelabChannel;
use openpedal_separate::DemucsCpp;
use openpedal_tonelab::{Audio, Channel, Job};

use crate::native_backend::{JobOps, Mix, Separator, Stems};

/// Threads for separation: `$OPENPEDAL_DEMUCS_THREADS`, default 2 (each needs about 1.5 GB: iPad memory).
pub const DEFAULT_THREADS: usize = 2;

fn threads() -> usize {
    std::env::var("OPENPEDAL_DEMUCS_THREADS").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(DEFAULT_THREADS)
}

/// `DemucsCpp::load(weights)` as an `Arc<dyn Separator>`.
pub fn load_separator(weights: &Path) -> Result<Arc<dyn Separator>> {
    Ok(Arc::new(DemucsCpp::load(weights)?.with_threads(threads())))
}

fn channel(c: TonelabChannel) -> Channel {
    match c {
        TonelabChannel::Mid => Channel::Mid,
        TonelabChannel::Left => Channel::Left,
        TonelabChannel::Right => Channel::Right,
    }
}

fn fmt_g(v: f32) -> String {
    let s = format!("{v}");
    s.strip_suffix(".0").unwrap_or(&s).to_string()
}

fn excerpt(dir: &Path) -> Option<(f32, f32)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("stems/excerpt.json")).ok()?).ok()?;
    Some((v["offset_s"].as_f64()? as f32, v["duration_s"].as_f64()? as f32))
}

/// The `openpedal_tonelab` job functions.
pub struct TonelabJobOps;

impl JobOps for TonelabJobOps {
    /// Writes `job.json` (title, "file" kind, rate and duration from `mix.wav`).
    fn init_job(&self, dir: &Path, title: &str) -> Result<()> {
        let mix = openpedal_dsp::wav::read(&dir.join("mix.wav"))?;
        let n = mix.channels.first().map_or(0, |c| c.len());
        let json = serde_json::json!({
            "id": dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            "source": { "kind": "file", "ref": "", "title": title },
            "sample_rate": mix.sample_rate,
            "duration_s": (n as f64 / mix.sample_rate.max(1) as f64 * 1000.0).round() / 1000.0,
        });
        std::fs::write(dir.join("job.json"), serde_json::to_string_pretty(&json)? + "\n")?;
        Ok(())
    }

    fn read_mix(&self, dir: &Path, from: f32, to: Option<f32>) -> Result<(Mix, f32)> {
        let a = Job::open(dir)?.read_mix()?;
        let sr = a.sample_rate;
        let [l, r] = a.stereo();
        let at = |s: f32| ((s.max(0.0) as f64 * sr as f64) as usize).min(l.len());
        let (i, j) = (at(from), to.map_or(l.len(), at));
        if i >= j {
            bail!("the selected range is outside the song");
        }
        Ok((Mix { left: l[i..j].to_vec(), right: r[i..j].to_vec(), sample_rate: sr }, i as f32 / sr as f32))
    }

    fn stems_range(&self, dir: &Path) -> Option<(f32, f32)> {
        if !dir.join("stems/guitar.wav").is_file() {
            return None;
        }
        match excerpt(dir) {
            Some((off, dur)) => Some((off, off + dur)),
            None => Some((0.0, f32::MAX)), // whole-song stems
        }
    }

    fn write_stems(&self, dir: &Path, stems: &Stems, offset_s: f32) -> Result<()> {
        let job = Job::open(dir)?;
        job.write_stems(stems)?;
        let n = stems.stems.first().map_or(0, |(_, c)| c[0].len());
        job.write_json("stems/excerpt.json", &serde_json::json!({ "offset_s": offset_s, "duration_s": n as f64 / stems.sample_rate as f64 }))
    }

    fn has_analysis(&self, dir: &Path) -> bool {
        dir.join("analysis.json").is_file()
    }

    /// Analyses the stems' range of the mix and stores the times as song times.
    fn analyse(&self, dir: &Path) -> Result<()> {
        let job = Job::open(dir)?;
        let guitar = job.read_stem("guitar")?.context("stems/guitar.wav missing (run separation first)")?;
        let bass = job.read_stem("bass")?;
        let mix = job.read_mix()?;
        let off = excerpt(dir).map_or(0.0, |(o, _)| o as f64);
        let sr = mix.sample_rate as f64;
        let n = (guitar.len() as f64 / guitar.sample_rate as f64 * sr) as usize;
        let a = (off * sr) as usize;
        let cut = Audio { sample_rate: mix.sample_rate, channels: mix.channels.iter().map(|c| c[a.min(c.len())..(a + n).min(c.len())].to_vec()).collect() };
        let mut an = openpedal_tonelab::analyse_audio(&guitar, bass.as_ref(), &cut);
        for s in &mut an.sections {
            s.start += off;
            s.end += off;
        }
        for l in &mut an.low_notes {
            l.start += off;
            l.end += off;
        }
        job.write_json("analysis.json", &an)
    }

    fn pick_part(&self, dir: &Path, start: f32, end: f32, ch: TonelabChannel, label: &str) -> Result<String> {
        let job = Job::open(dir)?;
        let off = excerpt(dir).map_or(0.0, |(o, _)| o);
        let ch = channel(ch);
        let part = openpedal_tonelab::pick_part(&job, start - off, end - off, ch, "")?;
        // The part is stored with song times.
        let mut parts = job.read_parts()?;
        if let Some(p) = parts.iter_mut().find(|p| p.id == part.id) {
            p.start = start as f64;
            p.end = end as f64;
            p.label = if label.is_empty() {
                let name = match ch {
                    Channel::Mid => "mid",
                    Channel::Left => "left",
                    Channel::Right => "right",
                };
                format!("{name} {}-{}s", fmt_g(start), fmt_g(end))
            } else {
                label.to_string()
            };
        }
        job.write_parts(&parts)?;
        Ok(part.id)
    }

    fn synth_di(&self, dir: &Path, part: &str, match_di: Option<&Path>) -> Result<()> {
        let job = Job::open(dir)?;
        let di = match match_di {
            Some(p) => Some(openpedal_dsp::wav::read(p)?.channels.swap_remove(0)),
            None => None,
        };
        openpedal_tonelab::synth_part(&job, part, di.as_deref())?;
        Ok(())
    }
}
