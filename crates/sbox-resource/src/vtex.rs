//! `.vtex_c` texture loader.
//!
//! DATA block layout (little endian):
//! ```text
//! u16 version, u16 flags, f32[4] reflectivity,
//! u16 width, u16 height, u16 depth, u8 format, u8 mip_count,
//! u32 picmip0_res, u32 extra_offset, u32 extra_count
//! ```
//! Pixel data is appended after the resource blocks, smallest mip first.

use crate::Resource;
use anyhow::{ensure, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TexFormat {
    Dxt1,
    Dxt5,
    I8,
    Rgba8888,
    R16,
    Rg1616,
    Rgba16161616,
    R16F,
    Rg1616F,
    Rgba16161616F,
    R32F,
    Rg3232F,
    Rgb323232F,
    Rgba32323232F,
    Bc6h,
    Bc7,
    Ati2n,
    Ia88,
    Ati1n,
    Bgra8888,
    Other(u8),
}

impl TexFormat {
    fn from_id(id: u8) -> Self {
        match id {
            1 => Self::Dxt1,
            2 => Self::Dxt5,
            3 => Self::I8,
            4 => Self::Rgba8888,
            5 => Self::R16,
            6 => Self::Rg1616,
            7 => Self::Rgba16161616,
            8 => Self::R16F,
            9 => Self::Rg1616F,
            10 => Self::Rgba16161616F,
            11 => Self::R32F,
            12 => Self::Rg3232F,
            13 => Self::Rgb323232F,
            14 => Self::Rgba32323232F,
            19 => Self::Bc6h,
            20 => Self::Bc7,
            21 => Self::Ati2n,
            22 => Self::Ia88,
            27 => Self::Ati1n,
            28 => Self::Bgra8888,
            n => Self::Other(n),
        }
    }

    /// (block width/height, bytes per block) or None for unsupported.
    pub fn block_info(self) -> Option<(u32, u32)> {
        use TexFormat::*;
        Some(match self {
            Dxt1 | Ati1n => (4, 8),
            Dxt5 | Bc6h | Bc7 | Ati2n => (4, 16),
            I8 => (1, 1),
            R16 | R16F | Ia88 => (1, 2),
            Rgba8888 | Bgra8888 | Rg1616 | Rg1616F | R32F => (1, 4),
            Rgba16161616 | Rgba16161616F | Rg3232F => (1, 8),
            Rgb323232F => (1, 12),
            Rgba32323232F => (1, 16),
            Other(_) => return None,
        })
    }

    pub fn is_compressed(self) -> bool {
        self.block_info().map_or(false, |(b, _)| b == 4)
    }

    /// Size in bytes of one image of the given dimensions.
    pub fn image_size(self, w: u32, h: u32) -> Option<usize> {
        let (b, bytes) = self.block_info()?;
        Some((w.div_ceil(b) * h.div_ceil(b) * bytes) as usize)
    }
}

pub mod flags {
    pub const CLAMP_S: u16 = 0x01;
    pub const CLAMP_T: u16 = 0x02;
    pub const CLAMP_U: u16 = 0x04;
    pub const NO_LOD: u16 = 0x08;
    pub const CUBE: u16 = 0x10;
    pub const VOLUME: u16 = 0x20;
    pub const ARRAY: u16 = 0x40;
}

#[derive(Debug)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub format: TexFormat,
    pub flags: u16,
    pub reflectivity: [f32; 4],
    /// Mip levels, largest first. Each holds `depth` (or 6 faces for cubemaps) images back to back.
    pub mips: Vec<Vec<u8>>,
    /// Sprite-sheet frames of the first sequence: UV origin of each cell (cells are `1 / ceil(sqrt(n))` wide).
    pub sheet: Vec<[f32; 2]>,
}

impl Texture {
    pub fn is_cube(&self) -> bool {
        self.flags & flags::CUBE != 0
    }
    pub fn mip_size(&self, level: usize) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

pub fn parse(file: &[u8]) -> Result<Texture> {
    let res = Resource::parse(file)?;
    let d = res.block_data("DATA").ok_or_else(|| anyhow::anyhow!("no DATA block"))?;
    ensure!(d.len() >= 40, "vtex header truncated");
    let version = u16_at(d, 0);
    ensure!(version == 1, "unsupported vtex version {version}");
    let flags = u16_at(d, 2);
    let mut reflectivity = [0f32; 4];
    for (i, r) in reflectivity.iter_mut().enumerate() {
        *r = f32::from_le_bytes(d[4 + i * 4..8 + i * 4].try_into().unwrap());
    }
    let width = u16_at(d, 20) as u32;
    let height = u16_at(d, 22) as u32;
    let depth = (u16_at(d, 24) as u32).max(1);
    let format = TexFormat::from_id(d[26]);
    let mip_count = d[27] as usize;
    let extra_offset = u32_at(d, 32) as usize;
    let extra_count = u32_at(d, 36) as usize;

    // Extra data table: entries of (type, offset, size) relative to their own position.
    let mut compressed_sizes: Option<Vec<u32>> = None;
    let mut compression = 0u32;
    let mut sheet: Vec<[f32; 2]> = Vec::new();
    for i in 0..extra_count {
        let e = 32 + extra_offset + i * 12;
        if e + 12 > d.len() {
            break;
        }
        let ty = u32_at(d, e);
        let off = e + 4 + u32_at(d, e + 4) as usize;
        let _size = u32_at(d, e + 8) as usize;
        // type 2 = sprite sheet: header { u32 n, u32 sequences, .. u32 rel_offset @16, u32 frame_count @20 };
        // each frame is { f32 duration, u32 rel_offset, u32 images }, rel_offset points (from its own field)
        // to an image record whose f32s [4], [5] are the UV origin of the cell.
        if ty == 2 && off + 24 <= d.len() {
            let seq = off + 16 + u32_at(d, off + 16) as usize;
            let n = u32_at(d, off + 20) as usize;
            for k in 0..n.min(4096) {
                let f = seq + k * 12;
                if f + 12 > d.len() {
                    break;
                }
                let rec = f + 4 + u32_at(d, f + 4) as usize;
                if rec + 24 > d.len() {
                    break;
                }
                let fl = |o: usize| f32::from_bits(u32_at(d, rec + o));
                sheet.push([fl(16), fl(20)]);
            }
        }
        // type 4 = compressed mip sizes: { u32 compressed, u32 table_offset (from its own field), u32 count }
        if ty == 4 && off + 12 <= d.len() {
            compression = u32_at(d, off);
            let is_compressed = compression != 0;
            let table = off + 4 + u32_at(d, off + 4) as usize;
            let count = u32_at(d, off + 8) as usize;
            ensure!(table + count * 4 <= d.len(), "compressed mip table out of range");
            if is_compressed {
                compressed_sizes = Some((0..count).map(|k| u32_at(d, table + k * 4)).collect());
            }
        }
    }

    if format == TexFormat::Other(16) {
        // PNG_RGBA8888: a single PNG stream follows the header.
        let tail = u32_at(file, 0) as usize;
        let mut dec = png::Decoder::new(std::io::Cursor::new(&file[tail..]));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info()?;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf)?;
        buf.truncate(info.buffer_size());
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            c => anyhow::bail!("unsupported png color type {c:?}"),
        };
        return Ok(Texture {
            width: info.width,
            height: info.height,
            depth: 1,
            format: TexFormat::Rgba8888,
            flags,
            reflectivity,
            mips: vec![rgba],
            sheet: Vec::new(),
        });
    }

    let faces = if flags & flags::CUBE != 0 { 6 } else { 1 };
    let layers = if flags & flags::CUBE != 0 { faces } else { depth as usize };
    let tail_start = u32_at(file, 0) as usize;
    let mut pos = tail_start;
    let mut mips = vec![Vec::new(); mip_count];
    // smallest mip first
    for level in (0..mip_count).rev() {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let one = format.image_size(w, h).ok_or_else(|| anyhow::anyhow!("unsupported texture format {format:?}"))?;
        let n = one * layers;
        match &compressed_sizes {
            Some(sizes) => {
                let c = *sizes.get(level).ok_or_else(|| anyhow::anyhow!("missing compressed size for mip {level}"))? as usize;
                ensure!(pos + c <= file.len(), "compressed mip {level} out of range");
                mips[level] = if c >= n {
                    file[pos..pos + n].to_vec()
                } else {
                    match compression {
                        1 => lz4_flex::block::decompress(&file[pos..pos + c], n)?,
                        2 => zstd_decode(&file[pos..pos + c], n)?,
                        m => anyhow::bail!("unknown mip compression {m}"),
                    }
                };
                pos += c;
            }
            None => {
                ensure!(pos + n <= file.len(), "mip {level} out of range ({format:?} {w}x{h})");
                mips[level] = file[pos..pos + n].to_vec();
                pos += n;
            }
        }
    }
    ensure!(pos == file.len(), "texture data size mismatch: used {} of {}", pos - tail_start, file.len() - tail_start);
    Ok(Texture { width, height, depth, format, flags, reflectivity, mips, sheet })
}

fn zstd_decode(src: &[u8], expected: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut dec = ruzstd::StreamingDecoder::new(src).map_err(|e| anyhow::anyhow!("zstd: {e}"))?;
    let mut out = Vec::with_capacity(expected);
    dec.read_to_end(&mut out)?;
    ensure!(out.len() == expected, "zstd size mismatch: {} != {expected}", out.len());
    Ok(out)
}

#[cfg(test)]
mod sheet_tests {
    use super::*;

    #[test]
    fn explosion_sheet_has_64_cells_on_an_8x8_grid() {
        let Ok(b) = std::fs::read("E:/sbox-ref/game/core/textures/particles/flame/explosion/explosion_01.vtex_c") else { return };
        let t = parse(&b).unwrap();
        assert_eq!(t.sheet.len(), 64);
        // frame 1 sits one cell down, frame 8 one cell right
        assert!((t.sheet[1][1] - 0.125).abs() < 0.01 && t.sheet[1][0] < 0.01, "{:?}", t.sheet[1]);
        assert!((t.sheet[8][0] - 0.125).abs() < 0.01 && t.sheet[8][1] < 0.01, "{:?}", t.sheet[8]);
    }
}
