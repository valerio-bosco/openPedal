//! Needs the converted weights: OPENPEDAL_DEMUCS_WEIGHTS=/path/ggml-model-htdemucs-6s-f16.bin
//! (see docs/spikes/demucs-cpp.md). Run with `cargo test -p openpedal-separate --release -- --ignored`.
use std::sync::atomic::AtomicBool;

use openpedal_separate::{DemucsCpp, Separator};

fn corr(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        ab += (*x as f64) * (*y as f64);
        aa += (*x as f64).powi(2);
        bb += (*y as f64).powi(2);
    }
    ab / (aa.sqrt() * bb.sqrt() + 1e-30)
}

fn weights() -> DemucsCpp {
    let path = std::env::var("OPENPEDAL_DEMUCS_WEIGHTS").expect("set OPENPEDAL_DEMUCS_WEIGHTS to the weights file");
    DemucsCpp::load(path.as_ref()).unwrap()
}

/// 9 s of a decaying harmonic tone plus noise-like clicks (two segments), at 48 kHz (also tests resampling).
fn signal() -> (Vec<f32>, Vec<f32>) {
    let sr = 48_000.0f32;
    let l: Vec<f32> = (0..(9.0 * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            0.3 * (220.0 * std::f32::consts::TAU * t).sin() * (-(t % 1.0) * 3.0).exp() + if i % 12000 < 200 { 0.4 } else { 0.0 }
        })
        .collect();
    let r = l.iter().map(|x| x * 0.8).collect();
    (l, r)
}

#[test]
#[ignore = "needs OPENPEDAL_DEMUCS_WEIGHTS (52 MiB weights, never committed) and several GB of RAM"]
fn threads_match_single_thread() {
    let (l, r) = signal();
    let cancel = AtomicBool::new(false);
    let m = weights().with_threads(1);
    let one = m.separate(&l, &r, 48_000, &mut |_, _| {}, &cancel).unwrap();
    let four = m.with_threads(4).separate(&l, &r, 48_000, &mut |_, _| {}, &cancel).unwrap();
    assert_eq!(one.stems.len(), 6);
    assert_eq!(one.stems[0].1[0].len(), l.len());
    for ((n, a), (_, b)) in one.stems.iter().zip(&four.stems) {
        let c = corr(&a[0], &b[0]);
        assert!(c > 0.999 || a[0].iter().all(|x| x.abs() < 1e-6), "{n}: {c}");
    }
}

#[test]
#[ignore = "needs OPENPEDAL_DEMUCS_WEIGHTS"]
fn cancel_aborts() {
    let (l, r) = signal();
    let cancel = AtomicBool::new(true);
    let err = weights().separate(&l, &r, 48_000, &mut |_, _| {}, &cancel).err().expect("must fail");
    assert!(err.to_string().contains("cancelled"));
}
