//! Simulazione di cassa: convoluzione con una risposta all'impulso (IR).
//!
//! L'IR arriva da un file WAV oppure viene generata dai parametri del preset:
//! si disegna la curva di risposta della cassa, la si trasforma in un filtro a
//! fase minima (come quella di un altoparlante vero) e la si applica come FIR.

use std::f32::consts::PI;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;
use super::params::{BlockDesc, Category, ParamDesc, ParamValues};
use super::{db_to_lin, Block};
use crate::wav;

// Da migrare (D1): i parametri procedurali richiedono una ricostruzione; set_param per level_db.
pub const CAB_DESC: BlockDesc = BlockDesc {
    kind: "cab",
    label: "Cassa",
    category: Category::Amp,
    params: &[
        // Se c'è un file, i parametri procedurali sono ignorati.
        ParamDesc::file("ir", "IR (file WAV)", &["wav"]),
        ParamDesc::number("max_ms", "Lunghezza max IR", 5.0, 500.0, 80.0, "ms"),
        ParamDesc::number("level_db", "Livello", -24.0, 24.0, 0.0, "dB"),
        ParamDesc::log("low_hz", "Risonanza", 40.0, 250.0, 100.0, "Hz"),
        ParamDesc::number("low_q", "Q risonanza", 0.5, 3.0, 1.3, ""),
        ParamDesc::log("high_hz", "Taglio alti", 1500.0, 12000.0, 5000.0, "Hz"),
        ParamDesc::log("mid_hz", "Medi", 150.0, 2000.0, 500.0, "Hz"),
        ParamDesc::number("mid_db", "Guadagno medi", -12.0, 12.0, -3.0, "dB"),
        ParamDesc::log("presence_hz", "Presenza", 1000.0, 6000.0, 2500.0, "Hz"),
        ParamDesc::number("presence_db", "Guadagno presenza", -12.0, 12.0, 3.0, "dB"),
        ParamDesc::number("ripple_db", "Irregolarità", 0.0, 12.0, 3.0, "dB"),
        ParamDesc::int("seed", "Seme", 0.0, 9999.0, 1.0),
    ],
};

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
    /// File WAV con l'IR di una cassa vera. Se c'è, i parametri procedurali sono ignorati.
    pub ir: Option<String>,
    /// Lunghezza massima dell'IR da file: oltre è quasi sempre solo riverbero della stanza.
    pub max_ms: f32,
    pub level_db: f32,

    // Cassa procedurale.
    /// Risonanza del cono: sotto questa frequenza i bassi cadono.
    pub low_hz: f32,
    /// Quanto è pronunciata la risonanza (0.7 = piatta, 2 = "boomy").
    pub low_q: f32,
    /// Oltre questa frequenza gli alti cadono ripidi (36 dB/ottava).
    pub high_hz: f32,
    pub mid_hz: f32,
    /// Medi: negativo = "scavato", positivo = "nasale".
    pub mid_db: f32,
    pub presence_hz: f32,
    pub presence_db: f32,
    /// Irregolarità del cono: picchi e buchi che rendono la cassa "vera".
    pub ripple_db: f32,
    /// Cambia la disposizione delle irregolarità: ogni seme è una cassa diversa.
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

/// Lunghezza della IR procedurale: 1024 campioni (21 ms a 48 kHz) bastano,
/// perché un filtro a fase minima concentra l'energia all'inizio.
const PROCEDURAL_LEN: usize = 1024;
const FFT_LEN: usize = 8192;

pub struct Cab {
    ir: Vec<f32>,
    /// Storico dei campioni duplicato (2 x len): la finestra da moltiplicare
    /// per l'IR è sempre contigua, così il compilatore la vettorizza.
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
        let gain = db_to_lin(p.level_db);
        for s in &mut ir {
            *s *= gain;
        }
        let len = ir.len();
        Ok(Self { ir, history: vec![0.0; len * 2], pos: 0 })
    }
}

impl Block for Cab {
    fn process(&mut self, buf: &mut [f32]) {
        let len = self.ir.len();
        for s in buf {
            // Si scrive all'indietro: history[pos + k] è il campione di k passi fa.
            self.pos = if self.pos == 0 { len - 1 } else { self.pos - 1 };
            self.history[self.pos] = *s;
            self.history[self.pos + len] = *s;
            *s = dot(&self.ir, &self.history[self.pos..self.pos + len]);
        }
    }
}

/// Prodotto scalare con 8 accumulatori indipendenti, così diventa SIMD.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ac, ar) = a.split_at(a.len() / 8 * 8);
    let (bc, br) = b.split_at(ac.len());
    for (x, y) in ac.chunks_exact(8).zip(bc.chunks_exact(8)) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let tail: f32 = ar.iter().zip(br).map(|(x, y)| x * y).sum();
    acc.iter().sum::<f32>() + tail
}

fn load_ir(path: &Path, sr: f32, max_ms: f32) -> Result<Vec<f32>> {
    let data = wav::read(path).with_context(|| format!("IR della cassa: {}", path.display()))?;
    let src = &data.channels[0];
    ensure!(!src.is_empty(), "l'IR {} è vuota", path.display());

    // Se il file ha un'altra frequenza di campionamento, lo si porta a quella del motore.
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

/// Risposta in ampiezza (lineare) della cassa disegnata dai parametri.
fn target_magnitude(p: &CabParams, f: f32) -> f32 {
    let f = f.max(1.0);

    // Passa-alto risonante del 2° ordine: la risonanza del cono nel mobile.
    let r = f / p.low_hz;
    let hp = r * r / ((1.0 - r * r).powi(2) + (r / p.low_q.max(0.3)).powi(2)).sqrt();

    // Passa-basso del 6° ordine: oltre i 5 kHz circa il cono non riproduce quasi più niente.
    let lp = 1.0 / (1.0 + (f / p.high_hz).powi(12)).sqrt();

    let bump = |center: f32, db: f32, width_oct: f32| {
        let oct = (f / center).log2();
        db * (-(oct * oct) / (2.0 * width_oct * width_oct)).exp()
    };
    let mut db = bump(p.mid_hz, p.mid_db, 1.0) + bump(p.presence_hz, p.presence_db, 0.6);

    // Irregolarità sopra 1 kHz, sempre uguali per lo stesso seme.
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

/// IR a fase minima dalla curva desiderata, con il metodo del cepstrum.
fn procedural_ir(p: &CabParams, sr: f32) -> Vec<f32> {
    let n = FFT_LEN;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let ifft = planner.plan_fft_inverse(n);

    // 1. log dell'ampiezza su tutto lo spettro (simmetrico).
    let mut buf: Vec<Complex32> = (0..n)
        .map(|k| {
            let bin = if k <= n / 2 { k } else { n - k };
            let f = bin as f32 * sr / n as f32;
            Complex32::new(target_magnitude(p, f).max(1e-6).ln(), 0.0)
        })
        .collect();

    // 2. cepstrum reale, "piegato" in modo da renderlo causale (= fase minima).
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

    // 3. di nuovo nel dominio della frequenza, esponenziale, e poi nel tempo.
    fft.process(&mut buf);
    for c in buf.iter_mut() {
        *c = c.exp();
    }
    ifft.process(&mut buf);

    let mut ir: Vec<f32> = buf[..PROCEDURAL_LEN].iter().map(|c| c.re * scale).collect();
    fade_out(&mut ir);
    ir
}

/// Smorza l'ultimo 10% dell'IR, così il taglio non introduce artefatti.
fn fade_out(ir: &mut [f32]) {
    let fade = (ir.len() / 10).max(1);
    let start = ir.len() - fade;
    for (i, s) in ir[start..].iter_mut().enumerate() {
        *s *= 0.5 * (1.0 + (PI * i as f32 / fade as f32).cos());
    }
}

/// Porta a 0 dB il guadagno medio tra 300 Hz e 3 kHz, dove suona la chitarra:
/// IR diverse hanno volumi diversi, così cambiarle non fa salti di volume.
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
    fn la_cassa_procedurale_ha_la_forma_giusta() {
        let cab = Cab::new(&CabParams::default(), SR).unwrap();
        let (sub, low, mid, high, top) =
            (db_at(&cab, 40.0), db_at(&cab, 100.0), db_at(&cab, 1000.0), db_at(&cab, 8000.0), db_at(&cab, 15000.0));
        println!("40 Hz {sub:.1}  100 Hz {low:.1}  1 kHz {mid:.1}  8 kHz {high:.1}  15 kHz {top:.1} dB");
        assert!(sub < low - 8.0, "i bassissimi devono cadere");
        assert!(low > -3.0, "la risonanza sui bassi deve esserci");
        assert!(mid.abs() < 6.0, "i medi restano vicino a 0 dB");
        assert!(high < mid - 15.0, "gli alti devono cadere");
        assert!(top < mid - 40.0, "le frequenze altissime spariscono");
    }

    #[test]
    fn semi_diversi_danno_casse_diverse() {
        let a = Cab::new(&CabParams { seed: 1, ..Default::default() }, SR).unwrap();
        let b = Cab::new(&CabParams { seed: 2, ..Default::default() }, SR).unwrap();
        let diff: f32 = [1500.0, 2200.0, 3100.0, 4200.0].iter().map(|&f| (db_at(&a, f) - db_at(&b, f)).abs()).sum();
        assert!(diff > 1.0, "differenza {diff}");
    }

    #[test]
    fn ir_da_file_con_altra_frequenza_di_campionamento() {
        // Un impulso unitario a 44,1 kHz: la cassa deve diventare (quasi) un filo.
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
    fn un_file_mancante_e_un_errore_chiaro() {
        let p = CabParams { ir: Some("non-esiste.wav".into()), ..Default::default() };
        let err = Cab::new(&p, SR).err().expect("deve fallire");
        assert!(format!("{err:#}").contains("non-esiste.wav"));
    }

    #[test]
    fn la_convoluzione_e_un_fir_corretto() {
        let mut cab = Cab { ir: vec![0.5, 0.25, 0.125], history: vec![0.0; 6], pos: 0 };
        let mut buf = vec![1.0, 0.0, 0.0, 0.0, 2.0];
        cab.process(&mut buf);
        assert_eq!(buf, vec![0.5, 0.25, 0.125, 0.0, 1.0]);
    }

    #[test]
    fn abbastanza_veloce_per_il_tempo_reale() {
        // IR lunga 80 ms (caso peggiore da file): 1 s di audio deve costare molto meno di 1 s.
        let mut cab = Cab { ir: vec![0.001; 3840], history: vec![0.0; 7680], pos: 0 };
        let mut buf = vec![0.1f32; 48000];
        let t = std::time::Instant::now();
        for block in buf.chunks_mut(96) {
            cab.process(block);
        }
        let ms = t.elapsed().as_secs_f32() * 1000.0;
        println!("1 s di audio con IR da 3840 campioni: {ms:.1} ms");
        assert!(ms < 300.0);
    }
}
