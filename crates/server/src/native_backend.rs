//! Tone Lab in process (docs/PLAN.md §15): Demucs separation + the Rust analysis, no Python.
//!
//! The crates `openpedal-separate` and `openpedal-tonelab` are reached only through the traits
//! below (`Separator` has the §15 "Wave 1 APIs" signature; `JobOps` wraps the `Job` functions) and
//! the stubs of `native_adapter.rs` (feature `native-tonelab`), so the tests run with fakes.
//!
//! iOS start flow: the app decodes the song itself, writes `<jobs>/<job>/mix.wav` and sends
//! `tonelab_start { source: "<jobs>/<job>/mix.wav" }`. `fetch` registers that folder as a job (no
//! separation yet); the stems of the excerpt picked by `tonelab_pick` (+ 2 s margins) are made then.
#![cfg_attr(not(feature = "native-tonelab"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use openpedal_library::Library;
use openpedal_protocol::{TonelabChannel, TonelabStage};

use crate::tonelab::{plain_name, StepError, ToolCaps};
use crate::tonelab_backend::{run_fit, Ctx, TonelabBackend};

/// Seconds of context separated on each side of a picked part.
pub const MARGIN_S: f32 = 2.0;

/// Mirror of `openpedal_separate::Stems` (§15).
pub struct Stems {
    pub sample_rate: u32,
    pub stems: Vec<(String, [Vec<f32>; 2])>,
}

/// Mirror of `openpedal_separate::Separator` (§15).
pub trait Separator: Send + Sync {
    fn separate(&self, left: &[f32], right: &[f32], sample_rate: u32, progress: &mut dyn FnMut(f32, &str), cancel: &AtomicBool) -> Result<Stems>;
}

/// A stereo piece of the mix.
pub struct Mix {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub sample_rate: u32,
}

/// The job-folder operations of `openpedal-tonelab` (§15): `Job::create/open/write_mix/write_stems`,
/// `analyse`, `pick_part`, `synth_di`. `dir` is the job folder.
pub trait JobOps: Send + Sync {
    /// Writes `job.json` for a folder that has a `mix.wav` (title, duration, source kind "file").
    fn init_job(&self, dir: &Path, title: &str) -> Result<()>;
    /// `from..to` seconds of `mix.wav`, clamped to its length (`to` None = the end). Returns it with its start in seconds.
    fn read_mix(&self, dir: &Path, from: f32, to: Option<f32>) -> Result<(Mix, f32)>;
    /// The range of the mix the stems in `stems/` cover (seconds), if there are any.
    fn stems_range(&self, dir: &Path) -> Option<(f32, f32)>;
    /// Writes `stems/*.wav`; `offset_s` = where the stems start in the mix (0 for a whole song).
    fn write_stems(&self, dir: &Path, stems: &Stems, offset_s: f32) -> Result<()>;
    fn has_analysis(&self, dir: &Path) -> bool;
    fn analyse(&self, dir: &Path) -> Result<()>;
    /// Adds a part to `parts.json` (target.wav + notes) and returns its id.
    fn pick_part(&self, dir: &Path, start: f32, end: f32, channel: TonelabChannel, label: &str) -> Result<String>;
    fn synth_di(&self, dir: &Path, part: &str, match_di: Option<&Path>) -> Result<()>;
}

type Loader = Box<dyn Fn() -> Result<Arc<dyn Separator>> + Send + Sync>;

pub struct NativeBackend {
    jobs_dir: PathBuf,
    library: Library,
    ops: Arc<dyn JobOps>,
    loader: Loader,
    separator: Mutex<Option<Arc<dyn Separator>>>,
}

impl NativeBackend {
    /// `loader` makes the separator on first use (loading the weights is slow and uses memory).
    pub fn new(jobs_dir: PathBuf, library: Library, ops: Arc<dyn JobOps>, loader: Loader) -> Self {
        Self { jobs_dir, library, ops, loader, separator: Mutex::new(None) }
    }

    fn separator(&self) -> Result<Arc<dyn Separator>, StepError> {
        let mut slot = self.separator.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some((self.loader)().map_err(|e| StepError::Failed(format!("cannot load the separation model: {e:#}")))?);
        }
        Ok(slot.clone().expect("just set"))
    }

    fn job_dir(&self, job: &str) -> Result<PathBuf, StepError> {
        if !plain_name(job) {
            return Err(StepError::Failed(format!("invalid job id '{job}'")));
        }
        let dir = self.jobs_dir.join(job);
        if !dir.join("job.json").is_file() {
            return Err(StepError::Failed(format!("job '{job}' not found")));
        }
        Ok(dir)
    }
}

impl TonelabBackend for NativeBackend {
    fn name(&self) -> &'static str {
        "native"
    }

    fn caps(&self) -> Result<ToolCaps, String> {
        std::fs::create_dir_all(&self.jobs_dir).map_err(|e| format!("cannot create {}: {e}", self.jobs_dir.display()))?;
        Ok(ToolCaps { youtube: false, device: "cpu".into(), jobs_dir: self.jobs_dir.clone() })
    }

    /// `source` is `<jobs dir>/<job>/mix.wav`; the job id is the folder's name.
    fn fetch(&self, ctx: &Ctx, source: &str) -> Result<String, StepError> {
        let bad = || StepError::Failed("the source must be the mix.wav of a job folder inside the Tone Lab jobs directory".to_string());
        let file = Path::new(source).canonicalize().map_err(|_| bad())?;
        let root = self.jobs_dir.canonicalize().map_err(|_| bad())?;
        let dir = file.parent().ok_or_else(bad)?;
        let id = dir.file_name().and_then(|n| n.to_str()).ok_or_else(bad)?.to_string();
        let named_mix = file.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("mix.wav"));
        if !file.is_file() || !named_mix || dir.parent() != Some(root.as_path()) || !plain_name(&id) {
            return Err(bad());
        }
        ctx.report(NEW_JOB_KEY, TonelabStage::Fetch, "reading the song", None);
        if !dir.join("job.json").is_file() {
            self.ops.init_job(dir, &id)?;
        }
        Ok(id)
    }

    fn lazy_separation(&self) -> bool {
        true
    }

    fn separate(&self, ctx: &Ctx, job: &str, range: Option<(f32, f32)>) -> Result<(), StepError> {
        let dir = self.job_dir(job)?;
        let (from, to) = match range {
            Some((s, e)) => (s, Some(e)),
            None => (0.0, None),
        };
        let separator = self.separator()?;
        let (mix, offset) = self.ops.read_mix(&dir, from, to)?;
        if ctx.step.is_cancelled() {
            return Err(StepError::Cancelled);
        }
        ctx.report(job, TonelabStage::Separate, "separating the stems", Some(0.0));
        let mut progress = |f: f32, msg: &str| ctx.report(job, TonelabStage::Separate, msg, Some(f.clamp(0.0, 1.0)));
        let stems = match separator.separate(&mix.left, &mix.right, mix.sample_rate, &mut progress, ctx.step.cancel_flag()) {
            Ok(s) => s,
            Err(_) if ctx.step.is_cancelled() => return Err(StepError::Cancelled),
            Err(e) => return Err(StepError::Failed(format!("{e:#}"))),
        };
        if ctx.step.is_cancelled() {
            return Err(StepError::Cancelled);
        }
        self.ops.write_stems(&dir, &stems, offset)?;
        Ok(())
    }

    fn analyse(&self, ctx: &Ctx, job: &str) -> Result<(), StepError> {
        let dir = self.job_dir(job)?;
        ctx.report(job, TonelabStage::Analyse, "analysing the song", None);
        self.ops.analyse(&dir)?;
        Ok(())
    }

    fn pick_part(&self, ctx: &Ctx, job: &str, start: f32, end: f32, channel: TonelabChannel, label: &str) -> Result<String, StepError> {
        let dir = self.job_dir(job)?;
        let covered = self.ops.stems_range(&dir).is_some_and(|(a, b)| a <= start + 1e-3 && b >= end - 1e-3);
        if !covered {
            self.separate(ctx, job, Some(((start - MARGIN_S).max(0.0), end + MARGIN_S)))?;
            if !self.ops.has_analysis(&dir) {
                // Best effort: the part works without the song map.
                let _ = self.analyse(ctx, job);
            }
        }
        if ctx.step.is_cancelled() {
            return Err(StepError::Cancelled);
        }
        ctx.report(job, TonelabStage::Parts, "extracting and transcribing the part", None);
        Ok(self.ops.pick_part(&dir, start, end, channel, label)?)
    }

    fn synth_di(&self, ctx: &Ctx, job: &str, part: &str, match_di: Option<&Path>) -> Result<(), StepError> {
        let dir = self.job_dir(job)?;
        ctx.report(job, TonelabStage::Synth, "synthesising the DI", None);
        Ok(self.ops.synth_di(&dir, part, match_di)?)
    }

    /// The app records the live input itself (AudioHost) and writes `di-user.wav`.
    fn record_di(&self, _ctx: &Ctx, job: &str, part: &str, _seconds: f32) -> Result<(), StepError> {
        let dir = self.job_dir(job)?;
        if !plain_name(part) {
            return Err(StepError::Failed("invalid part id".into()));
        }
        if dir.join("parts").join(part).join("di-user.wav").is_file() {
            Ok(())
        } else {
            Err(StepError::Failed("the app records the DI itself: write parts/<part>/di-user.wav, then send tonelab_open".into()))
        }
    }

    fn fit(&self, ctx: &Ctx, job: &str, part: &str) -> Result<openpedal_tonematch::FitSummary, StepError> {
        run_fit(ctx, &self.library, &self.jobs_dir, job, part)
    }
}

const NEW_JOB_KEY: &str = crate::tonelab::NEW_JOB;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tonelab::Step;
    use std::sync::atomic::Ordering;

    struct FakeSep;
    impl Separator for FakeSep {
        fn separate(&self, l: &[f32], r: &[f32], sr: u32, progress: &mut dyn FnMut(f32, &str), _c: &AtomicBool) -> Result<Stems> {
            progress(0.5, "half");
            Ok(Stems { sample_rate: sr, stems: vec![("guitar".into(), [l.to_vec(), r.to_vec()])] })
        }
    }

    /// Records what was asked; the "mix" is 100 s at 10 Hz.
    #[derive(Default)]
    struct FakeOps {
        log: Mutex<Vec<String>>,
        stems: Mutex<Option<(f32, f32)>>,
    }
    impl JobOps for FakeOps {
        fn init_job(&self, dir: &Path, title: &str) -> Result<()> {
            self.log.lock().unwrap().push(format!("init {title}"));
            std::fs::write(dir.join("job.json"), "{}")?;
            Ok(())
        }
        fn read_mix(&self, _d: &Path, from: f32, to: Option<f32>) -> Result<(Mix, f32)> {
            let to = to.unwrap_or(100.0).min(100.0);
            self.log.lock().unwrap().push(format!("read {from}..{to}"));
            let n = ((to - from) * 10.0) as usize;
            Ok((Mix { left: vec![0.0; n], right: vec![0.0; n], sample_rate: 10 }, from))
        }
        fn stems_range(&self, _d: &Path) -> Option<(f32, f32)> {
            *self.stems.lock().unwrap()
        }
        fn write_stems(&self, _d: &Path, s: &Stems, offset: f32) -> Result<()> {
            let n = s.stems[0].1[0].len() as f32 / s.sample_rate as f32;
            *self.stems.lock().unwrap() = Some((offset, offset + n));
            self.log.lock().unwrap().push(format!("stems {offset}+{n}"));
            Ok(())
        }
        fn has_analysis(&self, _d: &Path) -> bool {
            self.log.lock().unwrap().iter().any(|l| l == "analyse")
        }
        fn analyse(&self, _d: &Path) -> Result<()> {
            self.log.lock().unwrap().push("analyse".into());
            Ok(())
        }
        fn pick_part(&self, _d: &Path, s: f32, e: f32, _c: TonelabChannel, label: &str) -> Result<String> {
            self.log.lock().unwrap().push(format!("pick {s}..{e} {label}"));
            Ok("p1".into())
        }
        fn synth_di(&self, _d: &Path, part: &str, m: Option<&Path>) -> Result<()> {
            self.log.lock().unwrap().push(format!("synth {part} {}", m.is_some()));
            Ok(())
        }
    }

    fn backend(name: &str) -> (NativeBackend, Arc<FakeOps>, PathBuf) {
        let root = std::env::temp_dir().join(format!("openpedal-native-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let jobs = root.join("jobs");
        std::fs::create_dir_all(jobs.join("song1")).unwrap();
        std::fs::write(jobs.join("song1/mix.wav"), b"RIFF").unwrap();
        let ops = Arc::new(FakeOps::default());
        let b = NativeBackend::new(jobs.clone(), Library::new(root.join("lib")), ops.clone(), Box::new(|| Ok(Arc::new(FakeSep) as Arc<dyn Separator>)));
        (b, ops, root)
    }

    fn with_ctx<T>(f: impl FnOnce(&Ctx) -> T) -> (T, Vec<(TonelabStage, Option<f32>)>) {
        let seen = Mutex::new(Vec::new());
        let sink = |_: &str, s: TonelabStage, _: String, f: Option<f32>| seen.lock().unwrap().push((s, f));
        let step = Step::new_for_tests();
        let out = f(&Ctx { step: &step, progress: &sink });
        (out, seen.into_inner().unwrap())
    }

    #[test]
    fn caps_say_no_youtube_and_cpu() {
        let (b, _, root) = backend("caps");
        let c = b.caps().unwrap();
        assert!(!c.youtube);
        assert_eq!(c.device, "cpu");
        assert!(b.lazy_separation());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn start_registers_the_folder_and_pick_separates_the_excerpt() {
        let (b, ops, root) = backend("flow");
        let mix = root.join("jobs/song1/mix.wav");
        let (id, _) = with_ctx(|c| b.fetch(c, mix.to_str().unwrap()));
        assert_eq!(id.unwrap(), "song1");
        assert_eq!(ops.log.lock().unwrap().as_slice(), ["init song1"]);
        // No stems yet: the pick separates 8-2 .. 28+2, analyses, cuts the part.
        let (part, seen) = with_ctx(|c| b.pick_part(c, "song1", 8.0, 28.0, TonelabChannel::Left, "riff"));
        assert_eq!(part.unwrap(), "p1");
        let log = ops.log.lock().unwrap().clone();
        assert_eq!(log[1..], ["read 6..30", "stems 6+24", "analyse", "pick 8..28 riff"]);
        assert!(seen.iter().any(|(s, f)| *s == TonelabStage::Separate && *f == Some(0.5)));
        drop(log);
        // A second part inside the same excerpt reuses the stems.
        let _ = with_ctx(|c| b.pick_part(c, "song1", 10.0, 20.0, TonelabChannel::Mid, ""));
        assert_eq!(ops.log.lock().unwrap().iter().filter(|l| l.starts_with("read")).count(), 1);
        // Outside it: separated again.
        let _ = with_ctx(|c| b.pick_part(c, "song1", 60.0, 70.0, TonelabChannel::Mid, ""));
        assert_eq!(ops.log.lock().unwrap().iter().filter(|l| l.starts_with("read")).count(), 2);
        let (r, _) = with_ctx(|c| b.synth_di(c, "song1", "p1", None));
        assert!(r.is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fetch_rejects_files_outside_the_jobs_dir_and_odd_names() {
        let (b, _, root) = backend("reject");
        std::fs::write(root.join("mix.wav"), b"RIFF").unwrap();
        std::fs::write(root.join("jobs/song1/other.wav"), b"RIFF").unwrap();
        std::fs::create_dir_all(root.join("jobs/a/b")).unwrap();
        std::fs::write(root.join("jobs/a/b/mix.wav"), b"RIFF").unwrap();
        for bad in [root.join("mix.wav"), root.join("jobs/song1/other.wav"), root.join("jobs/a/b/mix.wav"), root.join("jobs/song1/../song1/nope.wav"), PathBuf::from("/nonexistent/mix.wav")] {
            let (r, _) = with_ctx(|c| b.fetch(c, bad.to_str().unwrap()));
            assert!(matches!(r, Err(StepError::Failed(_))), "{}", bad.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancel_stops_before_separating() {
        let (b, ops, root) = backend("cancel");
        std::fs::write(root.join("jobs/song1/job.json"), "{}").unwrap();
        let step = Step::new_for_tests();
        step.cancel();
        let sink = |_: &str, _: TonelabStage, _: String, _: Option<f32>| {};
        let r = b.pick_part(&Ctx { step: &step, progress: &sink }, "song1", 8.0, 28.0, TonelabChannel::Mid, "");
        assert!(matches!(r, Err(StepError::Cancelled)));
        assert!(ops.log.lock().unwrap().iter().all(|l| !l.starts_with("pick")));
        assert!(!step.cancel_flag().load(Ordering::SeqCst) == false);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn record_di_needs_the_file_the_app_writes() {
        let (b, _, root) = backend("rec");
        std::fs::write(root.join("jobs/song1/job.json"), "{}").unwrap();
        let (r, _) = with_ctx(|c| b.record_di(c, "song1", "p1", 5.0));
        assert!(r.is_err());
        std::fs::create_dir_all(root.join("jobs/song1/parts/p1")).unwrap();
        std::fs::write(root.join("jobs/song1/parts/p1/di-user.wav"), b"x").unwrap();
        let (r, _) = with_ctx(|c| b.record_di(c, "song1", "p1", 5.0));
        assert!(r.is_ok());
        let _ = std::fs::remove_dir_all(root);
    }
}
