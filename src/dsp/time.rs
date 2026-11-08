use std::f32::consts::PI;

use serde::Deserialize;

use super::modulation::{Lfo, Wave};
use super::Block;

/// Linea di ritardo circolare con lettura frazionaria (interpolazione lineare).
pub struct DelayLine {
    buf: Vec<f32>,
    write: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        Self { buf: vec![0.0; max_samples.max(4)], write: 0 }
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) % self.buf.len();
    }

    /// Legge il campione di `delay` campioni fa (1.0 = l'ultimo scritto).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let delay = delay.clamp(1.0, (len - 2) as f32);
        let pos = self.write as f32 + len as f32 - delay;
        let i = pos as usize;
        let frac = pos - i as f32;
        let a = self.buf[i % len];
        let b = self.buf[(i + 1) % len];
        a + (b - a) * frac
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct DelayParams {
    pub time_ms: f32,
    pub feedback: f32,
    /// 0 = solo segnale pulito, 1 = solo eco.
    pub mix: f32,
    /// Le ripetizioni si scuriscono sopra questa frequenza (stile analogico).
    pub damp_hz: f32,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self { time_ms: 350.0, feedback: 0.35, mix: 0.3, damp_hz: 5000.0 }
    }
}

pub struct Delay {
    line: DelayLine,
    delay: f32,
    feedback: f32,
    mix: f32,
    lp: f32,
    lp_coef: f32,
}

impl Delay {
    pub fn new(p: &DelayParams, sr: f32) -> Self {
        let delay = (p.time_ms.clamp(1.0, 5000.0) * 0.001 * sr).max(1.0);
        Self {
            line: DelayLine::new(delay as usize + 4),
            delay,
            feedback: p.feedback.clamp(0.0, 0.98),
            mix: p.mix.clamp(0.0, 1.0),
            lp: 0.0,
            lp_coef: (-2.0 * PI * p.damp_hz.clamp(200.0, sr * 0.45) / sr).exp(),
        }
    }
}

impl Block for Delay {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let echo = self.line.read(self.delay);
            self.lp = echo + (self.lp - echo) * self.lp_coef;
            self.line.push(*s + self.lp * self.feedback);
            *s = *s * (1.0 - self.mix) + echo * self.mix;
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct ChorusParams {
    pub rate_hz: f32,
    /// Ritardo centrale in ms.
    pub base_ms: f32,
    /// Escursione della modulazione in ms (deve essere minore di base_ms).
    pub depth_ms: f32,
    pub mix: f32,
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self { rate_hz: 0.8, base_ms: 12.0, depth_ms: 3.0, mix: 0.5 }
    }
}

pub struct Chorus {
    line: DelayLine,
    lfo: Lfo,
    base: f32,
    depth: f32,
    mix: f32,
}

impl Chorus {
    pub fn new(p: &ChorusParams, sr: f32) -> Self {
        let base = p.base_ms.max(1.0) * 0.001 * sr;
        let depth = p.depth_ms.clamp(0.0, p.base_ms.max(1.0) * 0.95) * 0.001 * sr;
        Self {
            line: DelayLine::new((base + depth) as usize + 4),
            lfo: Lfo::new(p.rate_hz, Wave::Sine, sr),
            base,
            depth,
            mix: p.mix.clamp(0.0, 1.0),
        }
    }
}

impl Block for Chorus {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            self.line.push(*s);
            let wet = self.line.read(self.base + self.depth * self.lfo.next());
            *s = *s * (1.0 - self.mix) + wet * self.mix;
        }
    }
}
