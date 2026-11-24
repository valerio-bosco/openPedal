//! Parity with the Python tool on the Cryogen job. Needs audio, so every test is #[ignore]d:
//! `TONELAB_JOB=~/.local/share/openpedal/tonelab/jobs/2026-11-24-cryogen cargo test -p openpedal-tonelab --release -- --ignored --nocapture`
use openpedal_tonelab::*;
use std::path::PathBuf;

fn job() -> Job {
    let p = std::env::var("TONELAB_JOB").expect("set TONELAB_JOB");
    Job::open(&PathBuf::from(p)).unwrap()
}

fn mono(a: &Audio) -> Vec<f32> {
    a.channels[0].clone()
}

#[test]
#[ignore]
fn transcribe_p2_matches_python() {
    let job = job();
    let target = job.read_part_wav("p2", "target.wav").unwrap();
    let py = job.read_parts().unwrap().into_iter().find(|p| p.id == "p2").unwrap();
    let t0 = std::time::Instant::now();
    let notes = transcribe(&mono(&target), target.sample_rate);
    let dt = t0.elapsed().as_secs_f64();
    let matched = py
        .notes
        .iter()
        .filter(|p| notes.iter().any(|n| n.midi == p.midi && (n.t - p.t).abs() <= 0.040))
        .count();
    let frac = matched as f64 / py.notes.len() as f64;
    println!("python {} notes, rust {} notes, matched {:.1}%, {:.2}s for {:.0}s", py.notes.len(), notes.len(), frac * 100.0, dt, target.channels[0].len() as f64 / target.sample_rate as f64);
    assert!(frac >= 0.85);
}
