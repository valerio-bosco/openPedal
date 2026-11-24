use openpedal_tonematch::features::{analyze, band_centres, crest_db};
use openpedal_tonematch::BANDS;
use rustfft::{num_complex::Complex, FftPlanner};

const SR: f32 = 48000.0;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }
}

fn white(n: usize) -> Vec<f32> {
    let mut r = Lcg(42);
    (0..n).map(|_| r.next()).collect()
}

/// White noise shaped to a 1/f power spectrum.
fn pink(n: usize) -> Vec<f32> {
    let mut spec: Vec<Complex<f32>> = white(n).into_iter().map(|v| Complex::new(v, 0.0)).collect();
    let mut planner = FftPlanner::new();
    planner.plan_fft_forward(n).process(&mut spec);
    for (k, c) in spec.iter_mut().enumerate() {
        let kk = k.min(n - k).max(1) as f32;
        *c *= 1.0 / kk.sqrt();
    }
    planner.plan_fft_inverse(n).process(&mut spec);
    spec.iter().map(|c| c.re / n as f32).collect()
}

fn slope_db_per_band(bands: &[f32; BANDS]) -> f32 {
    let xs: Vec<f32> = (0..BANDS).map(|i| i as f32).collect();
    let (mx, my) = (xs.iter().sum::<f32>() / BANDS as f32, bands.iter().sum::<f32>() / BANDS as f32);
    let num: f32 = xs.iter().zip(bands).map(|(x, y)| (x - mx) * (y - my)).sum();
    let den: f32 = xs.iter().map(|x| (x - mx) * (x - mx)).sum();
    num / den
}

#[test]
fn band_centres_span_40_hz_to_10_khz() {
    let c = band_centres();
    assert_eq!(c.len(), 25);
    assert!((c[0] - 39.4).abs() < 0.5 && (c[14] - 1000.0).abs() < 1e-3 && (c[24] - 10079.0).abs() < 2.0);
}

#[test]
fn white_noise_rises_one_db_per_third_octave_and_pink_is_flat() {
    let w = analyze(&white(48000 * 20), SR);
    let s = slope_db_per_band(&w.bands_db);
    assert!((s - 1.0).abs() < 0.15, "white slope {s}");
    let p = analyze(&pink(1 << 20), SR);
    let s = slope_db_per_band(&p.bands_db);
    assert!(s.abs() < 0.15, "pink slope {s}");
}

#[test]
fn a_sine_lands_in_its_band() {
    for (hz, band) in [(1000.0f32, 14usize), (100.0, 4), (4000.0, 20)] {
        let x: Vec<f32> = (0..48000 * 3).map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / SR).sin()).collect();
        let f = analyze(&x, SR);
        let top = (0..BANDS).max_by(|a, b| f.bands_db[*a].total_cmp(&f.bands_db[*b])).unwrap();
        assert_eq!(top, band, "{hz} Hz");
        assert_eq!(f.bands_db[top], 0.0);
        let others = (0..BANDS).filter(|b| *b != top).map(|b| f.bands_db[b]).fold(f32::MIN, f32::max);
        assert!(others < -30.0, "{hz} Hz: leakage {others} dB");
    }
}

#[test]
fn crest_of_a_sine_is_3_db() {
    let x: Vec<f32> = (0..48000).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 997.0 * i as f32 / SR).sin()).collect();
    assert!((crest_db(&x) - 3.01).abs() < 0.02, "{}", crest_db(&x));
}
