//! Minimal reading and writing of WAV files (32-bit float; 16/24-bit PCM when reading).

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

/// Writes a 32-bit float WAV; the sizes are filled in by `finish`.
pub struct WavWriter {
    out: BufWriter<File>,
    samples: u32,
}

impl WavWriter {
    pub fn create(path: &Path, channels: u16, sample_rate: u32) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
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

/// Audio read from a file: one vector per channel.
pub struct WavData {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

pub fn read(path: &Path) -> Result<WavData> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    ensure!(bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE", "not a WAV file");

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
            // WAVE_FORMAT_EXTENSIBLE: the real format is in the first 2 bytes of the GUID.
            if format == 0xFFFE && len >= 26 {
                format = u16_at(body + 24);
            }
            fmt = Some((format, u16_at(body + 2) as usize, u32_at(body + 4), u16_at(body + 14)));
        } else if id == b"data" {
            let Some((format, channels, sample_rate, bits)) = fmt else { bail!("missing fmt chunk") };
            let end = (body + len).min(bytes.len());
            let data = &bytes[body..end];
            let decoded: Vec<f32> = match (format, bits) {
                (3, 32) => data.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect(),
                (1, 16) => {
                    data.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.0).collect()
                }
                (1, 24) => data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0)
                    .collect(),
                (1, 32) => data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| i32::from_le_bytes(*b) as f32 / 2_147_483_648.0)
                    .collect(),
                _ => bail!("unsupported WAV format (format {format}, {bits} bit)"),
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
    bail!("missing data chunk")
}
