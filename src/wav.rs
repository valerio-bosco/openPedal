//! Lettura e scrittura minimale di file WAV (float a 32 bit, PCM a 16/24 bit in lettura).

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

/// Scrive un WAV float a 32 bit; le dimensioni si fissano in `finish`.
pub struct WavWriter {
    out: BufWriter<File>,
    samples: u32,
}

impl WavWriter {
    pub fn create(path: &Path, channels: u16, sample_rate: u32) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = File::create(path).with_context(|| format!("impossibile creare {}", path.display()))?;
        let mut out = BufWriter::new(file);
        let block_align = channels * 4;
        out.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        out.write_all(&3u16.to_le_bytes())?; // 3 = IEEE float
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&sample_rate.to_le_bytes())?;
        out.write_all(&(sample_rate * block_align as u32).to_le_bytes())?;
        out.write_all(&block_align.to_le_bytes())?;
        out.write_all(&32u16.to_le_bytes())?;
        out.write_all(b"data\0\0\0\0")?;
        Ok(Self { out, samples: 0 })
    }

    pub fn write(&mut self, sample: f32) -> Result<()> {
        self.out.write_all(&sample.to_le_bytes())?;
        self.samples += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        let data_len = self.samples * 4;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(36 + data_len).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&data_len.to_le_bytes())?;
        self.out.flush()?;
        Ok(())
    }
}

/// Audio letto da file: un vettore per canale.
pub struct WavData {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

pub fn read(path: &Path) -> Result<WavData> {
    let bytes = std::fs::read(path).with_context(|| format!("impossibile leggere {}", path.display()))?;
    ensure!(bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE", "non è un file WAV");

    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);

    let mut fmt = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(pos + 4) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            let mut format = u16_at(body);
            // WAVE_FORMAT_EXTENSIBLE: il formato vero sta nei primi 2 byte del GUID.
            if format == 0xFFFE && len >= 26 {
                format = u16_at(body + 24);
            }
            fmt = Some((format, u16_at(body + 2) as usize, u32_at(body + 4), u16_at(body + 14)));
        } else if id == b"data" {
            let Some((format, channels, sample_rate, bits)) = fmt else { bail!("chunk fmt mancante") };
            let end = (body + len).min(bytes.len());
            let data = &bytes[body..end];
            let decoded: Vec<f32> = match (format, bits) {
                (3, 32) => data.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
                (1, 16) => {
                    data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect()
                }
                (1, 24) => data
                    .chunks_exact(3)
                    .map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0)
                    .collect(),
                (1, 32) => data
                    .chunks_exact(4)
                    .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0)
                    .collect(),
                _ => bail!("formato WAV non supportato (formato {format}, {bits} bit)"),
            };
            let mut out = vec![Vec::with_capacity(decoded.len() / channels); channels];
            for frame in decoded.chunks_exact(channels) {
                for (ch, s) in frame.iter().enumerate() {
                    out[ch].push(*s);
                }
            }
            return Ok(WavData { sample_rate, channels: out });
        }
        pos = body + len + (len & 1);
    }
    bail!("chunk data mancante")
}
