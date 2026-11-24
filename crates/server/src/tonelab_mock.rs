//! Tone Lab without Python or a GPU: a fake job answers every `tonelab_*` message, so the
//! interface can be developed and tested end to end (`openpedal ui --no-audio` where the tool is
//! not installed, or `OPENPEDAL_TONELAB=mock`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openpedal_graph::model::{EdgeSpec, GraphFile, NodeSpec, INPUT, OUTPUT};
use openpedal_protocol::{
    ClientMsg, ServerMsg, TonelabChannel, TonelabFit, TonelabJobInfo, TonelabJobSummary, TonelabPart, TonelabSection, TonelabStage,
    TonelabTemplateResult,
};

use crate::tonelab::{Tonelab, NEW_JOB};

/// Id of the fake job.
pub const DEMO_JOB: &str = "2026-11-24-demo-song";
/// Time between two fake progress messages.
const TICK: Duration = Duration::from_millis(120);

/// Where the fake server keeps uploads.
pub fn demo_dir() -> PathBuf {
    std::env::temp_dir().join("openpedal-tonelab-demo")
}

pub fn demo_fit() -> TonelabFit {
    TonelabFit { template: "fuzz-octave".into(), error_db: 1.89, holdout_error_db: None, validate_error_db: Some(5.33) }
}

/// The fake job as it is when opened: a few sections and one fitted part.
pub fn demo_job() -> TonelabJobInfo {
    let section = |start, end, kind: &str, guitar_db, corr| TonelabSection { start, end, kind: kind.into(), guitar_db, corr };
    TonelabJobInfo {
        id: DEMO_JOB.into(),
        title: "Demo song".into(),
        source_kind: "file".into(),
        duration_s: 60.0,
        tempo_bpm: Some(132.5),
        key: Some("B minor".into()),
        sections: vec![
            section(0.0, 8.0, "double", -29.2, 0.74),
            section(8.0, 28.0, "centre", -17.8, 0.91),
            section(28.0, 44.0, "wide", -22.4, 0.31),
            section(44.0, 60.0, "none", -58.0, 0.0),
        ],
        parts: vec![TonelabPart {
            id: "p1".into(),
            label: "riff".into(),
            start: 8.0,
            end: 28.0,
            channel: TonelabChannel::Mid,
            notes: 42,
            has_user_di: false,
            has_synth_di: true,
            fit: Some(demo_fit()),
        }],
    }
}

pub fn demo_templates() -> Vec<TonelabTemplateResult> {
    let row = |template: &str, error_db, renders, stage: &str| TonelabTemplateResult { template: template.into(), error_db, renders, stage: stage.into() };
    vec![
        row("fuzz-octave", 1.89, 412, "fit"),
        row("fuzz", 1.92, 410, "fit"),
        row("highgain", 3.4, 61, "screen"),
        row("overdrive", 4.1, 61, "screen"),
        row("crunch", 5.2, 61, "screen"),
        row("clean", 7.8, 61, "screen"),
    ]
}

/// A short stereo-free sine chord as a WAV (mono, 16 bit, 48 kHz): what the player gets for any file.
fn demo_wav() -> Vec<u8> {
    let rate = 48000u32;
    let n = rate as usize * 2;
    let mut data = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let env = (1.0 - t / 2.0).max(0.0);
        let s = env * 0.25 * ((t * 110.0 * std::f32::consts::TAU).sin() + 0.5 * (t * 220.0 * std::f32::consts::TAU).sin());
        data.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// State of the fake server: the one job, which `tonelab_pick`, `tonelab_record_di` and `tonelab_fit` change.
pub struct Demo {
    job: Mutex<TonelabJobInfo>,
}

impl Demo {
    pub fn new() -> Self {
        Self { job: Mutex::new(demo_job()) }
    }

    fn job(&self) -> std::sync::MutexGuard<'_, TonelabJobInfo> {
        self.job.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The preset `tonelab_save_preset` stores: a small fuzz chain.
    pub fn preset(&self) -> GraphFile {
        let node = |id: &str, kind: &str, x: f32| NodeSpec { id: id.into(), kind: kind.into(), pos: [x, 0.0], params: Default::default(), bypass: false };
        let edge = |from: &str, to: &str| EdgeSpec { from: from.into(), to: to.into() };
        GraphFile {
            name: "tm-demo".into(),
            version: 2,
            nodes: vec![node("in", INPUT, 0.0), node("fuzz", "fuzz", 220.0), node("out", OUTPUT, 440.0)],
            edges: vec![edge("in", "fuzz"), edge("fuzz", "out")],
        }
    }

    /// The bytes of any WAV of the fake job.
    pub fn wav(&self, job: &str, path: &str) -> Option<Vec<u8>> {
        (job == DEMO_JOB && crate::tonelab::safe_job_path(path).is_some()).then(demo_wav)
    }
}

impl Default for Demo {
    fn default() -> Self {
        Self::new()
    }
}

/// Fake progress: `steps` messages from `from_fraction` to 1.
fn run(t: &Tonelab, from: u64, job: &str, stage: TonelabStage, what: &str, steps: u32) {
    for i in 0..steps {
        t.progress_to(from, job, stage, format!("{what} ({}/{steps})", i + 1), Some(i as f32 / steps as f32));
        std::thread::sleep(TICK);
    }
    t.progress_to(from, job, stage, format!("{what}: done"), Some(1.0));
}

fn in_thread(t: &Arc<Tonelab>, f: impl FnOnce(&Arc<Tonelab>) + Send + 'static) {
    let t = t.clone();
    std::thread::spawn(move || f(&t));
}

/// Answers a `tonelab_*` message (not `tonelab_save_preset`) from the fake job.
pub(crate) fn handle(t: &Arc<Tonelab>, from: u64, msg: ClientMsg) {
    match msg {
        ClientMsg::TonelabCaps => t.to_client(from, ServerMsg::TonelabCaps { available: true, youtube: true, device: "cpu".into(), reason: None }),
        ClientMsg::TonelabJobs => {
            let j = t.demo.job();
            t.to_client(from, ServerMsg::TonelabJobs { jobs: vec![TonelabJobSummary { id: j.id.clone(), title: j.title.clone(), created: "2026-11-24T10:00:00+01:00".into(), duration_s: j.duration_s }] });
        }
        ClientMsg::TonelabOpen { job } => {
            let found = t.demo.job().clone();
            if job == found.id {
                t.to_all(ServerMsg::TonelabJob { job: found });
            } else {
                t.fail(from, format!("job '{job}' not found"));
            }
        }
        ClientMsg::TonelabStart { .. } => in_thread(t, move |t| {
            let step = match t.begin(NEW_JOB) {
                Ok(s) => s,
                Err(e) => return t.fail(from, format!("{e:#}")),
            };
            for (stage, what) in [(TonelabStage::Fetch, "reading the song"), (TonelabStage::Separate, "separating the stems"), (TonelabStage::Analyse, "analysing the song")] {
                if step.is_cancelled() {
                    return t.finish(from, NEW_JOB, NEW_JOB, "importing the song", Err(crate::tonelab::StepError::Cancelled));
                }
                run(t, from, NEW_JOB, stage, what, 3);
            }
            t.end(NEW_JOB);
            *t.demo.job() = demo_job();
            t.to_all(ServerMsg::TonelabJob { job: demo_job() });
        }),
        ClientMsg::TonelabPick { job, start, end, channel, label } => in_thread(t, move |t| {
            if end <= start || start < 0.0 {
                return t.fail(from, "the part must end after it starts".into());
            }
            let step = match t.begin(&job) {
                Ok(s) => s,
                Err(e) => return t.fail(from, format!("{e:#}")),
            };
            run(t, from, &job, TonelabStage::Parts, "extracting and transcribing the part", 3);
            run(t, from, &job, TonelabStage::Synth, "synthesising the DI", 2);
            if step.is_cancelled() {
                return t.finish(from, &job, &job, "picking the part", Err(crate::tonelab::StepError::Cancelled));
            }
            t.end(&job);
            let info = {
                let mut j = t.demo.job();
                let n = j.parts.iter().filter_map(|p| p.id.strip_prefix('p')?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
                j.parts.push(TonelabPart {
                    id: format!("p{n}"),
                    label: if label.trim().is_empty() { format!("part {n}") } else { label.trim().to_string() },
                    start,
                    end,
                    channel,
                    notes: 30,
                    has_user_di: false,
                    has_synth_di: true,
                    fit: None,
                });
                j.clone()
            };
            t.to_all(ServerMsg::TonelabJob { job: info });
        }),
        ClientMsg::TonelabRecordDi { job, part, seconds } => in_thread(t, move |t| {
            let step = match t.begin(&job) {
                Ok(s) => s,
                Err(e) => return t.fail(from, format!("{e:#}")),
            };
            if !t.demo.job().parts.iter().any(|p| p.id == part) {
                t.end(&job);
                return t.fail(from, format!("job '{job}' has no part '{part}'"));
            }
            let steps = (seconds.clamp(0.0, 1.0) * 5.0).ceil() as u32 + 2;
            run(t, from, &job, TonelabStage::Record, "recording the live input", steps);
            if step.is_cancelled() {
                return t.finish(from, &job, &job, "recording", Err(crate::tonelab::StepError::Cancelled));
            }
            t.end(&job);
            let info = {
                let mut j = t.demo.job();
                if let Some(p) = j.parts.iter_mut().find(|p| p.id == part) {
                    p.has_user_di = true;
                }
                j.clone()
            };
            t.to_all(ServerMsg::TonelabJob { job: info });
        }),
        ClientMsg::TonelabFit { job, part } => in_thread(t, move |t| {
            let step = match t.begin(&job) {
                Ok(s) => s,
                Err(e) => return t.fail(from, format!("{e:#}")),
            };
            let user_di = match t.demo.job().parts.iter().find(|p| p.id == part) {
                Some(p) => p.has_user_di,
                None => {
                    t.end(&job);
                    return t.fail(from, format!("job '{job}' has no part '{part}'"));
                }
            };
            run(t, from, &job, TonelabStage::Fit, "fitting the templates", 5);
            if step.is_cancelled() {
                return t.finish(from, &job, &job, "the fit", Err(crate::tonelab::StepError::Cancelled));
            }
            t.end(&job);
            let fit = TonelabFit { template: "fuzz-octave".into(), error_db: 1.89, holdout_error_db: user_di.then_some(3.4), validate_error_db: None };
            let info = {
                let mut j = t.demo.job();
                if let Some(p) = j.parts.iter_mut().find(|p| p.id == part) {
                    p.fit = Some(fit.clone());
                }
                j.clone()
            };
            t.to_all(ServerMsg::TonelabFitResult { job: job.clone(), part, fit, templates: demo_templates() });
            t.to_all(ServerMsg::TonelabJob { job: info });
        }),
        ClientMsg::TonelabCancel { job } => match t.step_of(&job) {
            Some(s) => s.cancel(),
            None => t.fail(from, format!("nothing is running for job '{job}'")),
        },
        _ => {}
    }
}
