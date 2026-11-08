//! Tone stack da ampli: bassi, medi, alti interagenti (task D4, vedi docs/PLAN.md §7).
//!
//! Modello TMB di D. T. Yeh (filtro analogico del 3° ordine), discretizzato con la
//! trasformazione bilineare. I potenziometri dei medi e degli alti sono lineari,
//! quello dei bassi ha una legge logaritmica approssimata (`l = bass²`).
//! Il guadagno è normalizzato a 1 a 1 kHz con tutte le manopole a 0.5.

use std::f64::consts::TAU;

use anyhow::Result;

use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::Block;

pub const TONESTACK_DESC: BlockDesc = BlockDesc {
    kind: "tonestack",
    label: "Tone stack",
    category: Category::Amp,
    params: &[
        ParamDesc::number("bass", "Bassi", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("mid", "Medi", 0.0, 1.0, 0.5, ""),
        ParamDesc::number("treble", "Alti", 0.0, 1.0, 0.5, ""),
        ParamDesc::choice("model", "Modello", &["fender", "marshall", "vox"], 0),
    ],
};

const BASS: usize = 0;
const MID: usize = 1;
const TREBLE: usize = 2;
const MODEL: usize = 3;

/// Passo di ricalcolo dei coefficienti (campioni).
const STEP: usize = 32;

/// Componenti di un modello: R1 (alti), R2 (bassi), R3 (medi), R4, C1, C2, C3.
struct Components {
    r1: f64,
    r2: f64,
    r3: f64,
    r4: f64,
    c1: f64,
    c2: f64,
    c3: f64,
}

const MODELS: [Components; 3] = [
    // Fender Bassman
    Components { r1: 250e3, r2: 1e6, r3: 25e3, r4: 56e3, c1: 250e-12, c2: 20e-9, c3: 20e-9 },
    // Marshall JCM800
    Components { r1: 220e3, r2: 1e6, r3: 25e3, r4: 33e3, c1: 470e-12, c2: 22e-9, c3: 22e-9 },
    // Vox AC30
    Components { r1: 1e6, r2: 1e6, r3: 10e3, r4: 100e3, c1: 50e-12, c2: 22e-9, c3: 22e-9 },
];

/// Coefficienti analogici H(s) = (b1 s + b2 s² + b3 s³) / (1 + a1 s + a2 s² + a3 s³).
fn analog(c: &Components, l: f64, m: f64, t: f64) -> ([f64; 3], [f64; 3]) {
    let Components { r1, r2, r3, r4, c1, c2, c3 } = *c;
    let b1 = t * c1 * r1 + m * c3 * r3 + l * (c1 * r2 + c2 * r2) + (c1 * r3 + c2 * r3);
    let b2 = t * (c1 * c2 * r1 * r4 + c1 * c3 * r1 * r4)
        - m * m * (c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + m * (c1 * c3 * r1 * r3 + c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * (c1 * c2 * r1 * r2 + c1 * c2 * r2 * r4 + c1 * c3 * r2 * r4)
        + l * m * (c1 * c3 * r2 * r3 + c2 * c3 * r2 * r3)
        + (c1 * c2 * r1 * r3 + c1 * c2 * r3 * r4 + c1 * c3 * r3 * r4);
    let b3 = l * m * (c1 * c2 * c3 * r1 * r2 * r3 + c1 * c2 * c3 * r2 * r3 * r4)
        - m * m * (c1 * c2 * c3 * r1 * r3 * r4 + c1 * c2 * c3 * r3 * r3 * r4)
        + m * (c1 * c2 * c3 * r1 * r3 * r3 + c1 * c2 * c3 * r3 * r3 * r4)
        + t * c1 * c2 * c3 * r1 * r3 * r4
        - t * m * c1 * c2 * c3 * r1 * r3 * r4
        + t * l * c1 * c2 * c3 * r1 * r2 * r4;
    let a1 = c1 * r1 + c1 * r3 + c2 * r3 + c2 * r4 + c3 * r4 + m * c3 * r3 + l * (c1 * r2 + c2 * r2);
    let a2 = m * (c1 * c3 * r1 * r3 - c2 * c3 * r3 * r4 + c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * m * (c1 * c3 * r2 * r3 + c2 * c3 * r2 * r3)
        - m * m * (c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * (c1 * c2 * r2 * r4 + c1 * c2 * r1 * r2 + c1 * c3 * r2 * r4 + c2 * c3 * r2 * r4)
        + (c1 * c2 * r1 * r4 + c1 * c3 * r1 * r4 + c1 * c2 * r3 * r4 + c1 * c2 * r1 * r3 + c1 * c3 * r3 * r4 + c2 * c3 * r3 * r4);
    let a3 = l * m * (c1 * c2 * c3 * r1 * r2 * r3 + c1 * c2 * c3 * r2 * r3 * r4)
        - m * m * (c1 * c2 * c3 * r1 * r3 * r4 + c1 * c2 * c3 * r3 * r3 * r4)
        + m * (c1 * c2 * c3 * r3 * r3 * r4 + c1 * c2 * c3 * r1 * r3 * r4)
        + l * c1 * c2 * c3 * r1 * r2 * r4
        + c1 * c2 * c3 * r1 * r3 * r4;
    ([b1, b2, b3], [a1, a2, a3])
}

/// Trasformazione bilineare s = 2 sr (1 - z⁻¹)/(1 + z⁻¹): ritorna (b[0..4], a[0..4]) con a[0] = 1.
fn bilinear(b: [f64; 3], a: [f64; 3], sr: f64) -> ([f64; 4], [f64; 4]) {
    let k = 2.0 * sr;
    // (1 - z⁻¹)^i (1 + z⁻¹)^(3-i) per i = 0..3, in potenze di z⁻¹
    const P: [[f64; 4]; 4] = [[1.0, 3.0, 3.0, 1.0], [1.0, 1.0, -1.0, -1.0], [1.0, -1.0, -1.0, 1.0], [1.0, -3.0, 3.0, -1.0]];
    let num = [0.0, b[0] * k, b[1] * k * k, b[2] * k * k * k];
    let den = [1.0, a[0] * k, a[1] * k * k, a[2] * k * k * k];
    let (mut nb, mut da) = ([0.0; 4], [0.0; 4]);
    for i in 0..4 {
        for j in 0..4 {
            nb[j] += num[i] * P[i][j];
            da[j] += den[i] * P[i][j];
        }
    }
    let a0 = da[0];
    ([nb[0] / a0, nb[1] / a0, nb[2] / a0, nb[3] / a0], [1.0, da[1] / a0, da[2] / a0, da[3] / a0])
}

/// Valori dei potenziometri (bassi, medi, alti) per le posizioni delle manopole 0..1.
fn pots(bass: f64, mid: f64, treble: f64) -> (f64, f64, f64) {
    (bass * bass, mid, treble)
}

/// Modulo di H(s) analogico a `freq` (usato solo per normalizzare il guadagno).
fn analog_gain(c: &Components, l: f64, m: f64, t: f64, freq: f64) -> f64 {
    let (b, a) = analog(c, l, m, t);
    let w = TAU * freq;
    // s = jw: s² = -w², s³ = -j w³
    let nr = -b[1] * w * w;
    let ni = b[0] * w - b[2] * w * w * w;
    let dr = 1.0 - a[1] * w * w;
    let di = a[0] * w - a[2] * w * w * w;
    ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
}

pub fn build_tonestack(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = TONESTACK_DESC.params;
    let bass = v.number(&p[BASS]);
    let mid = v.number(&p[MID]);
    let treble = v.number(&p[TREBLE]);
    let mut ts = ToneStack {
        sr: sr as f64,
        model: v.choice(&p[MODEL]).min(2),
        cur: [bass, mid, treble],
        target: [bass, mid, treble],
        smooth: 1.0 - (-(STEP as f64) / (0.015 * sr as f64)).exp(),
        makeup: 1.0,
        b: [0.0; 4],
        a: [1.0, 0.0, 0.0, 0.0],
        z: [0.0; 3],
    };
    ts.update_model();
    Ok(Box::new(ts))
}

pub struct ToneStack {
    sr: f64,
    model: usize,
    /// valori attuali e obiettivo di bass, mid, treble (smussati a blocchi)
    cur: [f32; 3],
    target: [f32; 3],
    smooth: f64,
    makeup: f64,
    b: [f64; 4],
    a: [f64; 4],
    z: [f64; 3],
}

impl ToneStack {
    /// Cambio di modello: ricalcola la normalizzazione e i coefficienti.
    fn update_model(&mut self) {
        let c = &MODELS[self.model];
        let (l, m, t) = pots(0.5, 0.5, 0.5);
        self.makeup = 1.0 / analog_gain(c, l, m, t, 1000.0);
        self.update_coefs();
    }

    fn update_coefs(&mut self) {
        let (l, m, t) = pots(self.cur[0] as f64, self.cur[1] as f64, self.cur[2] as f64);
        let (b, a) = analog(&MODELS[self.model], l, m, t);
        let (mut bd, ad) = bilinear(b, a, self.sr);
        for x in &mut bd {
            *x *= self.makeup;
        }
        self.b = bd;
        self.a = ad;
    }
}

impl Block for ToneStack {
    fn process(&mut self, buf: &mut [f32]) {
        for chunk in buf.chunks_mut(STEP) {
            if self.cur != self.target {
                for i in 0..3 {
                    let d = self.target[i] - self.cur[i];
                    self.cur[i] = if d.abs() < 1e-4 { self.target[i] } else { self.cur[i] + d * self.smooth as f32 };
                }
                self.update_coefs();
            }
            let (b, a) = (self.b, self.a);
            for s in chunk {
                // forma trasposta II
                let x = *s as f64;
                let y = b[0] * x + self.z[0];
                self.z[0] = b[1] * x - a[1] * y + self.z[1];
                self.z[1] = b[2] * x - a[2] * y + self.z[2];
                self.z[2] = b[3] * x - a[3] * y;
                *s = y as f32;
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            BASS | MID | TREBLE => self.target[index] = value.clamp(0.0, 1.0),
            MODEL => {
                let m = (value.round().max(0.0) as usize).min(2);
                if m != self.model {
                    self.model = m;
                    self.update_model();
                }
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.z = [0.0; 3];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::Value;

    const SR: f32 = 48000.0;

    fn make(model: usize, bass: f32, mid: f32, treble: f32) -> Box<dyn Block> {
        let mut v = ParamValues::default();
        v.set("model", Value::Number(model as f32));
        v.set("bass", Value::Number(bass));
        v.set("mid", Value::Number(mid));
        v.set("treble", Value::Number(treble));
        build_tonestack(&v, SR).unwrap()
    }

    /// Guadagno in dB a `freq` misurato con un seno.
    fn gain_db(model: usize, bass: f32, mid: f32, treble: f32, freq: f32) -> f32 {
        let mut ts = make(model, bass, mid, treble);
        let n = 24000;
        let mut buf: Vec<f32> = (0..n).map(|i| 0.1 * (std::f32::consts::TAU * freq * i as f32 / SR).sin()).collect();
        ts.process(&mut buf);
        let peak = buf[n / 2..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        20.0 * (peak / 0.1).log10()
    }

    #[test]
    fn i_bassi_si_sentono() {
        for (model, name) in ["fender", "marshall", "vox"].iter().enumerate() {
            let low = gain_db(model, 0.0, 0.5, 0.5, 100.0);
            let high = gain_db(model, 1.0, 0.5, 0.5, 100.0);
            println!("{name}: 100 Hz bass=0 {low:.1} dB, bass=1 {high:.1} dB");
            assert!(high - low > 6.0, "{name}: {low} vs {high}");
        }
    }

    #[test]
    fn guadagno_normalizzato_a_1khz() {
        for model in 0..3 {
            let g = gain_db(model, 0.5, 0.5, 0.5, 1000.0);
            assert!(g.abs() < 0.3, "modello {model}: {g} dB");
        }
    }

    #[test]
    fn gli_alti_si_sentono() {
        let low = gain_db(0, 0.5, 0.5, 0.0, 5000.0);
        let high = gain_db(0, 0.5, 0.5, 1.0, 5000.0);
        println!("fender: 5 kHz treble=0 {low:.1} dB, treble=1 {high:.1} dB");
        assert!(high - low > 6.0);
    }

    #[test]
    fn stabile_per_ogni_combinazione() {
        let vals = [0.0, 0.5, 1.0];
        for model in 0..3 {
            for &b in &vals {
                for &m in &vals {
                    for &t in &vals {
                        let mut ts = make(model, b, m, t);
                        let mut buf: Vec<f32> = (0..96000).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect();
                        ts.process(&mut buf);
                        let tail = buf[90000..].iter().fold(0.0f32, |mx, s| mx.max(s.abs()));
                        assert!(buf.iter().all(|s| s.is_finite()), "{model} {b} {m} {t}");
                        assert!(tail < 1e-4, "non decade: {model} {b} {m} {t} coda {tail}");
                    }
                }
            }
        }
    }

    #[test]
    fn cambio_dal_vivo_senza_salti() {
        let mut ts = make(0, 0.5, 0.5, 0.5);
        let mut buf: Vec<f32> = (0..9600).map(|i| 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin()).collect();
        let (a, b) = buf.split_at_mut(4800);
        ts.process(a);
        ts.set_param(BASS, 1.0);
        ts.set_param(TREBLE, 0.0);
        ts.process(b);
        let mut jump = 0.0f32;
        for w in buf[4000..].windows(2) {
            jump = jump.max((w[1] - w[0]).abs());
        }
        assert!(jump < 0.2, "salto {jump}");
    }
}
