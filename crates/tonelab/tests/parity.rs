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

fn bands(x: &[f32], sr: f64) -> Vec<f64> {
    let mut fft = openpedal_tonelab::dsp::Fft::new();
    let xd: Vec<f64> = x.iter().map(|&v| v as f64).collect();
    let (f, p) = openpedal_tonelab::dsp::welch(&mut fft, &xd, sr, 8192);
    let w = 2f64.powf(1.0 / 6.0);
    (0..).map(|n| 1000.0 * 2f64.powf(n as f64 / 3.0 - 10.0 / 3.0)).take_while(|c| *c < 10000.0).filter(|c| *c >= 100.0).map(|c| {
        let s: f64 = f.iter().zip(&p).filter(|(fi, _)| **fi >= c / w && **fi < c * w).map(|(_, p)| p).sum();
        10.0 * (s + 1e-30).log10()
    }).collect()
}

#[test]
#[ignore]
fn synth_di_spectrum_matches_python() {
    let job = job();
    let py = job.read_parts().unwrap().into_iter().find(|p| p.id == "p2").unwrap();
    // The stored di-synth.wav predates the current Python synth (same notes give a different tilt),
    // so compare against a fresh run: TONELAB_PY_DI=<wav written by tonelab.synth.synth_di(notes)>.
    let want = match std::env::var("TONELAB_PY_DI") {
        Ok(p) => openpedal_dsp::wav::read(&PathBuf::from(p)).map(|d| Audio { sample_rate: d.sample_rate, channels: d.channels }).unwrap(),
        Err(_) => job.read_part_wav("p2", "di-synth.wav").unwrap(),
    };
    let got = synth_di(&py.notes, 48000, None);
    let (a, b) = (bands(&got, 48000.0), bands(&want.channels[0], 48000.0));
    let d: Vec<f64> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
    let rms = (d.iter().map(|v| v * v).sum::<f64>() / d.len() as f64).sqrt();
    println!("bands {} rms diff {rms:.2} dB; diffs {:?}", d.len(), d.iter().map(|v| (v * 10.0).round() / 10.0).collect::<Vec<_>>());
    assert!(rms < 1.0);
}
