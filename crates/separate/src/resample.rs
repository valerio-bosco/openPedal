//! Windowed-sinc resampler (Kaiser window, 32 zero crossings per side, polyphase table for rational ratios).
//! Linear phase, no latency: output sample n sits at input time n * from / to.

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..50 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
        if term < 1e-14 * sum {
            break;
        }
    }
    sum
}

const HALF: i64 = 32;
const BETA: f64 = 9.0;

/// Low-pass sinc with cutoff `fc` (fraction of the input Nyquist), evaluated at `x` input samples from centre.
fn kernel(x: f64, fc: f64) -> f64 {
    let t = x / HALF as f64;
    if t.abs() >= 1.0 {
        return 0.0;
    }
    let w = bessel_i0(BETA * (1.0 - t * t).sqrt()) / bessel_i0(BETA);
    let a = std::f64::consts::PI * x * fc;
    let s = if a.abs() < 1e-12 { 1.0 } else { a.sin() / a };
    fc * s * w
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Resamples `x` from `from` Hz to `to` Hz.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let g = gcd(from as u64, to as u64);
    let (num, den) = ((from as u64 / g) as i64, (to as u64 / g) as i64); // input samples per output = num/den
    let out_len = (x.len() as u64 * to as u64 / from as u64) as usize;
    // Downsampling: the cutoff drops to the new Nyquist and the kernel widens by 1/fc.
    let fc = (to as f64 / from as f64).min(1.0);
    let half = (HALF as f64 / fc).ceil() as i64;
    let taps = |frac: f64| -> Vec<f32> { (-half + 1..=half).map(|j| kernel(j as f64 - frac, fc) as f32).collect() };
    // One filter per phase when there are few phases (44.1k <-> 48k has 147/160), else computed per sample.
    let table: Option<Vec<Vec<f32>>> = (den <= 2048).then(|| (0..den).map(|q| taps(q as f64 / den as f64)).collect());
    let n = x.len() as i64;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len as i64 {
        let pos = i * num;
        let centre = pos.div_euclid(den);
        let q = pos.rem_euclid(den);
        let owned;
        let t: &[f32] = match &table {
            Some(t) => &t[q as usize],
            None => {
                owned = taps(q as f64 / den as f64);
                &owned
            }
        };
        let mut acc = 0f64;
        for (j, &w) in (-half + 1..=half).zip(t) {
            let k = centre + j;
            if (0..n).contains(&k) {
                acc += x[k as usize] as f64 * w as f64;
            }
        }
        out.push(acc as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f64, sr: u32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / sr as f64).sin() as f32).collect()
    }

    fn rms_err(a: &[f32], b: &[f32], skip: usize) -> f64 {
        let n = a.len().min(b.len()) - 2 * skip;
        ((skip..skip + n).map(|i| ((a[i] - b[i]) as f64).powi(2)).sum::<f64>() / n as f64).sqrt()
    }

    #[test]
    fn sine_survives_both_directions() {
        for (from, to) in [(48_000, 44_100), (44_100, 48_000), (22_050, 44_100), (96_000, 44_100)] {
            let x = sine(1000.0, from, from as usize);
            let y = resample(&x, from, to);
            assert!((y.len() as i64 - to as i64).abs() <= 1);
            let want = sine(1000.0, to, y.len());
            assert!(rms_err(&y, &want, 2000) < 2e-3, "{from}->{to}");
        }
    }
}
