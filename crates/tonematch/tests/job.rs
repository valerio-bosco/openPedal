use std::path::PathBuf;

use openpedal_dsp::wav::WavWriter;
use openpedal_tonematch::fit::{fit_job, FitOptions};

const SR: u32 = 48000;

fn write_wav(path: &std::path::Path, x: &[f32]) {
    let mut w = WavWriter::create(path, 1, SR).unwrap();
    for s in x {
        w.write(*s).unwrap();
    }
    w.finish().unwrap();
}

#[test]
fn fits_a_job_folder_and_writes_the_outputs() {
    let job = std::env::temp_dir().join(format!("tonematch-job-{}", std::process::id()));
    let part = job.join("parts/riff");
    std::fs::create_dir_all(&part).unwrap();
    std::fs::write(job.join("job.json"), r#"{"id":"j1","source":{"kind":"file","ref":"x","title":"Song"}}"#).unwrap();
    std::fs::write(job.join("parts.json"), r#"[{"id":"riff","label":"Riff","start":8.0,"end":12.0,"channel":"mid","notes":[]}]"#).unwrap();
    let di: Vec<f32> = (0..SR * 3).map(|i| 0.05 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / SR as f32).sin()).collect();
    write_wav(&part.join("di-synth.wav"), &di);
    write_wav(&part.join("target.wav"), &di.iter().map(|s| s * 2.0).collect::<Vec<_>>());

    let opts = FitOptions {
        templates_dir: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")),
        template: Some("rec".into()),
        budget: 20,
        window_s: 2.0,
        ..FitOptions::default()
    };
    let summary = fit_job(&job, "riff", &opts, &|_| {}).unwrap();
    let out = part.join("fit");
    let preset = std::fs::read_to_string(out.join("preset.toml")).unwrap();
    assert!(preset.starts_with("# Tone Match: part \"Riff\" of \"Song\" (8.0-12.0 s), template rec."));
    assert!(preset.contains("name = \"tm-j1-riff\""));
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    for key in ["template", "params", "error_db", "holdout_error_db", "crest_db", "bands_hz", "diff_db", "renders", "seconds"] {
        assert!(report.get(key).is_some(), "report.json lacks {key}");
    }
    assert!(report["holdout_error_db"].is_null());
    assert_eq!(report["bands_hz"].as_array().unwrap().len(), 25);
    assert_eq!(openpedal_dsp::wav::read(&out.join("render.wav")).unwrap().channels[0].len(), di.len());
    assert_eq!(summary.rows.len(), 1);
    std::fs::remove_dir_all(&job).ok();
}
