//! Offline frequency-response analysis of an effect (docs/PLAN.md §12.2).
//!
//! The effect is compiled into a standalone plan (`input -> effect -> output`) at the engine's
//! sample rate, a low-level impulse (-30 dBFS) goes through it, and the FFT of what comes out,
//! relative to the input, is the magnitude response. This runs on a control/server thread and
//! never touches the playing plan, so nothing here is real-time code.
//!
//! The impulse is small enough for drives to stay mostly in their linear region only when the
//! gain is moderate: the curve is the small-signal response at that level, which is what a
//! player expects from a "what does this filter/tone do" picture. Time-based effects show their
//! comb or tail structure after smoothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use serde::Serialize;

use crate::compile::compile_with;
use crate::model::{EdgeSpec, GraphFile, NodeSpec, PedalFile, INPUT, OUTPUT, PEDAL_PREFIX};
use crate::registry::{EffectDesc, Registry};
use openpedal_dsp::params::{ParamValues, Value};

/// Number of log-spaced points in a response.
pub const POINTS: usize = 128;
/// Lowest and highest frequency of a response, in Hz.
pub const F_MIN: f32 = 20.0;
pub const F_MAX: f32 = 20_000.0;
/// Level of the test impulse.
const IMPULSE_DBFS: f32 = -30.0;
/// Half-width of the smoothing window, in octaves (the whole window is a sixth of an octave).
const SMOOTH_OCTAVES: f32 = 1.0 / 12.0;
/// Id of the draft pedal inside the analysis registry.
const DRAFT: &str = "__draft";

/// A magnitude response: `db[i]` is the gain at `freq[i]` Hz, relative to the input.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    pub freq: Vec<f32>,
    pub db: Vec<f32>,
}

/// The response of a node of a graph with its current values (`pedals` are the available
/// composite pedals).
pub fn analyze_node(node: &NodeSpec, pedals: &[(String, PedalFile)], sample_rate: f32) -> Result<Response> {
    if node.kind == INPUT || node.kind == OUTPUT {
        return Err(anyhow!("'{}' ({}) has no response to show: it is not an effect", node.id, node.kind));
    }
    let registry = Registry::new(pedals);
    let mut spec = node.clone();
    spec.id = "fx".into();
    spec.bypass = false;
    spec.pos = [0.0, 0.0];
    analyze_spec(&registry, spec, sample_rate)
}

/// The response of a pedal file, possibly an unsaved draft, with the given control values
/// (knob position 0..1, or option index; missing controls take their default).
pub fn analyze_pedal(
    pedal: &PedalFile,
    values: &BTreeMap<String, f32>,
    others: &[(String, PedalFile)],
    sample_rate: f32,
) -> Result<Response> {
    let registry = Registry::new(others).with_effect(EffectDesc::from_pedal(DRAFT, pedal));
    let kind = format!("{PEDAL_PREFIX}{DRAFT}");
    let effect = registry.get(&kind).expect("just registered");
    registry.check_effect(effect).map_err(|m| anyhow!(m))?;
    let mut params = ParamValues::default();
    for (id, v) in values {
        if effect.control(id).is_none() {
            let valid: Vec<_> = effect.controls.iter().map(|c| c.id.as_str()).collect();
            return Err(anyhow!("the pedal has no control '{id}' (controls: {})", valid.join(", ")));
        }
        params.set(id, Value::Number(*v));
    }
    analyze_spec(&registry, NodeSpec { id: "fx".into(), kind, pos: [0.0, 0.0], params, bypass: false }, sample_rate)
}

fn analyze_spec(registry: &Registry, node: NodeSpec, sr: f32) -> Result<Response> {
    let plain = |id: &str, kind: &str| NodeSpec { id: id.into(), kind: kind.into(), pos: [0.0, 0.0], params: ParamValues::default(), bypass: false };
    let graph = GraphFile {
        name: "analysis".into(),
        version: 2,
        nodes: vec![plain("in", INPUT), node, plain("out", OUTPUT)],
        edges: vec![EdgeSpec { from: "in".into(), to: "fx".into() }, EdgeSpec { from: "fx".into(), to: "out".into() }],
    };
    let (mut plan, _) = compile_with(&graph, registry, sr)?;

    // Long enough for the lowest frequencies and a decay: about 0.68 s.
    let n = ((sr * 0.68) as usize).next_power_of_two().max(1024);
    let amplitude = 10f32.powf(IMPULSE_DBFS / 20.0);
    let mut signal = vec![0.0f32; n];
    signal[0] = amplitude;
    plan.process(&mut signal);

    let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
    Ok(response_from_impulse(&signal, amplitude, sr, fft))
}

fn response_from_impulse(output: &[f32], amplitude: f32, sr: f32, fft: Arc<dyn rustfft::Fft<f32>>) -> Response {
    let n = output.len();
    let mut spectrum: Vec<Complex<f32>> = output.iter().map(|&s| Complex::new(s, 0.0)).collect();
    fft.process(&mut spectrum);
    // Power relative to the input's (a flat spectrum of `amplitude`).
    let power: Vec<f32> = spectrum[..n / 2 + 1].iter().map(|c| c.norm_sqr() / (amplitude * amplitude)).collect();

    let f_max = F_MAX.min(0.45 * sr);
    let per_bin = n as f32 / sr;
    let width = 2f32.powf(SMOOTH_OCTAVES);
    let mut freq = Vec::with_capacity(POINTS);
    let mut db = Vec::with_capacity(POINTS);
    for i in 0..POINTS {
        let f = F_MIN * (f_max / F_MIN).powf(i as f32 / (POINTS - 1) as f32);
        let pos = f * per_bin;
        let (lo, hi) = ((pos / width).ceil() as usize, ((pos * width).floor() as usize).min(power.len() - 1));
        let p = if hi >= lo {
            power[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
        } else {
            // No bin inside the window (very low frequencies): interpolate between the neighbours.
            let (a, b) = (pos.floor() as usize, (pos.ceil() as usize).min(power.len() - 1));
            power[a] + (power[b] - power[a]) * (pos - a as f32)
        };
        freq.push(f);
        db.push((10.0 * p.max(1e-12).log10()).max(-120.0));
    }
    Response { freq, db }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const SR: f32 = 48_000.0;

    fn pedal(text: &str) -> PedalFile {
        toml::from_str(text).unwrap()
    }

    /// The response at the point nearest to `hz`.
    fn at(r: &Response, hz: f32) -> f32 {
        let i = r.freq.iter().enumerate().min_by(|a, b| (a.1.ln() - hz.ln()).abs().total_cmp(&(b.1.ln() - hz.ln()).abs())).unwrap().0;
        r.db[i]
    }

    const LOWPASS: &str = r#"
name = "LP"
[[node]]
id = "f"
type = "filter"
params = { kind = "lowpass", freq = 1000, q = 0.707 }
[[edge]]
from = "pedal_in"
to = "f"
[[edge]]
from = "f"
to = "pedal_out"
[[control]]
id = "cut"
kind = "knob"
default = 0.5
[[control.route]]
target = "f.freq"
map = "log"
min = 200
max = 5000
"#;

    #[test]
    fn grid_is_log_spaced_from_20_hz_to_20_khz() {
        let r = analyze_pedal(&pedal(LOWPASS), &BTreeMap::new(), &[], SR).unwrap();
        assert_eq!((r.freq.len(), r.db.len()), (POINTS, POINTS));
        assert!((r.freq[0] - 20.0).abs() < 1e-3 && (r.freq[POINTS - 1] - 20_000.0).abs() < 1.0);
        let ratios: Vec<f32> = r.freq.windows(2).map(|w| w[1] / w[0]).collect();
        assert!(ratios.iter().all(|q| (q - ratios[0]).abs() < 1e-3));
        assert!(r.db.iter().all(|d| d.is_finite()));
    }

    #[test]
    fn a_lowpass_pedal_rolls_off_above_its_cutoff() {
        // cut = 0.5 on a log map 200..5000 Hz -> 1000 Hz
        let r = analyze_pedal(&pedal(LOWPASS), &BTreeMap::new(), &[], SR).unwrap();
        assert!(at(&r, 100.0).abs() < 0.5, "passband {}", at(&r, 100.0));
        assert!((at(&r, 1000.0) + 3.0).abs() < 1.0, "corner {}", at(&r, 1000.0));
        let (one_oct, two_oct) = (at(&r, 2000.0), at(&r, 4000.0));
        assert!(one_oct < -9.0 && two_oct < one_oct - 8.0, "second-order slope: {one_oct} {two_oct}");
        assert!(at(&r, 15_000.0) < -35.0);
        // moving the control moves the corner
        let open = analyze_pedal(&pedal(LOWPASS), &BTreeMap::from([("cut".to_string(), 1.0)]), &[], SR).unwrap();
        assert!(at(&open, 3000.0) > -3.0 && at(&r, 3000.0) < -15.0);
    }

    #[test]
    fn a_gain_block_is_a_flat_line_at_its_gain() {
        let node = NodeSpec { id: "g".into(), kind: "gain".into(), pos: [0.0, 0.0], params: toml::from_str("db = 6.0").unwrap(), bypass: false };
        let r = analyze_node(&node, &[], SR).unwrap();
        assert!(r.db.iter().all(|d| (d - 6.0).abs() < 0.05), "{:?}", r.db);
    }

    #[test]
    fn unknown_controls_and_bad_drafts_are_clear_errors() {
        let e = analyze_pedal(&pedal(LOWPASS), &BTreeMap::from([("nope".to_string(), 1.0)]), &[], SR).unwrap_err();
        assert!(format!("{e}").contains("no control 'nope'"), "{e}");
        let bad = LOWPASS.replace("f.freq", "f.nothing");
        let e = analyze_pedal(&pedal(&bad), &BTreeMap::new(), &[], SR).unwrap_err();
        assert!(format!("{e}").contains("no parameter 'nothing'"), "{e}");
    }

    #[test]
    fn the_example_pedal_is_analysed_fast_and_its_controls_matter() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/pedals/grit-box.toml");
        let grit = pedal(&std::fs::read_to_string(path).unwrap());
        let start = Instant::now();
        let normal = analyze_pedal(&grit, &BTreeMap::new(), &[], SR).unwrap();
        let took = start.elapsed();
        assert!(took.as_millis() < 500, "took {took:?}");
        // the highpass inside tightens the lows; the lowpass cuts the top
        assert!(at(&normal, 30.0) < at(&normal, 500.0) - 6.0);
        assert!(at(&normal, 18_000.0) < at(&normal, 1000.0) - 6.0);
        // a brighter Tone raises the top end
        let bright = analyze_pedal(&grit, &BTreeMap::from([("tone".to_string(), 1.0)]), &[], SR).unwrap();
        assert!(at(&bright, 8000.0) > at(&normal, 8000.0) + 1.0);
        eprintln!("grit-box analysis: {took:?}");
    }
}
