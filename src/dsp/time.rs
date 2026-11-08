use std::f32::consts::PI;

use anyhow::Result;

use super::modulation::{Lfo, Wave};
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{one_pole_coef, Block, Smoothed};

pub const DELAY_DESC: BlockDesc = BlockDesc {
    kind: "delay",
    label: "Delay",
    category: Category::Time,
    params: &[
        ParamDesc::log("time_ms", "Tempo", 10.0, 2000.0, 350.0, "ms"),
        ParamDesc::number("feedback", "Ripetizioni", 0.0, 0.98, 0.35, ""),
        // 0 = solo segnale pulito, 1 = solo eco.
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 0.3, ""),
        // Le ripetizioni si scuriscono sopra questa frequenza (stile analogico).
        ParamDesc::log("damp_hz", "Tono eco", 200.0, 20000.0, 5000.0, "Hz"),
    ],
};

/// Memoria del delay: il massimo di `DELAY_DESC`.
const DELAY_MAX_MS: f32 = 2000.0;
/// Memoria del chorus: base + profondità non possono superarla.
const CHORUS_MAX_MS: f32 = 50.0;
/// Tempo di avvicinamento dei parametri cambiati dal vivo che saltano (ritardi).
const GLIDE_MS: f32 = 50.0;

// Indici dei parametri in DELAY_DESC e CHORUS_DESC.
const DELAY_TIME: usize = 0;
const DELAY_FEEDBACK: usize = 1;
const DELAY_MIX: usize = 2;
const DELAY_DAMP: usize = 3;

const CHORUS_RATE: usize = 0;
const CHORUS_BASE: usize = 1;
const CHORUS_DEPTH: usize = 2;
const CHORUS_MIX: usize = 3;

pub fn build_delay(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = DELAY_DESC.params;
    let params = DelayParams {
        time_ms: v.number(&p[0]),
        feedback: v.number(&p[1]),
        mix: v.number(&p[2]),
        damp_hz: v.number(&p[3]),
    };
    Ok(Box::new(Delay::new(&params, sr)))
}

pub const CHORUS_DESC: BlockDesc = BlockDesc {
    kind: "chorus",
    label: "Chorus",
    category: Category::Modulation,
    params: &[
        ParamDesc::log("rate_hz", "Velocità", 0.05, 10.0, 0.8, "Hz"),
        ParamDesc::number("base_ms", "Ritardo", 1.0, 30.0, 12.0, "ms"),
        // Escursione della modulazione (limitata a meno di base_ms).
        ParamDesc::number("depth_ms", "Profondità", 0.0, 15.0, 3.0, "ms"),
        ParamDesc::number("mix", "Mix", 0.0, 1.0, 0.5, ""),
    ],
};

pub fn build_chorus(v: &ParamValues, sr: f32) -> Result<Box<dyn Block>> {
    let p = CHORUS_DESC.params;
    let params = ChorusParams {
        rate_hz: v.number(&p[0]),
        base_ms: v.number(&p[1]),
        depth_ms: v.number(&p[2]),
        mix: v.number(&p[3]),
    };
    Ok(Box::new(Chorus::new(&params, sr)))
}

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

#[derive(Debug, Clone)]
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
    sr: f32,
    /// Ritardo in campioni che si avvicina al valore richiesto con un glide.
    /// Il glide è in f64: con f32 si fermerebbe a qualche campione dal bersaglio.
    delay_cur: f64,
    delay_target: f64,
    delay_coef: f64,
    feedback: f32,
    mix: Smoothed,
    lp: f32,
    lp_coef: f32,
}

impl Delay {
    pub fn new(p: &DelayParams, sr: f32) -> Self {
        let delay = (p.time_ms.clamp(1.0, DELAY_MAX_MS) * 0.001 * sr).max(1.0) as f64;
        Self {
            line: DelayLine::new((DELAY_MAX_MS * 0.001 * sr) as usize + 4),
            sr,
            delay_cur: delay,
            delay_target: delay,
            delay_coef: one_pole_coef(GLIDE_MS, sr) as f64,
            feedback: p.feedback.clamp(0.0, 0.98),
            mix: Smoothed::new(p.mix.clamp(0.0, 1.0), GLIDE_MS, sr),
            lp: 0.0,
            lp_coef: damp_coef(p.damp_hz, sr),
        }
    }
}

fn damp_coef(damp_hz: f32, sr: f32) -> f32 {
    (-2.0 * PI * damp_hz.clamp(200.0, sr * 0.45) / sr).exp()
}

impl Block for Delay {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            self.delay_cur = self.delay_target + (self.delay_cur - self.delay_target) * self.delay_coef;
            let echo = self.line.read(self.delay_cur as f32);
            self.lp = echo + (self.lp - echo) * self.lp_coef;
            self.line.push(*s + self.lp * self.feedback);
            let mix = self.mix.next();
            *s = *s * (1.0 - mix) + echo * mix;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            DELAY_TIME => self.delay_target = (value.clamp(1.0, DELAY_MAX_MS) * 0.001 * self.sr).max(1.0) as f64,
            DELAY_FEEDBACK => self.feedback = value.clamp(0.0, 0.98),
            DELAY_MIX => self.mix.target = value.clamp(0.0, 1.0),
            DELAY_DAMP => self.lp_coef = damp_coef(value, self.sr),
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
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
    sr: f32,
    /// Valori richiesti (prima dei limiti), per ricalcolare i ritardi.
    base_ms: f32,
    depth_ms: f32,
    /// Ritardi in campioni, con glide.
    base: Smoothed,
    depth: Smoothed,
    mix: Smoothed,
}

/// Ritardo centrale e profondità in campioni, limitati in modo che la somma
/// stia nella memoria preallocata di `CHORUS_MAX_MS`.
fn chorus_delays(base_ms: f32, depth_ms: f32, sr: f32) -> (f32, f32) {
    let base = base_ms.clamp(1.0, CHORUS_MAX_MS);
    let depth = depth_ms.clamp(0.0, (base * 0.95).min(CHORUS_MAX_MS - base));
    (base * 0.001 * sr, depth * 0.001 * sr)
}

impl Chorus {
    pub fn new(p: &ChorusParams, sr: f32) -> Self {
        let (base, depth) = chorus_delays(p.base_ms, p.depth_ms, sr);
        Self {
            line: DelayLine::new((CHORUS_MAX_MS * 0.001 * sr) as usize + 4),
            lfo: Lfo::new(p.rate_hz, Wave::Sine, sr),
            sr,
            base_ms: p.base_ms,
            depth_ms: p.depth_ms,
            base: Smoothed::new(base, GLIDE_MS, sr),
            depth: Smoothed::new(depth, GLIDE_MS, sr),
            mix: Smoothed::new(p.mix.clamp(0.0, 1.0), GLIDE_MS, sr),
        }
    }

    fn retarget_delays(&mut self) {
        let (base, depth) = chorus_delays(self.base_ms, self.depth_ms, self.sr);
        self.base.target = base;
        self.depth.target = depth;
    }
}

impl Block for Chorus {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            self.line.push(*s);
            let wet = self.line.read(self.base.next() + self.depth.next() * self.lfo.next());
            let mix = self.mix.next();
            *s = *s * (1.0 - mix) + wet * mix;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            CHORUS_RATE => self.lfo.set_rate(value, self.sr),
            CHORUS_BASE => {
                self.base_ms = value;
                self.retarget_delays();
            }
            CHORUS_DEPTH => {
                self.depth_ms = value;
                self.retarget_delays();
            }
            CHORUS_MIX => self.mix.target = value.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    #[test]
    fn set_param_cambia_il_tempo_del_delay() {
        // Il tempo passa da 100 ms a 200 ms: dopo il glide l'eco arriva a 9600 campioni.
        let p = DelayParams { time_ms: 100.0, feedback: 0.0, mix: 1.0, ..Default::default() };
        let mut d = Delay::new(&p, SR);
        d.set_param(DELAY_TIME, 200.0);
        let mut silence = vec![0.0; 48000];
        d.process(&mut silence);

        let mut buf = vec![0.0; 12000];
        buf[0] = 1.0;
        d.process(&mut buf);
        let echo_at = buf[1..].iter().position(|s| s.abs() > 0.1).unwrap() + 1;
        assert!((9599..=9601).contains(&echo_at), "eco a {echo_at}");
    }

    #[test]
    fn set_param_cambia_il_ritardo_del_chorus() {
        // Senza modulazione il ritardo è la base: da 10 ms a 30 ms (1440 campioni).
        let p = ChorusParams { base_ms: 10.0, depth_ms: 0.0, mix: 1.0, ..Default::default() };
        let mut c = Chorus::new(&p, SR);
        c.set_param(CHORUS_BASE, 30.0);
        let mut silence = vec![0.0; 48000];
        c.process(&mut silence);

        let mut buf = vec![0.0; 2000];
        buf[0] = 1.0;
        c.process(&mut buf);
        let echo_at = buf[1..].iter().position(|s| s.abs() > 0.5).unwrap() + 1;
        assert!((1439..=1441).contains(&echo_at), "eco a {echo_at}");
    }
}
