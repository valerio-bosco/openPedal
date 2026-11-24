//! The engine behind Tone Lab (docs/PLAN.md §15): the handlers in `tonelab.rs` only talk to a
//! `TonelabBackend`. `PythonBackend` runs the `tonelab` tool as subprocesses (desktop, the
//! default); `NativeBackend` (native_backend.rs) runs everything in process (iOS).

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde_json::Value;

use openpedal_library::Library;
use openpedal_protocol::{TonelabChannel, TonelabStage};

use crate::tonelab::{last_message, parse_progress, plain_name, read_json, Step, StepError, ToolCaps, NEW_JOB};

/// Chain templates, relative to the repository root.
pub const TEMPLATES_DIR: &str = "content/tonematch/templates";

/// Where a backend sends progress: `(job key, stage, message, fraction)`.
pub type ProgressSink<'a> = dyn Fn(&str, TonelabStage, String, Option<f32>) + Sync + 'a;

/// A running step as the backend sees it: the cancel flag and where progress goes.
pub struct Ctx<'a> {
    pub step: &'a Step,
    /// `(job key, stage, message, fraction)`; the key is `NEW_JOB` while a song is being fetched.
    pub progress: &'a ProgressSink<'a>,
}

impl Ctx<'_> {
    pub fn report(&self, job: &str, stage: TonelabStage, message: impl Into<String>, fraction: Option<f32>) {
        (self.progress)(job, stage, message.into(), fraction);
    }
}

/// What the handlers need from an engine. Every method blocks (the handlers call them from a
/// background thread) and must stop early with `StepError::Cancelled` when `ctx.step` is cancelled.
pub trait TonelabBackend: Send + Sync {
    /// `"python"` or `"native"`.
    fn name(&self) -> &'static str;
    /// What it can do here, or why it cannot.
    fn caps(&self) -> Result<ToolCaps, String>;
    /// Registers the song as a job (a URL or an uploaded file for Python, `<job dir>/mix.wav` for
    /// native) and returns the job id.
    fn fetch(&self, ctx: &Ctx, source: &str) -> Result<String, StepError>;
    /// `true`: `tonelab_start` stops after `fetch` and the stems are made at pick time (iOS).
    fn lazy_separation(&self) -> bool;
    /// Separates the whole song (`None`) or the excerpt `start..end` in seconds.
    fn separate(&self, ctx: &Ctx, job: &str, range: Option<(f32, f32)>) -> Result<(), StepError>;
    /// Sections, tempo, key: writes `analysis.json`.
    fn analyse(&self, ctx: &Ctx, job: &str) -> Result<(), StepError>;
    /// Cuts and transcribes a part; separates the excerpt first when there are no stems. Returns the part id.
    fn pick_part(&self, ctx: &Ctx, job: &str, start: f32, end: f32, channel: TonelabChannel, label: &str) -> Result<String, StepError>;
    /// Writes the synthetic DI of a part, matched to `match_di` when given.
    fn synth_di(&self, ctx: &Ctx, job: &str, part: &str, match_di: Option<&Path>) -> Result<(), StepError>;
    /// The `tonelab_record_di` hook: `Ok` = `parts/<part>/di-user.wav` exists now.
    fn record_di(&self, ctx: &Ctx, job: &str, part: &str, seconds: f32) -> Result<(), StepError>;
    /// Fits the templates to the part and writes `parts/<part>/fit/`.
    fn fit(&self, ctx: &Ctx, job: &str, part: &str) -> Result<openpedal_tonematch::FitSummary, StepError>;
}

/// The fit, in process on the calling thread (shared by both backends). Cancelling unwinds out of it.
pub fn run_fit(ctx: &Ctx, library: &Library, jobs_dir: &Path, job: &str, part: &str) -> Result<openpedal_tonematch::FitSummary, StepError> {
    let opts = openpedal_tonematch::FitOptions { templates_dir: TEMPLATES_DIR.into(), pedals_dir: library.pedals_dir(), ..Default::default() };
    let n_templates = openpedal_tonematch::Template::list(&opts.templates_dir).len().max(1);
    ctx.report(job, TonelabStage::Fit, "starting the fit", Some(0.0));
    openpedal_tonematch::lower_priority();
    // (templates screened, finalists fitted): the progress lines say which.
    let counts = Mutex::new((0usize, 0usize));
    let progress = |line: &str| {
        if ctx.step.is_cancelled() {
            // Unwinds out of the fit (the panic hook is not called); caught below.
            std::panic::resume_unwind(Box::new("fit cancelled"));
        }
        let fraction = {
            let mut c = counts.lock().unwrap_or_else(|e| e.into_inner());
            if line.starts_with("  screen") {
                c.0 += 1;
            } else if line.starts_with("  fit") {
                c.1 += 1;
            }
            (0.5 * (c.0 as f32 / n_templates as f32).min(1.0) + 0.45 * (c.1 as f32 / 2.0).min(1.0)).min(0.95)
        };
        ctx.report(job, TonelabStage::Fit, line.trim(), Some(fraction));
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| openpedal_tonematch::fit_job(&jobs_dir.join(job), part, &opts, &progress)));
    match run {
        Err(_) if ctx.step.is_cancelled() => Err(StepError::Cancelled),
        Err(_) => Err(StepError::Failed("the fit crashed".into())),
        Ok(Err(e)) => Err(StepError::Failed(format!("{e:#}"))),
        Ok(Ok(summary)) => Ok(summary),
    }
}

/// Runs the Python `tonelab` tool.
pub struct PythonBackend {
    tool: PathBuf,
    library: Library,
    caps: Mutex<Option<ToolCaps>>,
}

impl PythonBackend {
    pub fn new(tool: PathBuf, library: Library) -> Self {
        Self { tool, library, caps: Mutex::new(None) }
    }

    fn jobs_dir(&self) -> Result<PathBuf, StepError> {
        self.caps().map(|c| c.jobs_dir).map_err(|r| StepError::Failed(format!("Tone Lab is not available: {r}")))
    }

    /// Runs the tool with `args`, forwarding `PROGRESS` lines to `on_progress`. Returns its stdout.
    fn run_tool(&self, step: &Step, args: &[String], mut on_progress: impl FnMut(crate::tonelab::ProgressLine)) -> Result<String, StepError> {
        let tool = &self.tool;
        if step.is_cancelled() {
            return Err(StepError::Cancelled);
        }
        let mut child = Command::new(tool)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| StepError::Failed(format!("cannot run {}: {e}", tool.display())))?;
        let mut stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let child = Arc::new(Mutex::new(child));
        step.set_child(Some(child.clone()));
        if step.is_cancelled() {
            step.cancel();
        }
        let out_thread = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            s
        });
        let mut tail: Vec<String> = Vec::new();
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        while reader.read_until(b'\n', &mut buf).map(|n| n > 0).unwrap_or(false) {
            let line = String::from_utf8_lossy(&buf).trim_end().to_string();
            buf.clear();
            match parse_progress(&line) {
                Some(p) => on_progress(p),
                None => {
                    tail.push(line);
                    if tail.len() > 20 {
                        tail.remove(0);
                    }
                }
            }
        }
        let status = child.lock().unwrap_or_else(|e| e.into_inner()).wait();
        step.set_child(None);
        let stdout = out_thread.join().unwrap_or_default();
        if step.is_cancelled() {
            return Err(StepError::Cancelled);
        }
        match status {
            Ok(s) if s.success() => Ok(stdout),
            Ok(s) => Err(StepError::Failed(format!("{} ({s})", last_message(&tail)))),
            Err(e) => Err(StepError::Failed(format!("cannot wait for the tool: {e}"))),
        }
    }

    fn run_step(&self, ctx: &Ctx, key: &str, args: &[String]) -> Result<String, StepError> {
        self.run_tool(ctx.step, args, |p| ctx.report(key, p.stage, p.message, p.fraction))
    }
}

impl TonelabBackend for PythonBackend {
    fn name(&self) -> &'static str {
        "python"
    }

    /// Runs `tonelab caps --json` the first time.
    fn caps(&self) -> Result<ToolCaps, String> {
        if let Some(c) = self.caps.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(c);
        }
        let tool = &self.tool;
        let out = Command::new(tool).args(["caps", "--json"]).stdin(Stdio::null()).output().map_err(|e| format!("cannot run {}: {e}", tool.display()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("`tonelab caps` failed: {}", last_message(&err.lines().map(str::to_string).collect::<Vec<_>>())));
        }
        let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("`tonelab caps --json` printed something else: {e}"))?;
        let caps = ToolCaps {
            youtube: v["youtube"].as_bool().unwrap_or(false),
            device: v["device"].as_str().unwrap_or("cuda").to_string(),
            jobs_dir: PathBuf::from(v["jobs_dir"].as_str().ok_or("`tonelab caps --json` has no jobs_dir")?),
        };
        *self.caps.lock().unwrap_or_else(|e| e.into_inner()) = Some(caps.clone());
        Ok(caps)
    }

    fn fetch(&self, ctx: &Ctx, source: &str) -> Result<String, StepError> {
        let jobs_dir = self.jobs_dir()?;
        if !(source.starts_with("http://") || source.starts_with("https://")) {
            let uploads = jobs_dir.join("uploads");
            let uploads = uploads.canonicalize().unwrap_or(uploads);
            let file = Path::new(source).canonicalize().map_err(|_| StepError::Failed(format!("'{source}' is neither a URL nor an uploaded file")))?;
            if !file.starts_with(&uploads) || !file.is_file() {
                return Err(StepError::Failed("the source must be a URL or a file sent with POST /tonelab/upload".into()));
            }
        }
        let out = self.run_step(ctx, NEW_JOB, &["fetch".into(), source.to_string()])?;
        let id = out.lines().last().unwrap_or("").trim().to_string();
        if !plain_name(&id) {
            return Err(StepError::Failed(format!("the tool returned an odd job id: '{id}'")));
        }
        Ok(id)
    }

    fn lazy_separation(&self) -> bool {
        false
    }

    fn separate(&self, ctx: &Ctx, job: &str, range: Option<(f32, f32)>) -> Result<(), StepError> {
        let mut args: Vec<String> = vec!["separate".into(), job.into()];
        if let Some((s, e)) = range {
            args.extend(["--start".into(), s.to_string(), "--end".into(), e.to_string()]);
        }
        self.run_step(ctx, job, &args).map(|_| ())
    }

    fn analyse(&self, ctx: &Ctx, job: &str) -> Result<(), StepError> {
        self.run_step(ctx, job, &["analyse".into(), job.into()]).map(|_| ())
    }

    fn pick_part(&self, ctx: &Ctx, job: &str, start: f32, end: f32, channel: TonelabChannel, label: &str) -> Result<String, StepError> {
        let parts_file = self.jobs_dir()?.join(job).join("parts.json");
        let before = read_json(&parts_file).ok().and_then(|v| v.as_array().map(|a| a.len())).unwrap_or(0);
        let ch = match channel {
            TonelabChannel::Mid => "mid",
            TonelabChannel::Left => "left",
            TonelabChannel::Right => "right",
        };
        let mut args: Vec<String> = vec!["parts".into(), job.into(), "--start".into(), start.to_string(), "--end".into(), end.to_string(), "--channel".into(), ch.into()];
        if !label.trim().is_empty() {
            args.extend(["--label".into(), label.trim().to_string()]);
        }
        self.run_step(ctx, job, &args)?;
        let parts = read_json(&parts_file)?;
        parts
            .as_array()
            .filter(|a| a.len() > before)
            .and_then(|a| a.last())
            .and_then(|p| p["id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| StepError::Failed("the tool did not add a part".into()))
    }

    fn synth_di(&self, ctx: &Ctx, job: &str, part: &str, match_di: Option<&Path>) -> Result<(), StepError> {
        let mut args: Vec<String> = vec!["synth-di".into(), job.into(), "--part".into(), part.into()];
        if let Some(di) = match_di {
            args.extend(["--match-di".into(), di.to_string_lossy().into_owned()]);
        }
        self.run_step(ctx, job, &args).map(|_| ())
    }

    fn record_di(&self, _ctx: &Ctx, _job: &str, _part: &str, _seconds: f32) -> Result<(), StepError> {
        Err(StepError::Failed(
            "recording the DI from the live input is not available yet: record it with `openpedal --record` and put the dry channel into parts/<part>/di-user.wav".into(),
        ))
    }

    fn fit(&self, ctx: &Ctx, job: &str, part: &str) -> Result<openpedal_tonematch::FitSummary, StepError> {
        run_fit(ctx, &self.library, &self.jobs_dir()?, job, part)
    }
}
