use std::path::Path;

use openpedal_tonematch::features::{analyze, spectral_error};
use openpedal_tonematch::fit::{fit_signals, FitOptions};
use openpedal_tonematch::render::render;
use openpedal_tonematch::Template;

const SR: f32 = 48000.0;
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// Plucked strings (Karplus-Strong) on a small riff: a clean DI, 12 s.
fn plucked_di(seconds: f32) -> Vec<f32> {
    let n = (SR * seconds) as usize;
    let mut out = vec![0f32; n];
    let mut seed = 7u32;
    let notes = [82.4f32, 110.0, 98.0, 123.5, 146.8, 196.0, 164.8, 110.0];
    let step = (SR * 0.5) as usize;
    for (k, start) in (0..n).step_by(step).enumerate() {
        let hz = notes[k % notes.len()] * if k % 3 == 2 { 2.0 } else { 1.0 };
        let len = (SR / hz) as usize;
        let mut line: Vec<f32> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect();
        let mut i = 0;
        for o in out.iter_mut().take((start + step * 2).min(n)).skip(start) {
            let a = line[i];
            let b = line[(i + 1) % len];
            line[i] = 0.4985 * (a + b);
            *o += 0.08 * a;
            i = (i + 1) % len;
        }
    }
    out
}

#[test]
fn template_validation_rejects_bad_files() {
    let dir = Path::new(FIXTURES);
    assert!(Template::load(dir, "rec", &[]).is_ok());
    for (name, expect) in [("bad-key", "no parameter 'nope'"), ("bad-text", "not numeric"), ("bad-node", "no node 'ghost'")] {
        let err = format!("{:#}", Template::load(dir, name, &[]).unwrap_err());
        assert!(err.contains(expect), "{name}: {err}");
    }
}

/// Renders the fixture with known values, then fits it from the template's own (different) values.
/// Takes about 25 s in release (a few minutes in debug builds): run with `--release`.
#[test]
#[ignore = "slow in debug builds; run with `cargo test --release -p openpedal-tonematch`"]
fn recovers_a_known_chain() {
    recover(false);
}

#[test]
fn recovers_a_known_chain_with_holdout() {
    recover(true);
}

fn recover(holdout: bool) {
    let template = Template::load(Path::new(FIXTURES), "rec", &[]).unwrap();
    let di = plucked_di(if holdout { 8.0 } else { 10.0 });
    // gain_db, bass, treble, low_hz, high_hz: far from the template's start values.
    let truth = [15.0f32, 0.8, 0.3, 90.0, 4500.0];
    let target = render(&template.with_values(&truth), &[], &di, SR).unwrap();

    let start = render(&template.graph, &[], &di, SR).unwrap();
    let before = spectral_error(&analyze(&start, SR), &analyze(&target, SR));

    let opts = FitOptions { window_s: 6.0, ..FitOptions::default() };
    let fit = fit_signals(vec![template], &[], &di, &target, SR, &opts, holdout, &|_| {}).unwrap();
    let r = &fit.summary.report;
    eprintln!("before {before:.2} dB, after {:.2} dB, holdout {:?}, params {:?}", r.error_db, r.holdout_error_db, r.params);
    assert!(before > 2.0, "the start must be far from the target: {before}");
    assert!(r.error_db < 2.0, "fit error {}", r.error_db);
    assert_eq!(r.holdout_error_db.is_some(), holdout);
    if let Some(h) = r.holdout_error_db {
        assert!(h < 2.5, "holdout error {h}");
    }
}
