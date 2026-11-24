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

#[test]
#[ignore]
fn analyse_matches_python() {
    let job = job();
    let py = job.read_analysis().unwrap();
    let guitar = job.read_stem("guitar").unwrap().unwrap();
    let bass = job.read_stem("bass").unwrap();
    let mix = job.read_mix().unwrap();
    let t0 = std::time::Instant::now();
    let a = analyse_audio(&guitar, bass.as_ref(), &mix); // does not overwrite analysis.json
    let dt = t0.elapsed().as_secs_f64();
    let kind_at = |s: &[Section], t: f64| s.iter().find(|x| t >= x.start && t < x.end).map(|x| x.kind.clone());
    let dur = py.sections.last().unwrap().end;
    let step = 0.05;
    let n = (dur / step) as usize;
    let same = (0..n).filter(|i| kind_at(&a.sections, *i as f64 * step) == kind_at(&py.sections, *i as f64 * step)).count();
    let frac = same as f64 / n as f64;
    println!("tempo rust {} python {}; key rust {} python {}; kinds agree on {:.1}% of {:.0}s; low notes rust {} python {}; {:.1}s", a.tempo_bpm, py.tempo_bpm, a.key, py.key, frac * 100.0, dur, a.low_notes.len(), py.low_notes.len(), dt);
    for (r, p) in a.sections.iter().zip(&py.sections).filter(|(r, p)| r.kind != p.kind || (r.start - p.start).abs() > 0.01) {
        println!("  diff rust {:?} {}-{}  python {:?} {}-{}", r.kind, r.start, r.end, p.kind, p.start, p.end);
    }
    assert!(frac >= 0.90);
    assert!((a.tempo_bpm - py.tempo_bpm).abs() <= 2.0);
    assert_eq!(a.key, py.key);
}

#[test]
#[ignore]
fn pick_part_matches_python_p2() {
    // copy only job.json + the guitar stem into a scratch job so the real job is untouched
    let src = job();
    let tmp = std::env::temp_dir().join(format!("tonelab-par-{}", std::process::id()));
    std::fs::create_dir_all(tmp.join("stems")).unwrap();
    std::fs::copy(src.dir.join("job.json"), tmp.join("job.json")).unwrap();
    std::fs::copy(src.stem_path("guitar"), tmp.join("stems/guitar.wav")).unwrap();
    let j = Job::open(&tmp).unwrap();
    let t0 = std::time::Instant::now();
    let p = pick_part(&j, 8.0, 28.0, Channel::Mid, "riff").unwrap();
    let dt = t0.elapsed().as_secs_f64();
    let py = src.read_parts().unwrap().into_iter().find(|p| p.id == "p2").unwrap();
    let matched = py.notes.iter().filter(|q| p.notes.iter().any(|n| n.midi == q.midi && (n.t - q.t).abs() <= 0.040)).count();
    let a = j.read_part_wav("p1", "target.wav").unwrap();
    let b = src.read_part_wav("p2", "target.wav").unwrap();
    let n = a.channels[0].len().min(b.channels[0].len());
    let err = (a.channels[0][..n].iter().zip(&b.channels[0][..n]).map(|(x, y)| ((x - y) as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
    let rms = (b.channels[0][..n].iter().map(|y| (*y as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
    println!("pick_part {dt:.2}s; notes {} vs {}, matched {}; target lens {} vs {}; rel err {:.4}", p.notes.len(), py.notes.len(), matched, a.channels[0].len(), b.channels[0].len(), err / rms);
    let _ = std::fs::remove_dir_all(tmp);
    assert!(matched as f64 / py.notes.len() as f64 >= 0.85);
}
