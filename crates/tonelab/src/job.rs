//! Job folder (PLAN §14.2): same layout and JSON as the Python tool.
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use openpedal_dsp::wav::{self, WavWriter};
use serde::{Deserialize, Serialize};

pub const STEMS: [&str; 6] = ["guitar", "bass", "drums", "vocals", "piano", "other"];

/// Local copy of `openpedal_separate::Stems` (identical shape); replaced by a re-export at merge.
pub struct Stems {
    pub sample_rate: u32,
    pub stems: Vec<(String, [Vec<f32>; 2])>,
}

/// Multi-channel audio read from a job file.
pub struct Audio {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    /// Always two channels (mono is duplicated).
    pub fn stereo(&self) -> [&[f32]; 2] {
        let l = &self.channels[0];
        let r = self.channels.get(1).unwrap_or(l);
        [l, r]
    }
    pub fn len(&self) -> usize {
        self.channels.first().map_or(0, |c| c.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Mid,
    Left,
    Right,
}

impl std::str::FromStr for Channel {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "mid" => Channel::Mid,
            "left" => Channel::Left,
            "right" => Channel::Right,
            _ => bail!("channel must be mid, left or right"),
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Note {
    pub t: f64,
    pub dur: f64,
    pub midi: i32,
    pub amp: f64,
    #[serde(default)]
    pub octave_shifted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Part {
    pub id: String,
    pub label: String,
    pub start: f64,
    pub end: f64,
    pub channel: Channel,
    pub notes: Vec<Note>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Section {
    pub start: f64,
    pub end: f64,
    pub guitar_db: f64,
    pub corr: f64,
    pub side_mid_db: f64,
    pub centroid_hz: i64,
    pub kind: String,
    pub bass_db: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LowNote {
    pub start: f64,
    pub end: f64,
    pub hz: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Analysis {
    pub tempo_bpm: f64,
    pub key: String,
    pub sections: Vec<Section>,
    pub low_notes: Vec<LowNote>,
}

pub struct Job {
    pub dir: PathBuf,
}

fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t: String = out.trim_matches('-').chars().take(48).collect();
    let t = t.trim_matches('-').to_string();
    if t.is_empty() {
        "song".into()
    } else {
        t
    }
}

/// (year, month, day, h, m, s) in UTC.
fn utc_now() -> (i64, u32, u32, u32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

fn write_wav(path: &Path, channels: &[&[f32]], sr: u32) -> Result<()> {
    let mut w = WavWriter::create(path, channels.len() as u16, sr)?;
    for i in 0..channels[0].len() {
        for c in channels {
            w.write(c[i])?;
        }
    }
    w.finish()
}

fn read_audio(path: &Path) -> Result<Audio> {
    let d = wav::read(path)?;
    Ok(Audio { sample_rate: d.sample_rate, channels: d.channels })
}

impl Job {
    /// `<root>/<date>-<slug>[-n]/` with a `job.json` (rate and duration filled in by `write_mix`).
    pub fn create(root: &Path, title: &str, source_kind: &str) -> Result<Job> {
        Self::create_with_ref(root, title, source_kind, "")
    }

    pub fn create_with_ref(root: &Path, title: &str, source_kind: &str, source_ref: &str) -> Result<Job> {
        let (y, mo, d, h, mi, s) = utc_now();
        let base = format!("{y:04}-{mo:02}-{d:02}-{}", slugify(title));
        let (mut id, mut n) = (base.clone(), 2);
        while root.join(&id).exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        let dir = root.join(&id);
        std::fs::create_dir_all(&dir)?;
        let job = Job { dir };
        job.write_json(
            "job.json",
            &serde_json::json!({
                "id": id,
                "source": { "kind": source_kind, "ref": source_ref, "title": title },
                "created": format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}+00:00"),
                "sample_rate": 0,
                "duration_s": 0.0,
            }),
        )?;
        Ok(job)
    }

    pub fn open(dir: &Path) -> Result<Job> {
        if !dir.join("job.json").exists() {
            bail!("job not found: {}", dir.display());
        }
        Ok(Job { dir: dir.to_path_buf() })
    }

    pub fn id(&self) -> String {
        self.dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
    pub fn mix_path(&self) -> PathBuf {
        self.dir.join("mix.wav")
    }
    pub fn stem_path(&self, name: &str) -> PathBuf {
        self.dir.join("stems").join(format!("{name}.wav"))
    }
    pub fn part_dir(&self, id: &str) -> PathBuf {
        self.dir.join("parts").join(id)
    }

    pub fn write_json<T: Serialize>(&self, name: &str, v: &T) -> Result<()> {
        let mut s = serde_json::to_string_pretty(v)?;
        s.push('\n');
        std::fs::write(self.dir.join(name), s).with_context(|| format!("writing {name}"))
    }

    pub fn read_json<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<T> {
        let p = self.dir.join(name);
        let s = std::fs::read_to_string(&p).with_context(|| format!("{} missing (run the earlier step first)", p.display()))?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn write_mix(&self, l: &[f32], r: &[f32], sr: u32) -> Result<()> {
        write_wav(&self.mix_path(), &[l, r], sr)?;
        let mut j: serde_json::Value = self.read_json("job.json")?;
        j["sample_rate"] = sr.into();
        j["duration_s"] = serde_json::json!((l.len() as f64 / sr as f64 * 1000.0).round() / 1000.0);
        self.write_json("job.json", &j)
    }

    pub fn write_stems(&self, s: &Stems) -> Result<()> {
        for (name, [l, r]) in &s.stems {
            write_wav(&self.stem_path(name), &[l, r], s.sample_rate)?;
        }
        Ok(())
    }

    pub fn read_mix(&self) -> Result<Audio> {
        read_audio(&self.mix_path())
    }
    /// `None` when the stem file does not exist.
    pub fn read_stem(&self, name: &str) -> Result<Option<Audio>> {
        let p = self.stem_path(name);
        if !p.exists() {
            return Ok(None);
        }
        Ok(Some(read_audio(&p)?))
    }
    pub fn read_analysis(&self) -> Result<Analysis> {
        self.read_json("analysis.json")
    }
    pub fn read_parts(&self) -> Result<Vec<Part>> {
        if !self.dir.join("parts.json").exists() {
            return Ok(vec![]);
        }
        self.read_json("parts.json")
    }
    pub fn write_parts(&self, parts: &[Part]) -> Result<()> {
        self.write_json("parts.json", &parts)
    }
    pub fn write_part_wav(&self, id: &str, file: &str, mono: &[f32], sr: u32) -> Result<()> {
        write_wav(&self.part_dir(id).join(file), &[mono], sr)
    }
    pub fn read_part_wav(&self, id: &str, file: &str) -> Result<Audio> {
        read_audio(&self.part_dir(id).join(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let root = std::env::temp_dir().join(format!("tonelab-job-{}", std::process::id()));
        let job = Job::create(&root, "Crème Brûlée!", "file").unwrap();
        assert!(job.id().ends_with("-cr-me-br-l-e"));
        let l: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        job.write_mix(&l, &l, 48000).unwrap();
        let m = job.read_mix().unwrap();
        assert_eq!(m.sample_rate, 48000);
        assert_eq!(m.channels[1], l);
        let j: serde_json::Value = job.read_json("job.json").unwrap();
        assert_eq!(j["sample_rate"], 48000);
        assert!(job.read_stem("guitar").unwrap().is_none());
        let p = Part { id: "p1".into(), label: "x".into(), start: 1.0, end: 2.0, channel: Channel::Mid, notes: vec![] };
        job.write_parts(&[p]).unwrap();
        assert_eq!(job.read_parts().unwrap()[0].channel, Channel::Mid);
        let _ = std::fs::remove_dir_all(root);
    }
}
