use std::f32::consts::PI;

use serde::Deserialize;

use super::Block;

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum FilterKind {
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peak,
    Lowshelf,
    Highshelf,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct FilterParams {
    pub kind: FilterKind,
    pub freq: f32,
    pub q: f32,
    /// Usato solo da peak, lowshelf e highshelf.
    pub gain_db: f32,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self { kind: FilterKind::Lowpass, freq: 1000.0, q: 0.707, gain_db: 0.0 }
    }
}

/// Biquad (formule "Audio EQ Cookbook" di R. Bristow-Johnson),
/// forma diretta II trasposta.
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn new(p: &FilterParams, sr: f32) -> Self {
        let freq = p.freq.clamp(10.0, sr * 0.45);
        let q = p.q.max(0.05);
        let w0 = 2.0 * PI * freq / sr;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10f32.powf(p.gain_db / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match p.kind {
            FilterKind::Lowpass => {
                ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterKind::Highpass => {
                ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterKind::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterKind::Lowshelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + sq),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - sq),
                    (a + 1.0) + (a - 1.0) * cos + sq,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - sq,
                )
            }
            FilterKind::Highshelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - sq),
                    (a + 1.0) - (a - 1.0) * cos + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - sq,
                )
            }
        };

        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }
}

impl Block for Biquad {
    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let x = *s;
            let y = self.b0 * x + self.z1;
            self.z1 = self.b1 * x - self.a1 * y + self.z2;
            self.z2 = self.b2 * x - self.a2 * y;
            *s = y;
        }
    }
}
