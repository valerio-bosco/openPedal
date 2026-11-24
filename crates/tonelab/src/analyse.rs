//! `analyse`: 4 s windows of the guitar stem -> kind, level, brightness; tempo, key, low notes
//! (port of analyse.py; chroma/tempo follow librosa).
use std::f64::consts::PI;

use anyhow::{Context, Result};

use crate::dsp::{hann_periodic, onset_strength, resample_poly, stft_power, Fft};
use crate::job::{Analysis, Audio, Job, LowNote, Section};
use crate::pyin::pyin;

const WIN_S: f64 = 4.0;
const CORR_CENTRE: f64 = 0.8;
const CHROMA_DOUBLE: f64 = 0.8;
const QUIET_ABS_DB: f64 = -55.0;
const QUIET_REL_DB: f64 = 30.0;
const LOW_HZ: f64 = 80.0;
const EPS: f64 = 1e-12;

const KS_MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const KS_MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const NOTES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

const LOW_BASS_GAP_DB: f64 = 25.0;
const LOW_VOICED_PROB: f64 = 0.6;
const LOW_MAX_STD_ST: f64 = 1.0;
const LOW_MIN_S: f64 = 0.15;
const LOW_SUB_MID_DB: f64 = -30.0;

fn db(x: f64) -> f64 {
    20.0 * x.max(EPS).log10()
}

fn rms_iter<'a>(it: impl Iterator<Item = &'a f64>) -> f64 {
    let (mut s, mut n) = (0.0, 0usize);
    for v in it {
        s += v * v;
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        (s / n as f64).sqrt()
    }
}

fn corr(l: &[f64], r: &[f64]) -> f64 {
    let n = l.len() as f64;
    let (ml, mr) = (l.iter().sum::<f64>() / n, r.iter().sum::<f64>() / n);
    let (mut sl, mut sr, mut sx) = (0.0, 0.0, 0.0);
    for (a, b) in l.iter().zip(r) {
        let (a, b) = (a - ml, b - mr);
        sl += a * a;
        sr += b * b;
        sx += a * b;
    }
    let d = (sl * sr).sqrt();
    if d > EPS {
        sx / d
    } else {
        1.0
    }
}

/// librosa `filters.chroma` (tuning 0, ctroct 5, octwidth 2, L2 columns): 12 x (n_fft/2+1).
fn chroma_fb(sr: f64, n_fft: usize) -> Vec<Vec<f64>> {
    let nc = 12.0;
    let mut frq: Vec<f64> = (1..n_fft).map(|k| nc * (k as f64 * sr / n_fft as f64 / (440.0 / 16.0)).log2()).collect();
    frq.insert(0, frq[0] - 1.5 * nc);
    let bw: Vec<f64> = (0..n_fft).map(|i| if i + 1 < n_fft { (frq[i + 1] - frq[i]).max(1.0) } else { 1.0 }).collect();
    let mut w = vec![vec![0.0; n_fft]; 12];
    for k in 0..n_fft {
        let mut col = [0.0; 12];
        let mut norm = 0.0;
        for (c, cv) in col.iter_mut().enumerate() {
            let d = (frq[k] - c as f64 + 6.0 + 120.0).rem_euclid(12.0) - 6.0;
            *cv = (-0.5 * (2.0 * d / bw[k]).powi(2)).exp();
            norm += *cv * *cv;
        }
        let oct = (-0.5 * ((frq[k] / nc - 5.0) / 2.0).powi(2)).exp();
        for c in 0..12 {
            // roll by -3 along the chroma axis: C-based
            w[(c + 12 - 3) % 12][k] = col[c] / norm.sqrt().max(1e-300) * oct;
        }
    }
    w.iter().map(|r| r[..n_fft / 2 + 1].to_vec()).collect()
}

/// Frames x 12 chroma vectors, each normalised by its max (librosa `chroma_stft`, no tuning estimate).
fn chroma_stft(fft: &mut Fft, fb: &[Vec<f64>], y: &[f32], n_fft: usize, hop: usize) -> Vec<[f64; 12]> {
    stft_power(fft, y, n_fft, hop)
        .iter()
        .map(|fr| {
            let mut c = [0.0; 12];
            for (ci, row) in fb.iter().enumerate() {
                c[ci] = row.iter().zip(fr).map(|(w, p)| w * p).sum();
            }
            let m = c.iter().cloned().fold(0.0, f64::max);
            if m > f64::MIN_POSITIVE {
                c.iter_mut().for_each(|v| *v /= m);
            }
            c
        })
        .collect()
}

fn chroma_sim(fft: &mut Fft, fb: &[Vec<f64>], l: &[f32], r: &[f32]) -> f64 {
    let cl = chroma_stft(fft, fb, l, 2048, 512);
    let cr = chroma_stft(fft, fb, r, 2048, 512);
    let mut v: Vec<f64> = vec![];
    for (a, b) in cl.iter().zip(&cr) {
        let d = a.iter().map(|x| x * x).sum::<f64>().sqrt() * b.iter().map(|x| x * x).sum::<f64>().sqrt();
        if d > EPS {
            v.push(a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>() / d);
        }
    }
    if v.is_empty() {
        return 0.0;
    }
    median(&mut v)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn np_hanning(n: usize) -> Vec<f64> {
    if n == 1 {
        return vec![1.0];
    }
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / (n - 1) as f64).cos()).collect()
}

fn centroid(fft: &mut Fft, x: &[f64], sr: f64) -> f64 {
    let w = np_hanning(x.len());
    let xw: Vec<f64> = x.iter().zip(&w).map(|(a, b)| a * b).collect();
    let spec = fft.rfft(&xw, x.len());
    let (mut s, mut fs) = (0.0, 0.0);
    for (k, c) in spec.iter().enumerate() {
        let m = c.norm();
        s += m;
        fs += m * k as f64 * sr / x.len() as f64;
    }
    if s > EPS {
        fs / s
    } else {
        0.0
    }
}

struct Win {
    start: f64,
    end: f64,
    guitar_db: f64,
    corr: f64,
    side_mid_db: f64,
    centroid_hz: f64,
    bass_db: f64,
    chroma_sim: f64,
    kind: &'static str,
}

fn classify(w: &Win, loudest: f64) -> &'static str {
    if w.guitar_db < QUIET_ABS_DB || w.guitar_db < loudest - QUIET_REL_DB {
        "none"
    } else if w.corr >= CORR_CENTRE {
        "centre"
    } else if w.chroma_sim >= CHROMA_DOUBLE {
        "double"
    } else {
        "wide"
    }
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn merge_sections(wins: &[Win]) -> Vec<Section> {
    let mut groups: Vec<(usize, usize)> = vec![];
    for (i, w) in wins.iter().enumerate() {
        match groups.last_mut() {
            Some(g) if wins[g.0].kind == w.kind => g.1 = i,
            _ => groups.push((i, i)),
        }
    }
    groups
        .iter()
        .map(|&(a, b)| {
            let ws = &wins[a..=b];
            let avg = |f: fn(&Win) -> f64| r2(ws.iter().map(f).sum::<f64>() / ws.len() as f64);
            Section {
                start: r2(ws[0].start),
                end: r2(ws[ws.len() - 1].end),
                guitar_db: avg(|w| w.guitar_db),
                corr: avg(|w| w.corr),
                side_mid_db: avg(|w| w.side_mid_db),
                centroid_hz: avg(|w| w.centroid_hz).round_ties_even() as i64,
                kind: ws[0].kind.to_string(),
                bass_db: avg(|w| w.bass_db),
            }
        })
        .collect()
}

fn tempo_bpm(fft: &mut Fft, mono22: &[f32]) -> f64 {
    let (sr, hop, win_len) = (22050.0, 256usize, 689usize);
    let env = onset_strength(fft, mono22, sr, hop, true);
    let n = env.len();
    if n == 0 || env.iter().all(|v| *v == 0.0) {
        return 0.0;
    }
    let pad = win_len / 2;
    let mut padded = Vec::with_capacity(n + 2 * pad);
    padded.extend((0..pad).map(|i| env[0] * i as f64 / pad as f64));
    padded.extend_from_slice(&env);
    padded.extend((0..pad).map(|j| env[n - 1] * (pad - 1 - j) as f64 / pad as f64));
    let w = hann_periodic(win_len);
    let size = (2 * win_len).next_power_of_two();
    let mut acc = vec![0.0; win_len];
    let mut buf = vec![0.0; win_len];
    for t in 0..n {
        for i in 0..win_len {
            buf[i] = padded[t + i] * w[i];
        }
        let spec = fft.rfft(&buf, size);
        let p: Vec<_> = spec.iter().map(|c| rustfft::num_complex::Complex::new(c.norm_sqr(), 0.0)).collect();
        let ac = fft.irfft(&p, size);
        let m = ac[..win_len].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        if m > f64::MIN_POSITIVE {
            for k in 0..win_len {
                acc[k] += ac[k] / m;
            }
        }
    }
    let mut best = (f64::NEG_INFINITY, 0.0);
    for lag in 1..win_len {
        let bpm = 60.0 * sr / (hop as f64 * lag as f64);
        if bpm >= 320.0 {
            continue;
        }
        let s = (1e6 * acc[lag] / n as f64).ln_1p() - 0.5 * (bpm.log2() - 120f64.log2()).powi(2);
        if s > best.0 {
            best = (s, bpm);
        }
    }
    best.1
}

fn estimate_key(fft: &mut Fft, mono22: &[f32]) -> String {
    let n_fft = 4096;
    let fb = chroma_fb(22050.0, n_fft);
    let ch = chroma_stft(fft, &fb, mono22, n_fft, 512);
    let mut mean = [0.0; 12];
    for c in &ch {
        for i in 0..12 {
            mean[i] += c[i] / ch.len().max(1) as f64;
        }
    }
    let pc = |a: &[f64], b: &[f64]| {
        let (ma, mb) = (a.iter().sum::<f64>() / 12.0, b.iter().sum::<f64>() / 12.0);
        let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
        for i in 0..12 {
            sab += (a[i] - ma) * (b[i] - mb);
            saa += (a[i] - ma).powi(2);
            sbb += (b[i] - mb).powi(2);
        }
        sab / (saa * sbb).sqrt()
    };
    let (mut best, mut name) = (-2.0, "C major".to_string());
    for tonic in 0..12 {
        for (prof, mode) in [(&KS_MAJOR, "major"), (&KS_MINOR, "minor")] {
            let rolled: Vec<f64> = (0..12).map(|i| prof[(i + 12 - tonic) % 12]).collect();
            let c = pc(&mean, &rolled);
            if c > best {
                best = c;
                name = format!("{} {mode}", NOTES[tonic]);
            }
        }
    }
    name
}

fn sub_mid_db(fft: &mut Fft, x: &[f64], sr: f64) -> f64 {
    let w = np_hanning(x.len());
    let xw: Vec<f64> = x.iter().zip(&w).map(|(a, b)| a * b).collect();
    let spec = fft.rfft(&xw, x.len());
    let (mut sub, mut mid) = (0.0, 0.0);
    for (k, c) in spec.iter().enumerate() {
        let f = k as f64 * sr / x.len() as f64;
        if (25.0..LOW_HZ).contains(&f) {
            sub += c.norm_sqr();
        } else if (LOW_HZ..2.0 * LOW_HZ).contains(&f) {
            mid += c.norm_sqr();
        }
    }
    10.0 * (sub.max(EPS) / mid.max(EPS)).log10()
}

fn find_low_notes(fft: &mut Fft, mono: &[f32], sr: u32, wins: &[Win]) -> Vec<LowNote> {
    let sp = 16000.0;
    let y = resample_poly(mono, sr, 16000);
    let hop = 512usize;
    let min_frames = (LOW_MIN_S * sp / hop as f64).ceil() as usize;
    let srf = sr as f64;
    let mut out = vec![];
    for w in wins {
        if w.kind == "none" || w.guitar_db - w.bass_db < LOW_BASS_GAP_DB {
            continue;
        }
        let a = ((w.start * sp) as usize).min(y.len());
        let b = ((w.end * sp) as usize).min(y.len());
        let seg = &y[a..b];
        if seg.len() < 4096 {
            continue;
        }
        let p = pyin(seg, 30.0, 400.0, sp, 4096, hop);
        let ok: Vec<bool> = (0..p.f0.len()).map(|i| p.voiced[i] && p.f0[i].is_finite() && p.f0[i] < LOW_HZ && p.voiced_prob[i] > LOW_VOICED_PROB).collect();
        let t = |i: usize| w.start + (i * hop) as f64 / sp;
        let mut k = 0;
        while k < ok.len() {
            if !ok[k] {
                k += 1;
                continue;
            }
            let mut j = k;
            while j + 1 < ok.len() && ok[j + 1] {
                j += 1;
            }
            let run = &p.f0[k..=j];
            let (start, end) = (t(k), t(j) + hop as f64 / sp);
            if j - k + 1 >= min_frames && end - start >= LOW_MIN_S {
                let mut s = run.to_vec();
                let med = median(&mut s);
                let semis: Vec<f64> = run.iter().map(|f| 12.0 * (f / med).log2()).collect();
                let mean = semis.iter().sum::<f64>() / semis.len() as f64;
                let std = (semis.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / semis.len() as f64).sqrt();
                let xa = ((start * srf) as usize).min(mono.len());
                let xb = ((end * srf) as usize).min(mono.len());
                if std < LOW_MAX_STD_ST && xb - xa > 256 {
                    let x: Vec<f64> = mono[xa..xb].iter().map(|&v| v as f64).collect();
                    if sub_mid_db(fft, &x, srf) >= LOW_SUB_MID_DB {
                        out.push(LowNote { start: r2(start), end: r2(end), hz: (med * 10.0).round() / 10.0 });
                    }
                }
            }
            k = j + 1;
        }
    }
    out
}

fn mono_of(a: &Audio) -> Vec<f32> {
    let [l, r] = a.stereo();
    l.iter().zip(r).map(|(a, b)| (a + b) / 2.0).collect()
}

/// Analysis of in-memory audio (the guitar stem, the bass stem when there is one, the mix).
pub fn analyse_audio(guitar: &Audio, bass: Option<&Audio>, mix: &Audio) -> Analysis {
    let mut fft = Fft::new();
    let sr = guitar.sample_rate;
    let srf = sr as f64;
    let [gl, gr] = guitar.stereo();
    let total = gl.len();
    let n = (WIN_S * srf) as usize;
    let fb = chroma_fb(srf, 2048);
    let mut wins: Vec<Win> = vec![];
    let mut i = 0;
    while i < total {
        let e = (i + n).min(total);
        if e - i < n / 2 && !wins.is_empty() {
            wins.last_mut().unwrap().end = total as f64 / srf;
            break;
        }
        let l: Vec<f64> = gl[i..e].iter().map(|&v| v as f64).collect();
        let r: Vec<f64> = gr[i..e].iter().map(|&v| v as f64).collect();
        let mid: Vec<f64> = l.iter().zip(&r).map(|(a, b)| (a + b) / 2.0).collect();
        let side: Vec<f64> = l.iter().zip(&r).map(|(a, b)| (a - b) / 2.0).collect();
        let g_rms = rms_iter(l.iter().chain(r.iter()));
        let bass_db = match bass {
            Some(b) => {
                let [bl, br] = b.stereo();
                let (ba, bb) = (i.min(bl.len()), e.min(bl.len()));
                let v: Vec<f64> = bl[ba..bb].iter().chain(br[ba..bb].iter()).map(|&x| x as f64).collect();
                db(rms_iter(v.iter()))
            }
            None => -120.0,
        };
        wins.push(Win {
            start: i as f64 / srf,
            end: e as f64 / srf,
            guitar_db: db(g_rms),
            corr: corr(&l, &r),
            side_mid_db: db(rms_iter(side.iter())) - db(rms_iter(mid.iter())),
            centroid_hz: centroid(&mut fft, &mid, srf),
            bass_db,
            chroma_sim: chroma_sim(&mut fft, &fb, &gl[i..e], &gr[i..e]),
            kind: "none",
        });
        i += n;
    }
    let loudest = wins.iter().map(|w| w.guitar_db).fold(f64::NEG_INFINITY, f64::max);
    for k in 0..wins.len() {
        wins[k].kind = classify(&wins[k], loudest);
    }
    let mono22 = resample_poly(&mono_of(mix), mix.sample_rate, 22050);
    let tempo = tempo_bpm(&mut fft, &mono22);
    let key = estimate_key(&mut fft, &mono22);
    let low_notes = if bass.is_some() { find_low_notes(&mut fft, &mono_of(guitar), sr, &wins) } else { vec![] };
    Analysis { tempo_bpm: (tempo * 10.0).round() / 10.0, key, sections: merge_sections(&wins), low_notes }
}

/// Reads the guitar stem (+ bass stem, mix) of the job, analyses, writes `analysis.json`.
pub fn analyse(job: &Job) -> Result<Analysis> {
    let guitar = job.read_stem("guitar")?.context("stems/guitar.wav missing (run separation first)")?;
    let bass = job.read_stem("bass")?;
    let mix = job.read_mix()?;
    let a = analyse_audio(&guitar, bass.as_ref(), &mix);
    job.write_json("analysis.json", &a)?;
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(f: f64, secs: f64, sr: u32, amp: f64) -> Vec<f32> {
        (0..(secs * sr as f64) as usize).map(|i| (amp * (2.0 * PI * f * i as f64 / sr as f64).sin()) as f32).collect()
    }

    #[test]
    fn kinds_follow_stereo_image() {
        let sr = 22050;
        // 8 s centre (identical channels), 8 s wide (two different notes), 8 s silence
        let mut l = tone(220.0, 8.0, sr, 0.3);
        let mut r = l.clone();
        l.extend(tone(220.0, 8.0, sr, 0.3));
        r.extend(tone(330.0, 8.0, sr, 0.3));
        l.extend(vec![0.0; 8 * sr as usize]);
        r.extend(vec![0.0; 8 * sr as usize]);
        let a = Audio { sample_rate: sr, channels: vec![l.clone(), r.clone()] };
        let res = analyse_audio(&a, None, &a);
        let kinds: Vec<&str> = res.sections.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, ["centre", "wide", "none"], "{:?}", res.sections);
        assert_eq!(res.sections[0].end, 8.0);
    }

    #[test]
    fn tempo_of_a_click_track() {
        let sr = 22050;
        let mut x = vec![0.0f32; sr as usize * 20];
        let period = (60.0 / 120.0 * sr as f64) as usize; // 120 BPM
        let mut i = 0;
        while i + 400 < x.len() {
            for k in 0..400 {
                x[i + k] = ((1.0 - k as f64 / 400.0) * (2.0 * PI * 1000.0 * k as f64 / sr as f64).sin()) as f32;
            }
            i += period;
        }
        let mut fft = Fft::new();
        let bpm = tempo_bpm(&mut fft, &x);
        assert!((bpm - 120.0).abs() < 3.0, "{bpm}");
    }

    #[test]
    fn key_of_an_a_minor_arpeggio() {
        let sr = 22050;
        let mut x = vec![];
        for _ in 0..8 {
            for f in [220.0, 261.63, 329.63, 440.0, 329.63, 261.63] {
                x.extend(tone(f, 0.5, sr, 0.3));
            }
        }
        let mut fft = Fft::new();
        assert_eq!(estimate_key(&mut fft, &x), "A minor");
    }
}
