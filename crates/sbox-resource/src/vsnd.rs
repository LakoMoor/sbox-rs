//! `.vsnd_c` sound loader.
//!
//! DATA block (little endian): `u32 packed` (low 16 bits = sample rate, then `u8 format`, `u8 channels`),
//! `u32 reserved`, `u32 sample_count`, `f32 duration`, ... The encoded audio is appended after the blocks.

use crate::Resource;
use anyhow::{ensure, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundFormat {
    Pcm16,
    Mp3,
    Vorbis,
    /// Raw Opus packets back to back; packet boundaries come from a table in the DATA block.
    Opus,
    Unknown(u8),
}

#[derive(Debug)]
pub struct Sound {
    pub sample_rate: u32,
    pub channels: u32,
    pub format: SoundFormat,
    pub samples: u32,
    pub duration: f32,
    pub data: Vec<u8>,
    /// Cumulative `(end byte offset, end sample)` per packet (Opus only).
    pub packets: Vec<(u32, u32)>,
}

impl Sound {
    /// Decode an Opus sound to interleaved PCM16 at 48 kHz.
    pub fn decode_opus(&self) -> Result<Vec<i16>> {
        let ch = self.channels as usize;
        let mut dec = opus_decoder::OpusDecoder::new(48_000, ch).map_err(|e| anyhow::anyhow!("opus: {e:?}"))?;
        let mut out = Vec::new();
        let mut prev = (0usize, 0u32);
        for &(end, end_samples) in &self.packets {
            let end = end as usize;
            ensure!(end >= prev.0 && end <= self.data.len(), "opus packet table out of range");
            let frame = (end_samples - prev.1) as usize;
            let mut pcm = vec![0i16; frame.max(2880) * ch];
            let n = dec.decode(&self.data[prev.0..end], &mut pcm, false).map_err(|e| anyhow::anyhow!("opus packet: {e:?}"))?;
            out.extend_from_slice(&pcm[..n * ch]);
            prev = (end, end_samples);
        }
        Ok(out)
    }

    /// A playable file image: PCM is wrapped in a WAV header, MP3 / Ogg are returned as they are.
    pub fn to_file_bytes(&self) -> Option<Vec<u8>> {
        match self.format {
            SoundFormat::Opus => {
                let pcm = self.decode_opus().ok()?;
                let raw: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
                Sound { sample_rate: 48_000, format: SoundFormat::Pcm16, data: raw, packets: Vec::new(), ..*self }.to_file_bytes()
            }
            SoundFormat::Pcm16 => {
                let mut v = Vec::with_capacity(44 + self.data.len());
                let (ch, sr) = (self.channels as u16, self.sample_rate);
                v.extend(b"RIFF");
                v.extend((36 + self.data.len() as u32).to_le_bytes());
                v.extend(b"WAVEfmt ");
                v.extend(16u32.to_le_bytes());
                v.extend(1u16.to_le_bytes());
                v.extend(ch.to_le_bytes());
                v.extend(sr.to_le_bytes());
                v.extend((sr * ch as u32 * 2).to_le_bytes());
                v.extend((ch * 2).to_le_bytes());
                v.extend(16u16.to_le_bytes());
                v.extend(b"data");
                v.extend((self.data.len() as u32).to_le_bytes());
                v.extend(&self.data);
                Some(v)
            }
            SoundFormat::Mp3 | SoundFormat::Vorbis => Some(self.data.clone()),
            SoundFormat::Unknown(_) => None,
        }
    }
}

pub fn parse(file: &[u8]) -> Result<Sound> {
    let res = Resource::parse(file)?;
    let d = res.block_data("DATA").ok_or_else(|| anyhow::anyhow!("no DATA block"))?;
    ensure!(d.len() >= 16, "vsnd header truncated");
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let packed = u(0);
    let start = u32::from_le_bytes(file[0..4].try_into().unwrap()) as usize;
    ensure!(start <= file.len(), "vsnd payload offset out of range");
    let format = match (packed >> 16) as u8 {
        0 => SoundFormat::Pcm16,
        2 => SoundFormat::Mp3,
        4 => SoundFormat::Opus,
        5 => SoundFormat::Vorbis,
        f => SoundFormat::Unknown(f),
    };
    // Opus packet table: (cumulative end byte, cumulative end sample) pairs starting at u32 index 16.
    let mut packets = Vec::new();
    if format == SoundFormat::Opus {
        let words = d.len() / 4;
        let mut i = 16;
        while i + 1 < words {
            packets.push((u(i * 4), u(i * 4 + 4)));
            i += 2;
        }
        ensure!(packets.last().map_or(false, |p| p.0 as usize == file.len() - start), "opus packet table does not cover the payload");
    }
    Ok(Sound {
        sample_rate: packed & 0xFFFF,
        channels: ((packed >> 24) & 0xFF).max(1),
        format,
        samples: u(8),
        duration: f32::from_bits(u(12)),
        data: file[start..].to_vec(),
        packets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_wav_header_is_well_formed() {
        let s = Sound { sample_rate: 44100, channels: 2, format: SoundFormat::Pcm16, samples: 1, duration: 0.0, data: vec![0; 8], packets: Vec::new() };
        let w = s.to_file_bytes().unwrap();
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(w.len(), 44 + 8);
    }

    #[test]
    fn opus_music_decodes_to_header_length() {
        let Ok(b) = std::fs::read("E:/sbox-ref/game/addons/menu/Assets/music/menu-bg2.vsnd_c") else { return };
        let s = parse(&b).unwrap();
        assert_eq!(s.format, SoundFormat::Opus);
        let pcm = s.decode_opus().unwrap();
        let secs = pcm.len() as f32 / s.channels as f32 / 48_000.0;
        assert!((secs - s.duration).abs() < 0.1, "decoded {secs}s vs header {}s", s.duration);
        let rms = (pcm.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
        assert!(rms > 100.0, "decoded audio is silent (rms {rms})");
    }

    #[test]
    fn real_files_parse() {
        let base = "E:/sbox-ref/game/addons/menu/Assets/music/";
        for (name, fmt) in [("menu-avatar.vsnd_c", SoundFormat::Mp3), ("furniture_shop_loop.vsnd_c", SoundFormat::Vorbis)] {
            let Ok(b) = std::fs::read(format!("{base}{name}")) else { return };
            let s = parse(&b).unwrap();
            assert_eq!(s.format, fmt, "{name}");
            assert!(s.duration > 1.0 && !s.data.is_empty());
        }
    }
}
