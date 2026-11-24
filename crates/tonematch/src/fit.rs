//! The fit procedure: screen every template, fully fit the best ones, hold out, write the job output.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};
use openpedal_dsp::wav;
use openpedal_graph::model::PedalFile;
use serde::Serialize;

use crate::features::{self, band_centres, Analyzer, Features};
use crate::optim::{self, Minimum};
use crate::render::render;
use crate::template::Template;

#[derive(Debug, Clone)]
pub struct FitOptions {
    pub templates_dir: PathBuf,
    pub pedals_dir: PathBuf,
    /// Only this template (no screening).
    pub template: Option<String>,
    /// The user's DI (enables the hold-out).
    pub di: Option<PathBuf>,
    /// Another real DI to render the fitted preset on, compared with the target.
    pub validate_di: Option<PathBuf>,
    /// Evaluations of the full fit (default 350, 2 restarts); screening has its own budget.
    pub budget: usize,
    pub threads: usize,
    /// Length of the densest window used for fitting.
    pub window_s: f32,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            templates_dir: "content/tonematch/templates".into(),
            pedals_dir: "content/pedals".into(),
            template: None,
            di: None,
            validate_di: None,
            budget: 350,
            threads: 4,
            window_s: 8.0,
        }
    }
}

/// One line of the final table.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateResult {
    pub template: String,
    /// Spectral error on the fitting window, dB.
    pub error_db: f32,
    pub holdout_error_db: Option<f32>,
    /// `screen` (max(60, 6 x params) renders) or `fit` (full budget).
    pub stage: &'static str,
    pub renders: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrestDb {
    pub target: f32,
    pub render: f32,
}

/// `report.json` (§14.2), plus `validate_error_db` when `--validate-di` was given.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub template: String,
    pub params: BTreeMap<String, f32>,
    pub error_db: f32,
    pub holdout_error_db: Option<f32>,
    pub crest_db: CrestDb,
    pub bands_hz: Vec<f32>,
    pub diff_db: Vec<f32>,
    pub renders: usize,
    /// Renders spent on each template (screening plus full fit).
    pub templates: Vec<TemplateResult>,
    pub seconds: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_error_db: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct FitSummary {
    pub report: Report,
    /// The cost (spectral error + crest term) of the winner on the fitting window.
    pub cost: f32,
    pub rows: Vec<TemplateResult>,
    pub out_dir: Option<PathBuf>,
}

/// Progress lines.
pub type Progress<'a> = &'a (dyn Fn(&str) + Sync);

/// The result of fitting signals (no files).
pub struct SignalFit {
    pub template: Template,
    pub values: Vec<f32>,
    pub summary: FitSummary,
}

/// Screening budget per template: enough to get past the initial simplex of `n + 1` renders.
pub fn screen_budget(n_params: usize) -> usize {
    60.max(6 * n_params)
}

/// Start of the window of `len` samples with the most energy (steps of half a second).
pub fn densest_window(x: &[f32], len: usize, sample_rate: f32) -> usize {
    if x.len() <= len {
        return 0;
    }
    let mut cum = Vec::with_capacity(x.len() + 1);
    cum.push(0f64);
    let mut acc = 0f64;
    for &s in x {
        acc += f64::from(s) * f64::from(s);
        cum.push(acc);
    }
    let step = (sample_rate / 2.0) as usize;
    let (mut best, mut best_e) = (0, -1.0);
    let mut start = 0;
    while start + len <= x.len() {
        let e = cum[start + len] - cum[start];
        if e > best_e {
            best_e = e;
            best = start;
        }
        start += step.max(1);
    }
    best
}

struct Problem<'a> {
    template: &'a Template,
    pedals: &'a [(String, PedalFile)],
    di: &'a [f32],
    target: &'a Features,
    sr: f32,
}

impl Problem<'_> {
    fn features(&self, values: &[f32], analyzer: &mut Analyzer, di: &[f32]) -> Result<Features> {
        let wet = render(&self.template.with_values(values), self.pedals, di, self.sr)?;
        Ok(analyzer.analyze(&wet))
    }

    /// Optimises from `start` (parameter values) within `budget` evaluations.
    fn optimise(&self, start: &[f32], budget: usize, restarts: usize) -> (Minimum, Vec<f32>) {
        let mut analyzer = Analyzer::new(self.sr);
        let mut f = |x: &[f64]| -> f64 {
            let values = self.template.values_from_unit(x);
            match self.features(&values, &mut analyzer, self.di) {
                Ok(feat) => f64::from(features::cost(&feat, self.target)),
                Err(_) => 1e3,
            }
        };
        let min = optim::minimize(&mut f, &self.template.unit_from_values(start), budget, restarts);
        let values = self.template.values_from_unit(&min.x);
        (min, values)
    }
}

/// Runs `jobs` on at most `threads` threads and returns the results in job order.
fn parallel<T: Sync, R: Send>(jobs: &[T], threads: usize, work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, jobs.len().max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(job) = jobs.get(i) else { break };
                let r = work(job);
                out.lock().unwrap().push((i, r));
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

/// Fits the templates to `target` played from `di` (same length and rate, sample i against sample i).
/// With `holdout`, the first 70 % is used for fitting and the last 30 % is measured only.
#[allow(clippy::too_many_arguments)]
pub fn fit_signals(
    templates: Vec<Template>,
    pedals: &[(String, PedalFile)],
    di: &[f32],
    target: &[f32],
    sr: f32,
    opts: &FitOptions,
    holdout: bool,
    progress: Progress,
) -> Result<SignalFit> {
    ensure!(!templates.is_empty(), "no templates");
    let t0 = Instant::now();
    let n = di.len().min(target.len());
    let (di, target) = (&di[..n], &target[..n]);
    let split = if holdout { n * 7 / 10 } else { n };
    let win = ((opts.window_s * sr) as usize).min(split);
    let w0 = densest_window(&target[..split], win, sr);
    let (fit_di, fit_target) = (&di[w0..w0 + win], &target[w0..w0 + win]);
    progress(&format!(
        "fitting window {:.1}-{:.1} s of {:.1} s{}",
        w0 as f32 / sr,
        (w0 + win) as f32 / sr,
        n as f32 / sr,
        if holdout { ", hold-out = last 30 %" } else { "" }
    ));
    let target_feat = Analyzer::new(sr).analyze(fit_target);
    let hold_feat = holdout.then(|| Analyzer::new(sr).analyze(&target[split..]));

    let full_budget = opts.budget.max(20);
    let mut renders = 0usize;

    // Screening.
    let starts: Vec<Vec<f32>> = templates.iter().map(|t| t.initial(pedals)).collect();
    let mut stage: Vec<(Vec<f32>, f32, usize)> = starts.iter().map(|s| (s.clone(), f32::MAX, 0)).collect();
    if templates.len() > 1 {
        progress(&format!(
            "screening {} templates with max(60, 6 x params) renders each, no restarts",
            templates.len()
        ));
        let jobs: Vec<usize> = (0..templates.len()).collect();
        let results = parallel(&jobs, opts.threads, |&i| {
            let screen_budget = screen_budget(templates[i].params.len());
            let p = Problem { template: &templates[i], pedals, di: fit_di, target: &target_feat, sr };
            let (min, values) = p.optimise(&starts[i], screen_budget, 0);
            progress(&format!("  screen {:<14} cost {:.2} ({} renders)", templates[i].name, min.value, min.evals));
            (values, min.value as f32, min.evals)
        });
        for (i, r) in results.into_iter().enumerate() {
            renders += r.2;
            stage[i] = r;
        }
    }
    let mut order: Vec<usize> = (0..templates.len()).collect();
    order.sort_by(|a, b| stage[*a].1.total_cmp(&stage[*b].1));
    let finalists: Vec<usize> = if templates.len() > 1 { order.iter().take(2).cloned().collect() } else { vec![0] };

    // Full fit of the best ones, continuing from their screening point.
    progress(&format!(
        "full fit of {} with ~{full_budget} renders each",
        finalists.iter().map(|i| templates[*i].name.as_str()).collect::<Vec<_>>().join(", ")
    ));
    let fits = parallel(&finalists, opts.threads, |&i| {
        let p = Problem { template: &templates[i], pedals, di: fit_di, target: &target_feat, sr };
        let (min, values) = p.optimise(&stage[i].0, full_budget, 2);
        progress(&format!("  fit    {:<14} cost {:.2} ({} renders)", templates[i].name, min.value, min.evals));
        (values, min.value as f32, min.evals)
    });

    // Re-measure each finalist with the rounded values it would be saved with.
    struct Final {
        index: usize,
        renders: usize,
        values: Vec<f32>,
        cost: f32,
        error: f32,
        holdout: Option<f32>,
        feat: Features,
    }
    let mut finals = Vec::new();
    let mut an = Analyzer::new(sr);
    for (&i, (values, _, evals)) in finalists.iter().zip(fits) {
        renders += evals;
        let template_renders = evals + if templates.len() > 1 { stage[i].2 } else { 0 } + 1 + usize::from(holdout);
        let t = &templates[i];
        let rounded: Vec<f32> = t.params.iter().zip(&values).map(|(p, v)| p.round(*v)).collect();
        let p = Problem { template: t, pedals, di: fit_di, target: &target_feat, sr };
        let feat = p.features(&rounded, &mut an, fit_di)?;
        let holdout_err = match &hold_feat {
            Some(h) => Some(features::spectral_error(&p.features(&rounded, &mut an, &di[split..])?, h)),
            None => None,
        };
        renders += 1 + usize::from(holdout);
        finals.push(Final {
            index: i,
            renders: template_renders,
            values: rounded,
            cost: features::cost(&feat, &target_feat),
            error: features::spectral_error(&feat, &target_feat),
            holdout: holdout_err,
            feat,
        });
    }
    finals.sort_by(|a, b| a.cost.total_cmp(&b.cost));

    let mut rows: Vec<TemplateResult> = Vec::new();
    for f in &finals {
        rows.push(TemplateResult {
            template: templates[f.index].name.clone(),
            error_db: f.error,
            holdout_error_db: f.holdout,
            stage: "fit",
            renders: f.renders,
        });
    }
    for &i in &order {
        if !finalists.contains(&i) {
            // Screening cost = error + crest term; the table shows it as an indication.
            rows.push(TemplateResult { template: templates[i].name.clone(), error_db: stage[i].1, holdout_error_db: None, stage: "screen", renders: stage[i].2 });
        }
    }

    let best = &finals[0];
    let template = templates[best.index].clone();
    let params = template.params.iter().zip(&best.values).map(|(p, v)| (format!("{}.{}", p.node, p.key), *v)).collect();
    let report = Report {
        template: template.name.clone(),
        params,
        error_db: best.error,
        holdout_error_db: best.holdout,
        crest_db: CrestDb { target: target_feat.crest_db, render: best.feat.crest_db },
        bands_hz: band_centres().iter().map(|f| f.round()).collect(),
        diff_db: features::diff_db(&best.feat, &target_feat).iter().map(|d| (d * 100.0).round() / 100.0).collect(),
        renders,
        templates: rows.clone(),
        seconds: t0.elapsed().as_secs_f32(),
        validate_error_db: None,
    };
    Ok(SignalFit { values: best.values.clone(), template, summary: FitSummary { report, cost: best.cost, rows, out_dir: None } })
}

fn read_mono(path: &Path) -> Result<(Vec<f32>, f32)> {
    let w = wav::read(path)?;
    ensure!(!w.channels.is_empty() && !w.channels[0].is_empty(), "{} is empty", path.display());
    Ok((w.channels[0].clone(), w.sample_rate as f32))
}

fn json_file(path: &Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("error in {}", path.display()))
}

/// The DI to fit through, and whether the hold-out applies (a real recording): `--di`, then the
/// user's `di-user.wav`, then the synthetic `di-synth.wav`.
pub fn choose_di(explicit: Option<&Path>, part_dir: &Path) -> Option<(PathBuf, bool)> {
    if let Some(p) = explicit {
        return Some((p.to_path_buf(), true));
    }
    let user = part_dir.join("di-user.wav");
    if user.exists() {
        return Some((user, true));
    }
    let synth = part_dir.join("di-synth.wav");
    synth.exists().then_some((synth, false))
}

fn di_kind(explicit: bool, path: &Path) -> &'static str {
    if explicit {
        "--di"
    } else if path.file_name().is_some_and(|n| n == "di-user.wav") {
        "your recording"
    } else {
        "synthetic"
    }
}

/// Fits a part of a job folder and writes `parts/<part>/fit/`.
pub fn fit_job(job_dir: &Path, part: &str, opts: &FitOptions, progress: Progress) -> Result<FitSummary> {
    let t0 = Instant::now();
    let job = json_file(&job_dir.join("job.json"))?;
    let parts = json_file(&job_dir.join("parts.json"))?;
    let part_info = parts
        .as_array()
        .and_then(|a| a.iter().find(|p| p["id"].as_str() == Some(part)))
        .with_context(|| format!("part '{part}' is not in parts.json"))?;
    let job_id = job["id"].as_str().map(str::to_string).or_else(|| job_dir.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    let part_dir = job_dir.join("parts").join(part);

    let (target, sr) = read_mono(&part_dir.join("target.wav"))?;
    let synth = part_dir.join("di-synth.wav");
    let user = part_dir.join("di-user.wav");
    let (di_path, holdout) = match choose_di(opts.di.as_deref(), &part_dir) {
        Some(c) => c,
        None => bail!("no DI: expected {} or {}, or pass --di", user.display(), synth.display()),
    };
    progress(&format!("DI used: {} ({})", di_path.display(), di_kind(opts.di.is_some(), &di_path)));
    let (di, di_sr) = read_mono(&di_path)?;
    ensure!(di_sr == sr, "DI sample rate {di_sr} differs from the target's {sr}");
    progress(&format!("DI {} ({:.1} s), target {:.1} s", di_path.display(), di.len() as f32 / sr, target.len() as f32 / sr));

    let pedals = openpedal_graph::load::read_pedals(&opts.pedals_dir).unwrap_or_default();
    let names = match &opts.template {
        Some(n) => vec![n.clone()],
        None => Template::list(&opts.templates_dir),
    };
    ensure!(!names.is_empty(), "no templates in {}", opts.templates_dir.display());
    let mut templates = Vec::new();
    for name in &names {
        match Template::load(&opts.templates_dir, name, &pedals) {
            Ok(t) => templates.push(t),
            Err(e) if opts.template.is_none() => progress(&format!("skipping template {name}: {e:#}")),
            Err(e) => return Err(e),
        }
    }
    ensure!(!templates.is_empty(), "no valid templates in {}", opts.templates_dir.display());

    let mut fit = fit_signals(templates, &pedals, &di, &target, sr, opts, holdout, progress)?;
    let preset_name = format!("tm-{job_id}-{part}");
    let mut graph = fit.template.with_values(&fit.values);
    graph.name = preset_name.clone();

    // Full render of the fitted preset.
    let wet = render(&graph, &pedals, &di, sr)?;
    let out_dir = part_dir.join("fit");
    std::fs::create_dir_all(&out_dir)?;
    let mut w = wav::WavWriter::create(&out_dir.join("render.wav"), 1, sr as u32)?;
    for s in &wet {
        w.write(*s)?;
    }
    w.finish()?;

    if let Some(path) = &opts.validate_di {
        let (vdi, vsr) = read_mono(path)?;
        ensure!(vsr == sr, "validation DI sample rate {vsr} differs from the target's {sr}");
        let vwet = render(&graph, &pedals, &vdi, sr)?;
        let mut an = Analyzer::new(sr);
        let err = features::spectral_error(&an.analyze(&vwet), &an.analyze(&target));
        fit.summary.report.validate_error_db = Some(err);
        progress(&format!("validation on {}: {err:.2} dB", path.display()));
    }

    fit.summary.report.seconds = t0.elapsed().as_secs_f32();
    let r = &fit.summary.report;
    let s = |v: &serde_json::Value| v.as_f64().map(|x| format!("{x:.1}")).unwrap_or_else(|| "?".into());
    let header = format!(
        "# Tone Match: part \"{}\" of \"{}\" ({}-{} s), template {}.\n# Spectral error {:.2} dB on the fit window, hold-out {}.\n",
        part_info["label"].as_str().unwrap_or(part),
        job["source"]["title"].as_str().unwrap_or(&job_id),
        s(&part_info["start"]),
        s(&part_info["end"]),
        r.template,
        r.error_db,
        r.holdout_error_db.map(|h| format!("{h:.2} dB")).unwrap_or_else(|| "none".into()),
    );
    let body = toml::to_string_pretty(&graph).context("cannot serialize the preset")?;
    std::fs::write(out_dir.join("preset.toml"), format!("{header}{body}"))?;
    std::fs::write(out_dir.join("report.json"), serde_json::to_string_pretty(r)?)?;
    fit.summary.out_dir = Some(out_dir);
    Ok(fit.summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn di_priority_is_explicit_then_user_then_synth() {
        let dir = std::env::temp_dir().join(format!("tonematch-di-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(choose_di(None, &dir).is_none());
        std::fs::write(dir.join("di-synth.wav"), b"").unwrap();
        assert_eq!(choose_di(None, &dir), Some((dir.join("di-synth.wav"), false)));
        std::fs::write(dir.join("di-user.wav"), b"").unwrap();
        assert_eq!(choose_di(None, &dir), Some((dir.join("di-user.wav"), true)));
        let own = Path::new("/x/own.wav");
        assert_eq!(choose_di(Some(own), &dir), Some((own.to_path_buf(), true)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
