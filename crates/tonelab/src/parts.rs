//! `parts` and `synth-di`: pick a section + channel, transcribe it, synthesise a DI (port of parts.py).
use anyhow::{bail, Context, Result};

use crate::dsp::resample_poly;
use crate::job::{Channel, Job, Part};
use crate::synth::synth_di;
use crate::transcribe::transcribe;

pub const PART_RATE: u32 = 48000;

fn fmt_g(v: f32) -> String {
    let s = format!("{v}");
    s.strip_suffix(".0").unwrap_or(&s).to_string()
}

/// Excerpt `[start, end)` s of the guitar stem, one channel, mono 48 kHz -> `parts/<id>/target.wav`,
/// notes -> `parts.json`. An empty `label` becomes "<channel> <start>-<end>s".
pub fn pick_part(job: &Job, start: f32, end: f32, channel: Channel, label: &str) -> Result<Part> {
    if end <= start {
        bail!("end must be greater than start");
    }
    let g = job.read_stem("guitar")?.context("stems/guitar.wav missing")?;
    let sr = g.sample_rate as f64;
    let [l, r] = g.stereo();
    let (a, b) = ((start as f64 * sr) as usize, ((end as f64 * sr) as usize).min(l.len()));
    if a >= b {
        bail!("the selected range is outside the song");
    }
    let mono: Vec<f32> = match (g.channels.len(), channel) {
        (1, _) | (_, Channel::Left) => l[a..b].to_vec(),
        (_, Channel::Right) => r[a..b].to_vec(),
        (_, Channel::Mid) => l[a..b].iter().zip(&r[a..b]).map(|(x, y)| (x + y) / 2.0).collect(),
    };
    let target = resample_poly(&mono, g.sample_rate, PART_RATE);
    let mut parts = job.read_parts()?;
    let n = parts.iter().filter_map(|p| p.id.strip_prefix('p').and_then(|s| s.parse::<u32>().ok())).max().unwrap_or(0) + 1;
    let id = format!("p{n}");
    job.write_part_wav(&id, "target.wav", &target, PART_RATE)?;
    let ch = match channel {
        Channel::Mid => "mid",
        Channel::Left => "left",
        Channel::Right => "right",
    };
    let part = Part {
        id,
        label: if label.is_empty() { format!("{ch} {}-{}s", fmt_g(start), fmt_g(end)) } else { label.to_string() },
        start: start as f64,
        end: end as f64,
        channel,
        notes: transcribe(&target, PART_RATE),
    };
    parts.push(part.clone());
    job.write_parts(&parts)?;
    Ok(part)
}

/// Synthetic DI of a part -> `parts/<id>/di-synth.wav` (mono 48 kHz).
pub fn synth_part(job: &Job, part_id: &str, match_di: Option<&[f32]>) -> Result<std::path::PathBuf> {
    let parts = job.read_parts()?;
    let part = parts.iter().find(|p| p.id == part_id).with_context(|| format!("no part {part_id} in {}", job.id()))?;
    if part.notes.is_empty() {
        bail!("part {part_id} has no notes (transcription empty)");
    }
    let out = synth_di(&part.notes, PART_RATE, match_di);
    job.write_part_wav(part_id, "di-synth.wav", &out, PART_RATE)?;
    Ok(job.part_dir(part_id).join("di-synth.wav"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Stems;

    #[test]
    fn pick_and_synth_on_a_synthetic_job() {
        let root = std::env::temp_dir().join(format!("tonelab-parts-{}", std::process::id()));
        let job = Job::create(&root, "t", "file").unwrap();
        let sr = 44100u32;
        let mut l = vec![0.0f32; sr as usize * 4];
        for i in 0..sr as usize * 2 {
            let t = i as f64 / sr as f64;
            l[sr as usize + i] = (0.5 * (-t * 3.0).exp() * (1.0 - (-t * 400.0).exp()) * (2.0 * std::f64::consts::PI * 196.0 * t).sin()) as f32;
        }
        let r = vec![0.0f32; l.len()];
        job.write_mix(&l, &r, sr).unwrap();
        job.write_stems(&Stems { sample_rate: sr, stems: vec![("guitar".into(), [l, r])] }).unwrap();
        let p = pick_part(&job, 0.5, 3.5, Channel::Left, "").unwrap();
        assert_eq!(p.id, "p1");
        assert_eq!(p.label, "left 0.5-3.5s");
        assert!(p.notes.iter().any(|n| n.midi == 55), "{:?}", p.notes);
        let t = job.read_part_wav("p1", "target.wav").unwrap();
        assert_eq!((t.sample_rate, t.channels[0].len()), (48000, 144000));
        synth_part(&job, "p1", None).unwrap();
        assert!(job.part_dir("p1").join("di-synth.wav").exists());
        assert_eq!(pick_part(&job, 0.0, 1.0, Channel::Mid, "x").unwrap().id, "p2");
        let _ = std::fs::remove_dir_all(root);
    }
}
