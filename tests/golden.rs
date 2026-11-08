//! Golden render tests: every tracked preset is rendered over a deterministic synthetic
//! guitar signal and compared with a stored reference (`tests/golden/<preset>.f32`).
//!
//! Regenerate the references (only when a sound change is intended):
//! `OPENPEDAL_BLESS=1 cargo test --test golden`

use std::fs;
use std::path::{Path, PathBuf};

use openpedal::graph::model::PedalFile;
use openpedal::graph::{self, load};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
const SECONDS: f32 = 1.5;
const TOLERANCE: f32 = 1e-5;

/// Tiny deterministic PRNG (xorshift32) returning values in [-1, 1).
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / 2_147_483_648.0 - 1.0
    }
}

/// Adds a plucked note: decaying harmonics, 3 ms attack, a touch of pick noise.
fn pluck(out: &mut [f32], rng: &mut Rng, start: f32, freq: f32, len: f32, decay: f32, gain: f32) {
    let first = (start * SR) as usize;
    let n = ((len * SR) as usize).min(out.len().saturating_sub(first));
    for i in 0..n {
        let t = i as f32 / SR;
        let attack = (t / 0.003).min(1.0);
        let release = ((len - t) / 0.01).clamp(0.0, 1.0);
        let mut s = 0.0;
        for h in 1..=6 {
            let amp = 1.0 / h as f32;
            s += amp * (std::f32::consts::TAU * freq * h as f32 * t).sin() * (-t * decay * h as f32).exp();
        }
        let pick = rng.next() * 0.05 * (-t * 200.0).exp();
        out[first + i] += gain * attack * release * (s + pick);
    }
}

/// About -12 dBFS peak: single notes, a chord, palm-muted notes, silence in between.
fn guitar_signal() -> Vec<f32> {
    let mut rng = Rng(0x1234_5678);
    let mut x = vec![0.0f32; (SECONDS * SR) as usize];
    pluck(&mut x, &mut rng, 0.02, 110.0, 0.45, 4.0, 1.0);
    pluck(&mut x, &mut rng, 0.25, 164.81, 0.35, 5.0, 1.0);
    for f in [82.41, 123.47, 164.81, 207.65] {
        pluck(&mut x, &mut rng, 0.55, f, 0.40, 3.0, 0.6);
    }
    // Silence from 0.95 s to 1.05 s, then palm-muted short notes.
    for (k, f) in [110.0, 110.0, 146.83].into_iter().enumerate() {
        pluck(&mut x, &mut rng, 1.05 + 0.1 * k as f32, f, 0.07, 30.0, 1.0);
    }
    let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let scale = 0.25 / peak; // -12 dBFS
    x.iter_mut().for_each(|s| *s *= scale);
    x
}

fn render(preset: &Path, pedals: &[(String, PedalFile)], input: &[f32]) -> Vec<f32> {
    let g = load::read_graph(preset).unwrap_or_else(|e| panic!("{}: {e:#}", preset.display()));
    let (mut plan, _) = graph::compile(&g, pedals, SR).unwrap_or_else(|e| panic!("{}: {e:#}", preset.display()));
    let mut out = input.to_vec();
    for block in out.chunks_mut(BLOCK) {
        plan.process(block);
    }
    out
}

fn to_bytes(x: &[f32]) -> Vec<u8> {
    x.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn from_bytes(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

#[test]
fn presets_match_golden_renders() {
    let bless = std::env::var("OPENPEDAL_BLESS").is_ok_and(|v| v == "1");
    let golden_dir = PathBuf::from("tests/golden");
    let pedals = load::read_pedals(Path::new("pedals")).expect("read pedals");
    let input = guitar_signal();

    let mut presets: Vec<PathBuf> = fs::read_dir("presets")
        .expect("presets dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        // `test*` presets are local, untracked scratch files.
        .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with("test"))
        .collect();
    presets.sort();
    assert!(!presets.is_empty(), "no presets found");

    let mut failures = Vec::new();
    for path in &presets {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let out = render(path, &pedals, &input);

        // Sanity: finite and not silent.
        if let Some(i) = out.iter().position(|s| !s.is_finite()) {
            failures.push(format!("{name}: non-finite sample at index {i}"));
            continue;
        }
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak < 1e-4 {
            failures.push(format!("{name}: output is silent (peak {peak})"));
            continue;
        }

        let ref_path = golden_dir.join(format!("{name}.f32"));
        if bless {
            fs::create_dir_all(&golden_dir).unwrap();
            fs::write(&ref_path, to_bytes(&out)).unwrap();
            continue;
        }
        let Ok(bytes) = fs::read(&ref_path) else {
            failures.push(format!("{name}: missing reference {} (run with OPENPEDAL_BLESS=1)", ref_path.display()));
            continue;
        };
        let reference = from_bytes(&bytes);
        if reference.len() != out.len() {
            failures.push(format!("{name}: length {} != reference {}", out.len(), reference.len()));
            continue;
        }
        let mut max_diff = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut first_bad = None;
        for (i, (a, b)) in out.iter().zip(&reference).enumerate() {
            let d = (a - b).abs();
            if d > TOLERANCE && first_bad.is_none() {
                first_bad = Some(i);
            }
            max_diff = max_diff.max(d);
            sum_sq += (d as f64) * (d as f64);
        }
        let rms = (sum_sq / out.len() as f64).sqrt();
        println!("{name}: max diff {max_diff:.3e}, rms diff {rms:.3e}, peak {peak:.3}");
        if let Some(i) = first_bad {
            failures.push(format!(
                "{name}: first sample over tolerance at index {i} ({:.4} s), max diff {max_diff:.3e}, rms diff {rms:.3e}",
                i as f32 / SR
            ));
        }
    }
    assert!(failures.is_empty(), "golden render failures:\n  {}", failures.join("\n  "));
}
