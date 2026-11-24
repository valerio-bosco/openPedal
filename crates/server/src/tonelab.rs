//! Tone Lab (docs/PLAN.md §14.5): runs the `tonelab` Python tool as a subprocess, the fit in
//! process, and serves the job's WAV files over HTTP.
//!
//! Every `tonelab_*` message except `tonelab_save_preset` is answered from a background thread:
//! progress goes to the client that asked, results to every client. One step at a time per job.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tokio::sync::broadcast;

use openpedal_library::Library;
use openpedal_protocol::{
    ClientMsg, ServerMsg, TonelabChannel, TonelabFit, TonelabJobInfo, TonelabJobSummary, TonelabPart, TonelabSection, TonelabStage,
    TonelabTemplateResult,
};

use crate::app::{AppState, Broadcast};
use crate::tonelab_backend::{Ctx, PythonBackend, TonelabBackend};
use crate::tonelab_mock::Demo;

/// Where the tool lives, relative to the repository root the server runs from.
const REPO_TOOL: &str = "tools/tonelab/bin/tonelab";
/// Largest upload accepted (a long song as WAV).
const MAX_UPLOAD: usize = 1 << 30;
/// Largest piece of a file sent in one response (a `Range` request without an end gets this much).
const MAX_RANGE: u64 = 16 << 20;
/// Progress key of a song still being fetched: its job id does not exist yet.
pub const NEW_JOB: &str = "new";

/// What `tonelab caps --json` reports.
#[derive(Debug, Clone)]
pub struct ToolCaps {
    pub youtube: bool,
    pub device: String,
    pub jobs_dir: PathBuf,
}

enum Mode {
    /// A backend: the Python tool or the native one.
    Backend(Arc<dyn TonelabBackend>),
    /// Fixtures instead of Python (no tool and no audio: the interface's tests).
    Demo,
    /// Not available, with the reason.
    Off(String),
}

/// A running step of a job: lets `tonelab_cancel` stop it.
pub(crate) struct Step {
    cancelled: AtomicBool,
    pub(crate) child: Mutex<Option<Arc<Mutex<Child>>>>,
}

impl Step {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Step { cancelled: AtomicBool::new(false), child: Mutex::new(None) })
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests() -> Arc<Self> {
        Self::new()
    }

    pub(crate) fn set_child(&self, child: Option<Arc<Mutex<Child>>>) {
        *self.child.lock().unwrap_or_else(|e| e.into_inner()) = child;
    }

    pub(crate) fn cancel_flag(&self) -> &AtomicBool {
        &self.cancelled
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
pub(crate) enum StepError {
    Cancelled,
    Failed(String),
}

impl From<anyhow::Error> for StepError {
    fn from(e: anyhow::Error) -> Self {
        StepError::Failed(format!("{e:#}"))
    }
}

/// What the host chose for the native backend (set by the iOS FFI, or by `OPENPEDAL_TONELAB=native` on the desktop).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeConfig {
    pub jobs_dir: PathBuf,
    /// The Demucs weights file.
    pub weights: Option<PathBuf>,
    /// Chain templates for the fit; `None` = `content/tonematch/templates` (the repository's, desktop).
    /// The iPad passes the copy bundled in the app's resources.
    pub templates_dir: Option<PathBuf>,
}

static NATIVE_CONFIG: Mutex<Option<NativeConfig>> = Mutex::new(None);

/// Makes every server started afterwards use the native backend (called by the iOS FFI start).
pub fn set_native_tonelab(config: Option<NativeConfig>) {
    *NATIVE_CONFIG.lock().unwrap_or_else(|e| e.into_inner()) = config;
}

pub(crate) fn native_config() -> Option<NativeConfig> {
    NATIVE_CONFIG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub(crate) enum Choice {
    Backend(Arc<dyn TonelabBackend>),
    Demo,
    Off(String),
}

/// The native setup for `OPENPEDAL_TONELAB=native` on the desktop: weights from
/// `$OPENPEDAL_DEMUCS_WEIGHTS`, jobs in `$OPENPEDAL_TONELAB_JOBS` or the library's `tonelab/jobs`.
fn desktop_native_config(library: &Library) -> NativeConfig {
    let jobs_dir = std::env::var_os("OPENPEDAL_TONELAB_JOBS").map(PathBuf::from).unwrap_or_else(|| library.root().join("tonelab/jobs"));
    NativeConfig { jobs_dir, weights: std::env::var_os("OPENPEDAL_DEMUCS_WEIGHTS").map(PathBuf::from), templates_dir: None }
}

#[cfg(feature = "native-tonelab")]
fn native_backend(cfg: NativeConfig, library: &Library) -> Choice {
    let Some(weights) = cfg.weights.clone().filter(|w| w.is_file()) else {
        return Choice::Off(format!("the separation weights were not found ({}): set OPENPEDAL_DEMUCS_WEIGHTS", cfg.weights.as_ref().map_or("not set".into(), |w| w.display().to_string())));
    };
    let loader = Box::new(move || crate::native_adapter::load_separator(&weights));
    Choice::Backend(Arc::new(crate::native_backend::NativeBackend::new(cfg.jobs_dir, library.clone(), Arc::new(crate::native_adapter::TonelabJobOps), loader)))
}

#[cfg(not(feature = "native-tonelab"))]
fn native_backend(_cfg: NativeConfig, _library: &Library) -> Choice {
    Choice::Off("this build has no native Tone Lab (cargo feature native-tonelab)".into())
}

/// Backend selection: the host's native setup wins (iOS); then `OPENPEDAL_TONELAB`: `native`,
/// `mock`, `none` or a path to the Python tool; then the tool in the repository; then fixtures
/// when there is no audio engine; otherwise off.
pub(crate) fn choose_backend(tool_env: Option<&str>, repo_tool: &Path, audio: bool, native: Option<NativeConfig>, library: &Library) -> Choice {
    if let Some(cfg) = native {
        return native_backend(cfg, library);
    }
    let python = |p: PathBuf| Choice::Backend(Arc::new(PythonBackend::new(p, library.clone())));
    match tool_env {
        Some("native") => native_backend(desktop_native_config(library), library),
        Some("mock") => Choice::Demo,
        Some("none") => Choice::Off("disabled by OPENPEDAL_TONELAB=none".into()),
        Some(path) if !path.is_empty() => {
            let p = PathBuf::from(path);
            if p.is_file() {
                python(p)
            } else {
                Choice::Off(format!("OPENPEDAL_TONELAB points to {path}, which does not exist"))
            }
        }
        _ if repo_tool.is_file() => python(repo_tool.to_path_buf()),
        _ if !audio => Choice::Demo,
        _ => Choice::Off(format!("the tonelab tool was not found at {REPO_TOOL} (run openpedal from the repository root, or set OPENPEDAL_TONELAB)")),
    }
}

pub struct Tonelab {
    tx: broadcast::Sender<Broadcast>,
    library: Library,
    mode: Mode,
    steps: Mutex<HashMap<String, Arc<Step>>>,
    pub(crate) demo: Demo,
}

/// A line `PROGRESS <stage> <fraction|-> <message>` of the tool's stderr.
#[derive(Debug, PartialEq)]
pub struct ProgressLine {
    pub stage: TonelabStage,
    pub fraction: Option<f32>,
    pub message: String,
}

pub fn parse_progress(line: &str) -> Option<ProgressLine> {
    let mut it = line.trim_end().splitn(4, ' ');
    if it.next()? != "PROGRESS" {
        return None;
    }
    let stage = match it.next()? {
        "fetch" => TonelabStage::Fetch,
        "separate" => TonelabStage::Separate,
        "analyse" => TonelabStage::Analyse,
        "parts" => TonelabStage::Parts,
        "synth" | "synth-di" => TonelabStage::Synth,
        _ => return None,
    };
    let fraction = match it.next()? {
        "-" => None,
        f => Some(f.parse::<f32>().ok()?.clamp(0.0, 1.0)),
    };
    Some(ProgressLine { stage, fraction, message: it.next().unwrap_or("").to_string() })
}

/// One path component, no separators, not hidden: safe to join under the jobs directory.
pub(crate) fn plain_name(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && s.chars().all(|c| c != '/' && c != '\\' && c != '\0')
}

/// A relative path inside a job: normal components only (no `..`, no absolute paths), a `.wav` file.
pub fn safe_job_path(rest: &str) -> Option<PathBuf> {
    if rest.is_empty() || rest.contains('\\') || rest.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(rest).components() {
        match c {
            Component::Normal(n) => out.push(n),
            _ => return None,
        }
    }
    (out.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav"))).then_some(out)
}

fn f32_of(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(Value::as_f64).map(|x| x as f32)
}

pub(crate) fn read_json(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("error in {}", path.display()))
}

fn channel_of(s: &str) -> TonelabChannel {
    match s {
        "left" => TonelabChannel::Left,
        "right" => TonelabChannel::Right,
        _ => TonelabChannel::Mid,
    }
}

/// The fit of a part, from `parts/<id>/fit/report.json`.
fn read_fit(part_dir: &Path) -> Option<TonelabFit> {
    let r = read_json(&part_dir.join("fit/report.json")).ok()?;
    Some(TonelabFit {
        template: r.get("template")?.as_str()?.to_string(),
        error_db: f32_of(&r, "error_db")?,
        holdout_error_db: f32_of(&r, "holdout_error_db"),
        validate_error_db: f32_of(&r, "validate_error_db"),
    })
}

/// Reads a job folder into what the interface shows.
pub fn read_job(jobs_dir: &Path, id: &str) -> Result<TonelabJobInfo> {
    if !plain_name(id) {
        bail!("invalid job id '{id}'");
    }
    let dir = jobs_dir.join(id);
    let job = read_json(&dir.join("job.json")).with_context(|| format!("job '{id}' not found"))?;
    let analysis = read_json(&dir.join("analysis.json")).unwrap_or(Value::Null);
    let parts_json = read_json(&dir.join("parts.json")).unwrap_or(Value::Null);
    let sections = analysis["sections"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| TonelabSection {
                    start: f32_of(s, "start").unwrap_or(0.0),
                    end: f32_of(s, "end").unwrap_or(0.0),
                    kind: s["kind"].as_str().unwrap_or("none").to_string(),
                    guitar_db: f32_of(s, "guitar_db").unwrap_or(-90.0),
                    corr: f32_of(s, "corr").unwrap_or(0.0),
                })
                .collect()
        })
        .unwrap_or_default();
    let parts = parts_json
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let pid = p["id"].as_str()?.to_string();
                    let pd = dir.join("parts").join(&pid);
                    Some(TonelabPart {
                        label: p["label"].as_str().unwrap_or(&pid).to_string(),
                        start: f32_of(p, "start").unwrap_or(0.0),
                        end: f32_of(p, "end").unwrap_or(0.0),
                        channel: channel_of(p["channel"].as_str().unwrap_or("mid")),
                        notes: p["notes"].as_array().map_or(0, |n| n.len() as u32),
                        has_user_di: pd.join("di-user.wav").exists(),
                        has_synth_di: pd.join("di-synth.wav").exists(),
                        fit: read_fit(&pd),
                        id: pid,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(TonelabJobInfo {
        id: job["id"].as_str().unwrap_or(id).to_string(),
        title: job["source"]["title"].as_str().unwrap_or(id).to_string(),
        source_kind: job["source"]["kind"].as_str().unwrap_or("file").to_string(),
        duration_s: f32_of(&job, "duration_s").unwrap_or(0.0),
        tempo_bpm: f32_of(&analysis, "tempo_bpm"),
        key: analysis["key"].as_str().map(str::to_string),
        sections,
        parts,
    })
}

/// Every job folder, newest first.
pub fn list_jobs(jobs_dir: &Path) -> Vec<TonelabJobSummary> {
    let mut jobs: Vec<TonelabJobSummary> = std::fs::read_dir(jobs_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let job = read_json(&e.path().join("job.json")).ok()?;
            Some(TonelabJobSummary {
                id: job["id"].as_str().map(str::to_string).unwrap_or_else(|| e.file_name().to_string_lossy().into_owned()),
                title: job["source"]["title"].as_str().unwrap_or("").to_string(),
                created: job["created"].as_str().unwrap_or("").to_string(),
                duration_s: f32_of(&job, "duration_s").unwrap_or(0.0),
            })
        })
        .collect();
    jobs.sort_by(|a, b| b.created.cmp(&a.created).then(a.id.cmp(&b.id)));
    jobs
}

/// The newest `di-user.wav` of any job: the real DI the synthetic one is matched to.
fn newest_user_di(jobs_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for job in std::fs::read_dir(jobs_dir).ok()?.flatten() {
        for part in std::fs::read_dir(job.path().join("parts")).into_iter().flatten().flatten() {
            let wav = part.path().join("di-user.wav");
            if let Ok(t) = std::fs::metadata(&wav).and_then(|m| m.modified()) {
                if best.as_ref().is_none_or(|(b, _)| t > *b) {
                    best = Some((t, wav));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

pub(crate) fn last_message(stderr: &[String]) -> String {
    stderr.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_else(|| "the tool failed without a message".into())
}

impl Tonelab {
    /// `tool_env` is `$OPENPEDAL_TONELAB`: a path to the tool, `mock` (fixtures), `none` (off) or
    /// `native` (in process). `native` = the native setup when it is chosen by the host (iOS).
    /// `audio` = a real audio engine is running (without it and without the tool, fixtures answer).
    pub fn new(tx: broadcast::Sender<Broadcast>, library: Library, audio: bool, tool_env: Option<String>, repo_tool: PathBuf, native: Option<NativeConfig>) -> Self {
        let mode = match choose_backend(tool_env.as_deref(), &repo_tool, audio, native, &library) {
            Choice::Backend(b) => Mode::Backend(b),
            Choice::Demo => Mode::Demo,
            Choice::Off(r) => Mode::Off(r),
        };
        Self { tx, library, mode, steps: Mutex::new(HashMap::new()), demo: Demo::new() }
    }

    /// With a ready-made backend (tests).
    #[allow(dead_code)]
    pub fn with_backend(tx: broadcast::Sender<Broadcast>, library: Library, backend: Arc<dyn TonelabBackend>) -> Self {
        Self { tx, library, mode: Mode::Backend(backend), steps: Mutex::new(HashMap::new()), demo: Demo::new() }
    }

    #[allow(dead_code)]
    /// Which backend answers: `"python"`, `"native"`, or `None` (demo or off).
    pub fn backend_name(&self) -> Option<&'static str> {
        match &self.mode {
            Mode::Backend(b) => Some(b.name()),
            _ => None,
        }
    }

    pub(crate) fn is_demo(&self) -> bool {
        matches!(self.mode, Mode::Demo)
    }

    pub(crate) fn to_client(&self, from: u64, msg: ServerMsg) {
        let _ = self.tx.send(Broadcast { except: None, only: Some(from), msg });
    }

    pub(crate) fn to_all(&self, msg: ServerMsg) {
        let _ = self.tx.send(Broadcast { except: None, only: None, msg });
    }

    pub(crate) fn progress_to(&self, from: u64, job: &str, stage: TonelabStage, message: impl Into<String>, fraction: Option<f32>) {
        self.to_client(from, ServerMsg::TonelabProgress { job: job.into(), stage, message: message.into(), fraction });
    }

    pub(crate) fn fail(&self, from: u64, message: String) {
        self.to_client(from, ServerMsg::Error { message, node: None });
    }

    /// What the backend can do here, or why it cannot.
    pub fn caps(&self) -> Result<ToolCaps, String> {
        match &self.mode {
            Mode::Off(reason) => Err(reason.clone()),
            Mode::Demo => Ok(ToolCaps { youtube: true, device: "cpu".into(), jobs_dir: crate::tonelab_mock::demo_dir() }),
            Mode::Backend(b) => b.caps(),
        }
    }

    pub(crate) fn jobs_dir(&self) -> Result<PathBuf> {
        self.caps().map(|c| c.jobs_dir).map_err(|r| anyhow!("Tone Lab is not available: {r}"))
    }

    /// Registers a running step under `key`; one at a time per job.
    pub(crate) fn begin(&self, key: &str) -> Result<Arc<Step>> {
        let mut steps = self.steps.lock().unwrap_or_else(|e| e.into_inner());
        if steps.contains_key(key) {
            if key == NEW_JOB {
                bail!("another song is being imported: wait for it or cancel it");
            }
            bail!("job '{key}' is busy with another step: wait for it or cancel it");
        }
        let step = Step::new();
        steps.insert(key.to_string(), step.clone());
        Ok(step)
    }

    pub(crate) fn end(&self, key: &str) {
        self.steps.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
    }

    /// The running step of a job, if any.
    pub(crate) fn step_of(&self, key: &str) -> Option<Arc<Step>> {
        self.steps.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned()
    }

    /// Moves a step to the real job id once the fetch created the job.
    fn rekey(&self, from: &str, to: &str) -> Result<()> {
        let mut steps = self.steps.lock().unwrap_or_else(|e| e.into_inner());
        if steps.contains_key(to) {
            bail!("job '{to}' is busy with another step");
        }
        if let Some(s) = steps.remove(from) {
            steps.insert(to.to_string(), s);
        }
        Ok(())
    }

    /// Ends a step: frees the job, then reports a cancellation (to everyone) or a failure (to the asker).
    pub(crate) fn finish(&self, from: u64, job: &str, key: &str, what: &str, result: Result<(), StepError>) {
        self.end(key);
        match result {
            Ok(()) => {}
            Err(StepError::Cancelled) => self.to_all(ServerMsg::TonelabProgress { job: job.into(), stage: TonelabStage::Cancelled, message: format!("{what} cancelled"), fraction: None }),
            Err(StepError::Failed(m)) => self.fail(from, format!("{what} failed: {m}")),
        }
    }

    fn spawn(self: &Arc<Self>, f: impl FnOnce(&Arc<Tonelab>) + Send + 'static) {
        let t = self.clone();
        std::thread::spawn(move || f(&t));
    }

    /// Handles a `tonelab_*` message other than `tonelab_save_preset`; replies go out through the
    /// broadcast channel (progress to the asker, results to everyone).
    pub fn handle(self: &Arc<Self>, from: u64, msg: ClientMsg) {
        if self.is_demo() {
            return crate::tonelab_mock::handle(self, from, msg);
        }
        match msg {
            ClientMsg::TonelabCaps => self.spawn(move |t| {
                let reply = match t.caps() {
                    Ok(c) => ServerMsg::TonelabCaps { available: true, youtube: c.youtube, device: c.device, reason: None },
                    Err(reason) => ServerMsg::TonelabCaps { available: false, youtube: false, device: String::new(), reason: Some(reason) },
                };
                t.to_client(from, reply);
            }),
            ClientMsg::TonelabJobs => self.spawn(move |t| match t.jobs_dir() {
                Ok(dir) => t.to_client(from, ServerMsg::TonelabJobs { jobs: list_jobs(&dir) }),
                Err(e) => t.fail(from, format!("{e:#}")),
            }),
            ClientMsg::TonelabOpen { job } => self.spawn(move |t| match t.jobs_dir().and_then(|d| read_job(&d, &job)) {
                Ok(info) => t.to_all(ServerMsg::TonelabJob { job: info }),
                Err(e) => t.fail(from, format!("{e:#}")),
            }),
            ClientMsg::TonelabStart { source } => self.spawn(move |t| t.start(from, source)),
            ClientMsg::TonelabPick { job, start, end, channel, label } => self.spawn(move |t| t.pick(from, job, start, end, channel, label)),
            ClientMsg::TonelabRecordDi { job, part, seconds } => self.spawn(move |t| t.record_di(from, job, part, seconds)),
            ClientMsg::TonelabFit { job, part } => self.spawn(move |t| t.fit(from, job, part)),
            ClientMsg::TonelabCancel { job } => match self.step_of(&job) {
                Some(s) => s.cancel(),
                None => self.fail(from, format!("nothing is running for job '{job}'")),
            },
            _ => {}
        }
    }

    /// Runs `f` as a step with a progress sink that sends to client `from`.
    fn with_ctx<T>(&self, from: u64, step: &Step, f: impl FnOnce(&Ctx) -> T) -> T {
        let sink = |job: &str, stage: TonelabStage, message: String, fraction: Option<f32>| self.progress_to(from, job, stage, message, fraction);
        f(&Ctx { step, progress: &sink })
    }

    fn backend(&self) -> Result<Arc<dyn TonelabBackend>, StepError> {
        match &self.mode {
            Mode::Backend(b) => Ok(b.clone()),
            _ => Err(StepError::Failed("Tone Lab is not available".into())),
        }
    }

    fn send_job(&self, from: u64, id: &str) {
        match self.jobs_dir().and_then(|d| read_job(&d, id)) {
            Ok(info) => self.to_all(ServerMsg::TonelabJob { job: info }),
            Err(e) => self.fail(from, format!("{e:#}")),
        }
    }

    /// `fetch` (a URL, an uploaded file or a native job's mix.wav), then `separate` and `analyse`
    /// unless the backend separates lazily.
    fn start(&self, from: u64, source: String) {
        let step = match self.begin(NEW_JOB) {
            Ok(s) => s,
            Err(e) => return self.fail(from, format!("{e:#}")),
        };
        // The job id once known (the step is registered under it from then on).
        let mut id = NEW_JOB.to_string();
        let result = (|| -> Result<(), StepError> {
            let backend = self.backend()?;
            self.with_ctx(from, &step, |ctx| -> Result<(), StepError> {
                let new_id = backend.fetch(ctx, &source)?;
                self.rekey(NEW_JOB, &new_id)?;
                id = new_id;
                if !backend.lazy_separation() {
                    backend.separate(ctx, &id, None)?;
                    backend.analyse(ctx, &id)?;
                }
                Ok(())
            })
        })();
        let ok = result.is_ok();
        self.finish(from, &id, &id, "importing the song", result);
        if ok {
            self.send_job(from, &id);
        }
    }

    /// `parts` then `synth-di`.
    fn pick(&self, from: u64, job: String, start: f32, end: f32, channel: TonelabChannel, label: String) {
        if !plain_name(&job) {
            return self.fail(from, format!("invalid job id '{job}'"));
        }
        if !(start.is_finite() && end.is_finite() && start >= 0.0 && end > start) {
            return self.fail(from, "the part must end after it starts".into());
        }
        let step = match self.begin(&job) {
            Ok(s) => s,
            Err(e) => return self.fail(from, format!("{e:#}")),
        };
        let mut warning = None;
        let result = (|| -> Result<(), StepError> {
            let jobs_dir = self.jobs_dir()?;
            read_job(&jobs_dir, &job)?;
            let backend = self.backend()?;
            self.with_ctx(from, &step, |ctx| {
                let part = backend.pick_part(ctx, &job, start, end, channel, &label)?;
                let di = newest_user_di(&jobs_dir);
                match backend.synth_di(ctx, &job, &part, di.as_deref()) {
                    Err(StepError::Failed(m)) => warning = Some(format!("the part was added, but the synthetic DI failed: {m}")),
                    other => other?,
                }
                Ok(())
            })
        })();
        let ok = result.is_ok();
        self.finish(from, &job, &job, "picking the part", result);
        if ok {
            if let Some(w) = warning {
                self.fail(from, w);
            }
            self.send_job(from, &job);
        }
    }

    /// `tonelab_record_di`: the backend's hook, then the refreshed job.
    fn record_di(&self, from: u64, job: String, part: String, seconds: f32) {
        if !plain_name(&job) || !plain_name(&part) {
            return self.fail(from, "invalid job or part id".into());
        }
        let step = match self.begin(&job) {
            Ok(s) => s,
            Err(e) => return self.fail(from, format!("{e:#}")),
        };
        let result = self.backend().and_then(|b| self.with_ctx(from, &step, |ctx| b.record_di(ctx, &job, &part, seconds)));
        let ok = result.is_ok();
        match result {
            Err(StepError::Failed(m)) => {
                self.end(&job);
                self.fail(from, m);
            }
            other => self.finish(from, &job, &job, "recording", other),
        }
        if ok {
            self.send_job(from, &job);
        }
    }

    /// The fit, in this process on its own thread (the GPU is not involved).
    fn fit(&self, from: u64, job: String, part: String) {
        if !plain_name(&job) || !plain_name(&part) {
            return self.fail(from, "invalid job or part id".into());
        }
        let step = match self.begin(&job) {
            Ok(s) => s,
            Err(e) => return self.fail(from, format!("{e:#}")),
        };
        let jobs_dir = match self.jobs_dir() {
            Ok(d) => d,
            Err(e) => {
                self.end(&job);
                return self.fail(from, format!("{e:#}"));
            }
        };
        let run = self.backend().and_then(|b| self.with_ctx(from, &step, |ctx| b.fit(ctx, &job, &part)));
        match run {
            Err(e) => self.finish(from, &job, &job, "the fit", Err(e)),
            Ok(summary) => {
                self.end(&job);
                self.progress_to(from, &job, TonelabStage::Fit, "fit done", Some(1.0));
                let r = &summary.report;
                self.to_all(ServerMsg::TonelabFitResult {
                    job: job.clone(),
                    part,
                    fit: TonelabFit { template: r.template.clone(), error_db: r.error_db, holdout_error_db: r.holdout_error_db, validate_error_db: r.validate_error_db },
                    templates: summary
                        .rows
                        .iter()
                        .map(|t| TonelabTemplateResult { template: t.template.clone(), error_db: t.error_db, renders: t.renders as u32, stage: t.stage.to_string() })
                        .collect(),
                });
                if let Ok(info) = read_job(&jobs_dir, &job) {
                    self.to_all(ServerMsg::TonelabJob { job: info });
                }
            }
        }
    }

    /// `tonelab_save_preset`: copies the fitted preset of a part into the library.
    pub fn save_preset(&self, job: &str, part: &str, name: &str) -> Result<()> {
        if !plain_name(job) || !plain_name(part) {
            bail!("invalid job or part id");
        }
        let graph = if self.is_demo() {
            self.demo.preset()
        } else {
            let path = self.jobs_dir()?.join(job).join("parts").join(part).join("fit/preset.toml");
            if !path.is_file() {
                bail!("part '{part}' of job '{job}' has no fitted preset yet: run the fit first");
            }
            openpedal_graph::load::read_graph(&path)?
        };
        self.library.save_preset(name, &graph)
    }

    /// A job file to serve, resolved inside the jobs directory.
    fn file_path(&self, job: &str, rest: &str) -> Result<PathBuf> {
        if !plain_name(job) {
            bail!("invalid job id");
        }
        let rel = safe_job_path(rest).ok_or_else(|| anyhow!("invalid file path"))?;
        let jobs_dir = self.jobs_dir()?;
        let real = jobs_dir.join(job).join(rel).canonicalize().map_err(|_| anyhow!("no such file"))?;
        let root = jobs_dir.join(job).canonicalize().map_err(|_| anyhow!("no such job"))?;
        if !real.starts_with(&root) || !real.is_file() {
            bail!("no such file");
        }
        Ok(real)
    }
}

/// `Range: bytes=a-b` for a file of `len` bytes: the inclusive byte range, capped at `MAX_RANGE`.
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
        (start, end)
    };
    if start > end || start >= len {
        return None;
    }
    Some((start, end.min(start + MAX_RANGE - 1)))
}

fn status(code: StatusCode, msg: &str) -> Response {
    (code, msg.to_string()).into_response()
}

/// Reads `file[start..=end]`.
fn read_slice(path: &Path, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((end + 1 - start) as usize);
    f.take(end + 1 - start).read_to_end(&mut buf)?;
    Ok(buf)
}

/// `GET /tonelab/file/{job}/{*path}`: a job's WAV file, with `Range` support so the player can seek.
pub async fn serve_file(State(state): State<Arc<AppState>>, UrlPath((job, rest)): UrlPath<(String, String)>, headers: HeaderMap) -> Response {
    let t = state.tonelab.clone();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let result = tokio::task::spawn_blocking(move || -> Result<Response, (StatusCode, String)> {
        let not_found = |m: &str| (StatusCode::NOT_FOUND, m.to_string());
        let source = if t.is_demo() {
            Err(t.demo.wav(&job, &rest).ok_or_else(|| not_found("no such file"))?)
        } else {
            Ok(t.file_path(&job, &rest).map_err(|e| not_found(&format!("{e:#}")))?)
        };
        let total = match &source {
            Ok(path) => std::fs::metadata(path).map_err(|_| not_found("no such file"))?.len(),
            Err(wav) => wav.len() as u64,
        };
        let picked = range.as_deref().and_then(|r| parse_range(r, total));
        let (start, end) = picked.unwrap_or((0, total.saturating_sub(1)));
        let bytes = match &source {
            Ok(path) => read_slice(path, start, end).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "cannot read the file".to_string()))?,
            Err(wav) => wav[start as usize..=end as usize].to_vec(),
        };
        let mut resp = Response::builder().header(header::CONTENT_TYPE, "audio/wav").header(header::ACCEPT_RANGES, "bytes").header(header::CACHE_CONTROL, "no-cache");
        if picked.is_some() {
            resp = resp.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
        }
        resp.body(Body::from(bytes)).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "cannot build the response".to_string()))
    })
    .await;
    match result {
        Ok(Ok(r)) => r,
        Ok(Err((code, msg))) => status(code, &msg),
        Err(_) => status(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

/// A file name that is safe on disk: only its last component, no odd characters.
pub fn clean_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' }).collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ');
    if cleaned.is_empty() {
        "upload".into()
    } else {
        cleaned.chars().take(100).collect()
    }
}

/// `POST /tonelab/upload`: the raw file as the body and its name in `X-Filename`; answers `{ "path" }`.
pub async fn upload(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Body) -> Response {
    if !crate::app::same_origin(&headers) {
        return status(StatusCode::FORBIDDEN, "origin not allowed");
    }
    let name = clean_filename(headers.get("x-filename").and_then(|v| v.to_str().ok()).unwrap_or(""));
    let bytes = match axum::body::to_bytes(body, MAX_UPLOAD).await {
        Ok(b) if !b.is_empty() => b,
        Ok(_) => return status(StatusCode::BAD_REQUEST, "empty upload"),
        Err(_) => return status(StatusCode::PAYLOAD_TOO_LARGE, "the file is too large"),
    };
    let t = state.tonelab.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let dir = t.jobs_dir()?.join("uploads");
        std::fs::create_dir_all(&dir)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = dir.join(format!("{stamp}-{name}"));
        std::fs::write(&path, &bytes)?;
        Ok(path)
    })
    .await;
    match result {
        Ok(Ok(path)) => axum::Json(serde_json::json!({ "path": path.to_string_lossy() })).into_response(),
        Ok(Err(e)) => status(StatusCode::SERVICE_UNAVAILABLE, &format!("{e:#}")),
        Err(_) => status(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

pub fn upload_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_UPLOAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> Library {
        Library::new(std::env::temp_dir().join(format!("openpedal-select-{}", std::process::id())))
    }

    fn pick(env: Option<&str>, repo: &Path, audio: bool, native: Option<NativeConfig>) -> String {
        match choose_backend(env, repo, audio, native, &lib()) {
            Choice::Backend(b) => b.name().into(),
            Choice::Demo => "demo".into(),
            Choice::Off(_) => "off".into(),
        }
    }

    #[test]
    fn backend_selection() {
        let none = Path::new("/nonexistent/tonelab");
        let tool = std::env::current_exe().unwrap();
        assert_eq!(pick(Some("mock"), none, true, None), "demo");
        assert_eq!(pick(None, none, false, None), "demo");
        assert_eq!(pick(None, none, true, None), "off");
        assert_eq!(pick(Some("none"), &tool, true, None), "off");
        assert_eq!(pick(Some("/nonexistent/x"), &tool, true, None), "off");
        assert_eq!(pick(Some(tool.to_str().unwrap()), none, true, None), "python");
        assert_eq!(pick(None, &tool, true, None), "python");
        // Native: this build (feature off) reports it off; with the feature and no weights, also off.
        assert_eq!(pick(Some("native"), &tool, true, None), "off");
        let cfg = NativeConfig { jobs_dir: "/tmp/x".into(), weights: None, templates_dir: None };
        assert_eq!(pick(None, &tool, true, Some(cfg)), "off");
    }

    #[cfg(feature = "native-tonelab")]
    #[test]
    fn native_is_chosen_when_the_weights_exist() {
        let w = std::env::current_exe().unwrap();
        let cfg = NativeConfig { jobs_dir: std::env::temp_dir().join("openpedal-native-sel"), weights: Some(w), templates_dir: None };
        // The host's choice beats the env and the repository tool.
        assert_eq!(pick(Some("none"), &std::env::current_exe().unwrap(), true, Some(cfg)), "native");
    }

    #[test]
    fn progress_lines_parse() {
        assert_eq!(
            parse_progress("PROGRESS separate 0.250 on the GPU now"),
            Some(ProgressLine { stage: TonelabStage::Separate, fraction: Some(0.25), message: "on the GPU now".into() })
        );
        assert_eq!(parse_progress("PROGRESS fetch - hi").unwrap().fraction, None);
        assert_eq!(parse_progress("PROGRESS synth 1 done").unwrap().stage, TonelabStage::Synth);
        assert_eq!(parse_progress("PROGRESS nope - x"), None);
        assert_eq!(parse_progress("tonelab: error"), None);
    }

    #[test]
    fn file_paths_are_sanitised() {
        assert_eq!(safe_job_path("stems/guitar.wav"), Some(PathBuf::from("stems/guitar.wav")));
        assert_eq!(safe_job_path("parts/p1/fit/render.wav"), Some(PathBuf::from("parts/p1/fit/render.wav")));
        for bad in ["", "../x.wav", "a/../../x.wav", "/etc/x.wav", "a\\b.wav", "job.json", "stems/"] {
            assert_eq!(safe_job_path(bad), None, "{bad}");
        }
        assert!(plain_name("2026-11-24-cryogen"));
        for bad in ["", ".", "..", ".hidden", "a/b", "a\\b"] {
            assert!(!plain_name(bad), "{bad}");
        }
        assert_eq!(clean_filename("../../etc/pass wd.mp3"), "pass wd.mp3");
        assert_eq!(clean_filename("..."), "upload");
        assert_eq!(clean_filename("my song (1).m4a"), "my song _1_.m4a");
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=0-999", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-1", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
        assert_eq!(parse_range("bytes=0-", 100 << 20).unwrap().1, MAX_RANGE - 1);
    }

    #[test]
    fn job_folders_are_read() {
        let dir = std::env::temp_dir().join(format!("openpedal-tonelab-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let job = dir.join("j1");
        std::fs::create_dir_all(job.join("parts/p1/fit")).unwrap();
        std::fs::write(job.join("job.json"), r#"{"id":"j1","source":{"kind":"file","ref":"x","title":"Song"},"created":"2026-11-24T10:00:00+01:00","duration_s":61.5}"#).unwrap();
        std::fs::write(job.join("analysis.json"), r#"{"tempo_bpm":120.5,"key":"A minor","sections":[{"start":0,"end":4,"guitar_db":-20.5,"corr":0.9,"kind":"centre"}]}"#).unwrap();
        std::fs::write(job.join("parts.json"), r#"[{"id":"p1","label":"riff","start":8,"end":28,"channel":"left","notes":[{},{}]}]"#).unwrap();
        std::fs::write(job.join("parts/p1/di-synth.wav"), b"").unwrap();
        std::fs::write(job.join("parts/p1/fit/report.json"), r#"{"template":"fuzz","error_db":2.5,"holdout_error_db":null,"validate_error_db":5.5}"#).unwrap();
        let info = read_job(&dir, "j1").unwrap();
        assert_eq!((info.title.as_str(), info.source_kind.as_str(), info.tempo_bpm, info.key.as_deref()), ("Song", "file", Some(120.5), Some("A minor")));
        assert_eq!(info.sections.len(), 1);
        let p = &info.parts[0];
        assert_eq!((p.notes, p.has_synth_di, p.has_user_di, p.channel), (2, true, false, TonelabChannel::Left));
        let fit = p.fit.as_ref().unwrap();
        assert_eq!((fit.template.as_str(), fit.error_db, fit.holdout_error_db, fit.validate_error_db), ("fuzz", 2.5, None, Some(5.5)));
        assert_eq!(list_jobs(&dir).len(), 1);
        assert!(read_job(&dir, "../x").is_err());
        assert!(read_job(&dir, "missing").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
