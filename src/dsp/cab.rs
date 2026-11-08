//! Speaker cabinet simulation: convolution with an impulse response (IR).
//!
//! The IR comes from a WAV file or is generated from the preset parameters:
//! the cabinet's response curve is drawn, turned into a minimum-phase filter
//! (like a real loudspeaker's) and applied as an FIR.

use std::f32::consts::PI;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, Block, Smoothed};
use crate::wav;

pub const CAB_DESC: BlockDesc = BlockDesc {
    kind: "cab",
    label: "Cab",
    category: Category::Amp,
    params: &[
        // If a file is given, the procedural parameters are ignored.
        ParamDesc::file("ir", "IR (WAV file)", &["wav"]),
        ParamDesc::number("max_ms", "Max IR length", 5.0, 500.0, 80.0, "ms"),
        ParamDesc::number("level_db", "Level", -24.0, 24.0, 0.0, "dB"),
        ParamDesc::log("low_hz", "Resonance", 40.0, 250.0, 100.0, "Hz"),
        ParamDesc::number("low_q", "Resonance Q", 0.5, 3.0, 1.3, ""),
        ParamDesc::log("high_hz", "High cut", 1500.0, 12000.0, 5000.0, "Hz"),
        ParamDesc::log("mid_hz", "Mid", 150.0, 2000.0, 500.0, "Hz"),
        ParamDesc::number("mid_db", "Mid gain", -12.0, 12.0, -3.0, "dB"),
        ParamDesc::log("presence_hz", "Presence", 1000.0, 6000.0, 2500.0, "Hz"),
        ParamDesc::number("presence_db", "Presence gain", -12.0, 12.0, 3.0, "dB"),
        ParamDesc::number("ripple_db", "Ripple", 0.0, 12.0, 3.0, "dB"),
        ParamDesc::int("seed", "Seed", 0.0, 9999.0, 1.0),
    ],
};

// Parameter indices in CAB_DESC. Only the level changes live: the others
// alter the impulse response, so the block must be rebuilt.
const LEVEL: usize = 2;

pub fn build_cab(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = CAB_DESC.params;
    let params = CabParams {
        ir: v.text(&p[0]).filter(|s| !s.is_empty()).map(str::to_string),
        max_ms: v.number(&p[1]),
        level_db: v.number(&p[2]),
        low_hz: v.number(&p[3]),
        low_q: v.number(&p[4]),
        high_hz: v.number(&p[5]),
        mid_hz: v.number(&p[6]),
        mid_db: v.number(&p[7]),
        presence_hz: v.number(&p[8]),
        presence_db: v.number(&p[9]),
        ripple_db: v.number(&p[10]),
        seed: v.number(&p[11]) as u32,
    };
    Ok(Box::new(Cab::new(&params, sr)?))
}

#[derive(Debug, Clone)]
pub struct CabParams {
    /// WAV file with the IR of a real cabinet. If set, the procedural parameters are ignored.
    pub ir: Option<String>,
    /// Maximum length of the IR from file: beyond that it is almost always just room reverb.
    pub max_ms: f32,
    pub level_db: f32,

    // Procedural cabinet.
    /// Cone resonance: below this frequency the bass falls off.
    pub low_hz: f32,
    /// How pronounced the resonance is (0.7 = flat, 2 = "boomy").
    pub low_q: f32,
    /// Above this frequency the highs fall off steeply (36 dB/octave).
    pub high_hz: f32,
    pub mid_hz: f32,
    /// Mids: negative = "scooped", positive = "nasal".
    pub mid_db: f32,
    pub presence_hz: f32,
    pub presence_db: f32,
    /// Cone irregularities: peaks and dips that make the cabinet sound "real".
    pub ripple_db: f32,
    /// Changes the layout of the irregularities: each seed is a different cabinet.
    pub seed: u32,
}

impl Default for CabParams {
    fn default() -> Self {
        Self {
            ir: None,
            max_ms: 80.0,
            level_db: 0.0,
            low_hz: 100.0,
            low_q: 1.3,
            high_hz: 5000.0,
            mid_hz: 500.0,
            mid_db: -3.0,
            presence_hz: 2500.0,
            presence_db: 3.0,
            ripple_db: 3.0,
            seed: 1,
        }
    }
}

/// Length of the procedural IR: 1024 samples (21 ms at 48 kHz) are enough,
/// because a minimum-phase filter concentrates the energy at the start.
const PROCEDURAL_LEN: usize = 1024;
const FFT_LEN: usize = 8192;

pub struct Cab {
    ir: Vec<f32>,
    /// Output level, smoothed: the only parameter that changes live.
    level: Smoothed,
    /// Sample history, stored twice (2 x len): the window to multiply
    /// with the IR is always contiguous, so the compiler can vectorize it.
    history: Vec<f32>,
    pos: usize,
}

impl Cab {
    pub fn new(p: &CabParams, sr: f32) -> Result<Self> {
        let mut ir = match &p.ir {
            Some(path) => load_ir(Path::new(path), sr, p.max_ms)?,
            None => procedural_ir(p, sr),
        };
        normalize(&mut ir, sr);
        let len = ir.len();
        Ok(Self {
            ir,
            level: Smoothed::new(db_to_lin(p.level_db), 15.0, sr),
            history: vec![0.0; len * 2],
            pos: 0,
        })
    }
}

impl Block for Cab {
    fn process(&mut self, buf: &mut [f32]) {
        let len = self.ir.len();
        for s in buf {
            // Writes backwards: history[pos + k] is the sample from k steps ago.
            self.pos = if self.pos == 0 { len - 1 } else { self.pos - 1 };
            self.history[self.pos] = *s;
            self.history[self.pos + len] = *s;
            *s = dot(&self.ir, &self.history[self.pos..self.pos + len]) * self.level.next();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        // The other indices (IR, length, procedural, seed) are ignored: they change
        // the IR, which is only recomputed when the block is rebuilt.
        if index == LEVEL {
            self.level.target = db_to_lin(value);
        }
    }
}

/// Dot product with 8 independent accumulators, so it becomes SIMD.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ac, ar) = a.split_at(a.len() / 8 * 8);
    let (bc, br) = b.split_at(ac.len());
    for (x, y) in ac.as_chunks::<8>().0.iter().zip(bc.as_chunks::<8>().0) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let tail: f32 = ar.iter().zip(br).map(|(x, y)| x * y).sum();
    acc.iter().sum::<f32>() + tail
}

fn load_ir(path: &Path, sr: f32, max_ms: f32) -> Result<Vec<f32>> {
    let data = wav::read(path).with_context(|| format!("cabinet IR: {}", path.display()))?;
    let src = &data.channels[0];
    ensure!(!src.is_empty(), "the IR {} is empty", path.display());

    // If the file has a different sample rate, resample it to the engine's rate.
    let ratio = data.sample_rate as f32 / sr;
    let max_len = ((max_ms.max(5.0) * 0.001 * sr) as usize).max(16);
    let out_len = ((src.len() as f32 / ratio) as usize).clamp(1, max_len);
    let mut ir: Vec<f32> = (0..out_len)
        .map(|i| {
            let pos = i as f32 * ratio;
            let j = pos as usize;
            let frac = pos - j as f32;
            let a = src[j.min(src.len() - 1)];
            let b = src[(j + 1).min(src.len() - 1)];
            a + (b - a) * frac
        })
        .collect();
    fade_out(&mut ir);
    Ok(ir)
}

/// Amplitude response (linear) of the cabinet drawn from the parameters.
fn target_magnitude(p: &CabParams, f: f32) -> f32 {
    let f = f.max(1.0);

    // 2nd-order resonant high-pass: the cone resonance in the cabinet.
    let r = f / p.low_hz;
    let hp = r * r / ((1.0 - r * r).powi(2) + (r / p.low_q.max(0.3)).powi(2)).sqrt();

    // 6th-order low-pass: above about 5 kHz the cone reproduces almost nothing.
    let lp = 1.0 / (1.0 + (f / p.high_hz).powi(12)).sqrt();

    let bump = |center: f32, db: f32, width_oct: f32| {
        let oct = (f / center).log2();
        db * (-(oct * oct) / (2.0 * width_oct * width_oct)).exp()
    };
    let mut db = bump(p.mid_hz, p.mid_db, 1.0) + bump(p.presence_hz, p.presence_db, 0.6);

    // Irregularities above 1 kHz, always the same for the same seed.
    let mut rng = p.seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        (rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    for _ in 0..7 {
        let center = 1000.0 * (p.high_hz * 1.6 / 1000.0).max(1.1).powf(0.5 + 0.5 * next());
        let amount = p.ripple_db * next();
        let width = 0.12 + 0.1 * next().abs();
        db += bump(center, amount, width);
    }

    hp * lp * 10f32.powf(db / 20.0)
}

/// Minimum-phase IR from the desired curve, using the cepstrum method.
fn procedural_ir(p: &CabParams, sr: f32) -> Vec<f32> {
    let n = FFT_LEN;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let ifft = planner.plan_fft_inverse(n);

    // 1. log of the magnitude over the whole spectrum (symmetric).
    let mut buf: Vec<Complex32> = (0..n)
        .map(|k| {
            let bin = if k <= n / 2 { k } else { n - k };
            let f = bin as f32 * sr / n as f32;
            Complex32::new(target_magnitude(p, f).max(1e-6).ln(), 0.0)
        })
        .collect();

    // 2. real cepstrum, "folded" so that it becomes causal (= minimum phase).
    ifft.process(&mut buf);
    let scale = 1.0 / n as f32;
    for (k, c) in buf.iter_mut().enumerate() {
        let w = match k {
            0 => 1.0,
            k if k < n / 2 => 2.0,
            k if k == n / 2 => 1.0,
            _ => 0.0,
        };
        *c = Complex32::new(c.re * scale * w, 0.0);
    }

    // 3. back to the frequency domain, exponentiate, then back to time.
    fft.process(&mut buf);
    for c in buf.iter_mut() {
        *c = c.exp();
    }
    ifft.process(&mut buf);

    let mut ir: Vec<f32> = buf[..PROCEDURAL_LEN].iter().map(|c| c.re * scale).collect();
    fade_out(&mut ir);
    ir
}

/// Fades out the last 10% of the IR, so the cut does not introduce artifacts.
fn fade_out(ir: &mut [f32]) {
    let fade = (ir.len() / 10).max(1);
    let start = ir.len() - fade;
    for (i, s) in ir[start..].iter_mut().enumerate() {
        *s *= 0.5 * (1.0 + (PI * i as f32 / fade as f32).cos());
    }
}

/// Brings the average gain between 300 Hz and 3 kHz, where the guitar plays, to 0 dB:
/// different IRs have different volumes, so switching them does not jump in level.
fn normalize(ir: &mut [f32], sr: f32) {
    let n = ir.len().next_power_of_two().max(4096);
    let mut buf: Vec<Complex32> = ir.iter().map(|&s| Complex32::new(s, 0.0)).collect();
    buf.resize(n, Complex32::new(0.0, 0.0));
    FftPlanner::<f32>::new().plan_fft_forward(n).process(&mut buf);

    let bin = |f: f32| (f * n as f32 / sr) as usize;
    let band = &buf[bin(300.0)..bin(3000.0)];
    let mean = band.iter().map(|c| c.norm()).sum::<f32>() / band.len() as f32;
    if mean > 1e-9 {
        for s in ir.iter_mut() {
            *s /= mean;
        }
    }
}

#[cfg(test)]
pub(crate) fn magnitude_at(ir: &[f32], f: f32, sr: f32) -> f32 {
    let w = 2.0 * PI * f / sr;
    let (re, im) = ir.iter().enumerate().fold((0.0f32, 0.0f32), |(re, im), (i, &s)| {
        (re + s * (w * i as f32).cos(), im - s * (w * i as f32).sin())
    });
    (re * re + im * im).sqrt()
}

#[cfg(test)]
impl Cab {
    pub(crate) fn ir(&self) -> &[f32] {
        &self.ir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn db_at(cab: &Cab, f: f32) -> f32 {
        20.0 * magnitude_at(cab.ir(), f, SR).log10()
    }

    #[test]
    fn procedural_cab_has_the_right_shape() {
        let cab = Cab::new(&CabParams::default(), SR).unwrap();
        let (sub, low, mid, high, top) =
            (db_at(&cab, 40.0), db_at(&cab, 100.0), db_at(&cab, 1000.0), db_at(&cab, 8000.0), db_at(&cab, 15000.0));
        println!("40 Hz {sub:.1}  100 Hz {low:.1}  1 kHz {mid:.1}  8 kHz {high:.1}  15 kHz {top:.1} dB");
        assert!(sub < low - 8.0, "the lowest bass must fall off");
        assert!(low > -3.0, "the bass resonance must be there");
        assert!(mid.abs() < 6.0, "the mids stay near 0 dB");
        assert!(high < mid - 15.0, "the highs must fall off");
        assert!(top < mid - 40.0, "the very high frequencies disappear");
    }

    #[test]
    fn different_seeds_give_different_cabs() {
        let a = Cab::new(&CabParams { seed: 1, ..Default::default() }, SR).unwrap();
        let b = Cab::new(&CabParams { seed: 2, ..Default::default() }, SR).unwrap();
        let diff: f32 = [1500.0, 2200.0, 3100.0, 4200.0].iter().map(|&f| (db_at(&a, f) - db_at(&b, f)).abs()).sum();
        assert!(diff > 1.0, "difference {diff}");
    }

    #[test]
    fn ir_file_with_other_sample_rate() {
        // A unit impulse at 44.1 kHz: the cabinet must become (almost) a wire.
        let dir = std::env::temp_dir().join(format!("openpedal-test-{}", std::process::id()));
        let path = dir.join("dirac.wav");
        let mut w = wav::WavWriter::create(&path, 1, 44100).unwrap();
        for i in 0..2000 {
            w.write(if i == 0 { 1.0 } else { 0.0 }).unwrap();
        }
        w.finish().unwrap();

        let p = CabParams { ir: Some(path.to_string_lossy().into()), ..Default::default() };
        let cab = Cab::new(&p, SR).unwrap();
        assert_eq!(cab.ir().len(), (2000.0 * SR / 44100.0) as usize);
        assert!(db_at(&cab, 1000.0).abs() < 0.5);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_file_is_a_clear_error() {
        let p = CabParams { ir: Some("non-esiste.wav".into()), ..Default::default() };
        let err = Cab::new(&p, SR).err().expect("must fail");
        assert!(format!("{err:#}").contains("non-esiste.wav"));
    }

    #[test]
    fn convolution_is_a_correct_fir() {
        let mut cab = Cab { ir: vec![0.5, 0.25, 0.125], level: Smoothed::new(1.0, 15.0, SR), history: vec![0.0; 6], pos: 0 };
        let mut buf = vec![1.0, 0.0, 0.0, 0.0, 2.0];
        cab.process(&mut buf);
        assert_eq!(buf, vec![0.5, 0.25, 0.125, 0.0, 1.0]);
    }

    #[test]
    fn set_param_changes_level_live() {
        let mut cab = Cab::new(&CabParams::default(), SR).unwrap();
        let sine = |from: usize, n: usize| -> Vec<f32> {
            (from..from + n).map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / SR).sin()).collect()
        };
        let mut buf = sine(0, 9600);
        cab.process(&mut buf);
        let before = buf[4800..].iter().fold(0.0f32, |m, s| m.max(s.abs()));

        // -20 dB: after the glide (15 ms) the peak is a tenth.
        cab.set_param(LEVEL, -20.0);
        let mut buf = sine(9600, 48000);
        cab.process(&mut buf);
        let after = buf[buf.len() - 4800..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((after / before - 0.1).abs() < 0.02, "ratio {}", after / before);
    }

    #[test]
    fn fast_enough_for_realtime() {
        // 80 ms IR (worst case from a file): 1 s of audio must cost much less than 1 s.
        let mut cab = Cab { ir: vec![0.001; 3840], level: Smoothed::new(1.0, 15.0, SR), history: vec![0.0; 7680], pos: 0 };
        let mut buf = vec![0.1f32; 48000];
        let t = std::time::Instant::now();
        for block in buf.chunks_mut(96) {
            cab.process(block);
        }
        let ms = t.elapsed().as_secs_f32() * 1000.0;
        println!("1 s of audio with a 3840-sample IR: {ms:.1} ms");
        assert!(ms < 300.0);
    }
}
