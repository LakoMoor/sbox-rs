//! Binary KeyValues3 decoder (the DATA/MDAT/etc. blocks of resource files).

use anyhow::{bail, ensure, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Double(f64),
    String(String),
    Blob(Vec<u8>),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

pub struct Header {
    pub version: u32,
    pub format: [u8; 16],
    pub method: u32,
    pub dict_id: u16,
    pub frame_size: u16,
    pub n_bytes: u32,
    pub n_ints: u32,
    pub n_eights: u32,
    pub types_size: u32,
    pub n_objects: u16,
    pub n_arrays: u16,
    pub uncompressed: u32,
    pub compressed: u32,
    pub block_count: u32,
    pub block_total: u32,
    pub payload_start: usize,
}

fn rd32(d: &[u8], o: usize) -> Result<u32> {
    ensure!(o + 4 <= d.len(), "kv3 truncated");
    Ok(u32::from_le_bytes(d[o..o + 4].try_into().unwrap()))
}
fn rd16(d: &[u8], o: usize) -> Result<u16> {
    ensure!(o + 2 <= d.len(), "kv3 truncated");
    Ok(u16::from_le_bytes(d[o..o + 2].try_into().unwrap()))
}

pub fn read_header(d: &[u8]) -> Result<Header> {
    let magic = rd32(d, 0)?;
    ensure!(magic & 0xFFFFFF00 == 0x4B563300, "not KV3: magic {magic:#x}");
    let version = magic & 0xFF;
    ensure!(d.len() >= 20, "kv3 truncated");
    let format: [u8; 16] = d[4..20].try_into().unwrap();
    let mut o = 20;
    let method = rd32(d, o)?;
    o += 4;
    let dict_id = rd16(d, o)?;
    let frame_size = rd16(d, o + 2)?;
    o += 4;
    let n_bytes = rd32(d, o)?;
    let n_ints = rd32(d, o + 4)?;
    let n_eights = rd32(d, o + 8)?;
    o += 12;
    let types_size = rd32(d, o)?;
    let n_objects = rd16(d, o + 4)?;
    let n_arrays = rd16(d, o + 6)?;
    o += 8;
    let uncompressed = rd32(d, o)?;
    let compressed = rd32(d, o + 4)?;
    let block_count = rd32(d, o + 8)?;
    let block_total = rd32(d, o + 12)?;
    o += 16;
    Ok(Header {
        version, format, method, dict_id, frame_size, n_bytes, n_ints, n_eights, types_size, n_objects, n_arrays,
        uncompressed, compressed, block_count, block_total, payload_start: o,
    })
}

pub fn decompress(d: &[u8], h: &Header) -> Result<Vec<u8>> {
    let src = &d[h.payload_start..];
    match h.method {
        0 => Ok(src[..(h.uncompressed as usize).min(src.len())].to_vec()),
        1 => {
            ensure!(src.len() >= h.compressed as usize, "compressed data truncated");
            let out = lz4_flex::block::decompress(&src[..h.compressed as usize], h.uncompressed as usize)?;
            Ok(out)
        }
        m => bail!("unsupported kv3 compression {m}"),
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    bpos: usize,
    ints: &'a [u8],
    ipos: usize,
    eights: &'a [u8],
    epos: usize,
    types: &'a [u8],
    tpos: usize,
    strings: Vec<String>,
    /// External blobs (blocks) stored after the main buffer: concatenated data + ranges.
    blob_data: Vec<u8>,
    blob_ranges: Vec<(usize, usize)>,
    blob_next: usize,
}

impl<'a> Cursor<'a> {
    fn byte(&mut self) -> Result<u8> {
        let v = *self.bytes.get(self.bpos).ok_or_else(|| anyhow::anyhow!("kv3: byte stream exhausted"))?;
        self.bpos += 1;
        Ok(v)
    }
    fn int(&mut self) -> Result<i32> {
        ensure!(self.ipos + 4 <= self.ints.len(), "kv3: int stream exhausted");
        let v = i32::from_le_bytes(self.ints[self.ipos..self.ipos + 4].try_into().unwrap());
        self.ipos += 4;
        Ok(v)
    }
    fn eight(&mut self) -> Result<[u8; 8]> {
        ensure!(self.epos + 8 <= self.eights.len(), "kv3: eight stream exhausted");
        let v = self.eights[self.epos..self.epos + 8].try_into().unwrap();
        self.epos += 8;
        Ok(v)
    }
    fn ty(&mut self) -> Result<u8> {
        let v = *self.types.get(self.tpos).ok_or_else(|| anyhow::anyhow!("kv3: type stream exhausted"))?;
        self.tpos += 1;
        Ok(v)
    }
    fn string(&mut self) -> Result<String> {
        let id = self.int()?;
        if id < 0 {
            return Ok(String::new());
        }
        self.strings.get(id as usize).cloned().ok_or_else(|| anyhow::anyhow!("kv3: bad string id {id}"))
    }
    /// Reads a type byte, consuming the trailing flags byte when bit 7 is set.
    fn ty_flags(&mut self) -> Result<u8> {
        let t = self.ty()?;
        if t & 0x80 != 0 {
            self.ty()?;
        }
        Ok(t & 0x7f)
    }

    fn value(&mut self, t: u8) -> Result<Value> {
        Ok(match t {
            1 => Value::Null,
            2 => Value::Bool(self.byte()? != 0),
            3 => Value::Int(i64::from_le_bytes(self.eight()?)),
            4 => Value::UInt(u64::from_le_bytes(self.eight()?)),
            5 => Value::Double(f64::from_le_bytes(self.eight()?)),
            6 => Value::String(self.string()?),
            7 if !self.blob_ranges.is_empty() => {
                let (a, len) = *self.blob_ranges.get(self.blob_next).ok_or_else(|| anyhow::anyhow!("kv3: blob index out of range"))?;
                self.blob_next += 1;
                Value::Blob(self.blob_data[a..a + len].to_vec())
            }
            7 => {
                let n = self.int()? as usize;
                ensure!(self.bpos + n <= self.bytes.len(), "kv3: blob out of range");
                let v = self.bytes[self.bpos..self.bpos + n].to_vec();
                self.bpos += n;
                Value::Blob(v)
            }
            8 => {
                let n = self.int()?;
                let mut v = Vec::with_capacity(n.max(0) as usize);
                for _ in 0..n {
                    let t = self.ty_flags()?;
                    v.push(self.value(t)?);
                }
                Value::Array(v)
            }
            9 => {
                let n = self.int()?;
                let mut v = Vec::with_capacity(n.max(0) as usize);
                for _ in 0..n {
                    let name = self.string()?;
                    let t = self.ty_flags()?;
                    v.push((name, self.value(t)?));
                }
                Value::Object(v)
            }
            10 | 21 => {
                let n = if t == 10 { self.int()? } else { self.byte()? as i32 };
                let sub = self.ty_flags()?;
                let mut v = Vec::with_capacity(n.max(0) as usize);
                for _ in 0..n {
                    v.push(self.value(sub)?);
                }
                Value::Array(v)
            }
            11 => Value::Int(self.int()? as i64),
            12 => Value::UInt(self.int()? as u32 as u64),
            13 => Value::Bool(true),
            14 => Value::Bool(false),
            15 => Value::Int(0),
            16 => Value::Int(1),
            17 => Value::Double(0.0),
            18 => Value::Double(1.0),
            t => bail!("kv3: unsupported value type {t}"),
        })
    }
}

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

pub fn parse(d: &[u8]) -> Result<Value> {
    let h = read_header(d)?;
    let buf = decompress(d, &h)?;
    let n_bytes = h.n_bytes as usize;
    let ints_start = align(n_bytes, 4);
    let ints_end = ints_start + h.n_ints as usize * 4;
    let eights_start = align(ints_end, 8);
    let eights_end = eights_start + h.n_eights as usize * 8;
    ensure!(eights_end <= buf.len(), "kv3: buffers exceed data");
    let ints = &buf[ints_start..ints_end];
    ensure!(ints.len() >= 4, "kv3: no string count");
    let n_strings = u32::from_le_bytes(ints[0..4].try_into().unwrap()) as usize;

    let mut pos = eights_end;
    let mut strings = Vec::with_capacity(n_strings);
    for _ in 0..n_strings {
        let end = buf[pos..].iter().position(|&b| b == 0).ok_or_else(|| anyhow::anyhow!("kv3: unterminated string"))?;
        strings.push(String::from_utf8_lossy(&buf[pos..pos + end]).into_owned());
        pos += end + 1;
    }
    // After strings+types: [u32 blob size] * block_count, 00 DD EE FF, [u16 compressed size] * block_count
    let types_end = eights_end + h.types_size as usize;
    ensure!(pos <= types_end && types_end <= buf.len(), "kv3: no type data");
    let (mut blob_data, mut blob_ranges) = (Vec::new(), Vec::new());
    if h.block_count > 0 {
        let n = h.block_count as usize;
        let sizes_end = types_end + 4 * n;
        ensure!(sizes_end + 4 + 2 * n <= buf.len(), "kv3: blob tables truncated");
        let sizes: Vec<usize> =
            (0..n).map(|i| u32::from_le_bytes(buf[types_end + i * 4..types_end + i * 4 + 4].try_into().unwrap()) as usize).collect();
        let comp: Vec<usize> = (0..n)
            .map(|i| u16::from_le_bytes([buf[sizes_end + 4 + i * 2], buf[sizes_end + 4 + i * 2 + 1]]) as usize)
            .collect();
        let tail = &d[h.payload_start + h.compressed as usize..];
        blob_data.reserve(h.block_total as usize);
        let mut tpos = 0usize;
        for (size, c) in sizes.iter().zip(&comp) {
            ensure!(tpos + c <= tail.len(), "kv3: blob data truncated");
            let src = &tail[tpos..tpos + c];
            tpos += c;
            let start = blob_data.len();
            if *c == *size {
                blob_data.extend_from_slice(src);
            } else {
                // LZ4 blocks are linked: matches may reference up to 64 KiB of earlier output.
                let dict_start = start.saturating_sub(65536);
                let out = lz4_flex::block::decompress_with_dict(src, *size, &blob_data[dict_start..start])?;
                blob_data.extend_from_slice(&out);
            }
            blob_ranges.push((start, *size));
        }
    }
    let mut c = Cursor {
        bytes: &buf[..n_bytes],
        bpos: 0,
        ints,
        ipos: 4,
        eights: &buf[eights_start..eights_end],
        epos: 0,
        types: &buf[pos..types_end],
        tpos: 0,
        strings,
        blob_data,
        blob_ranges,
        blob_next: 0,
    };
    let t = c.ty_flags()?;
    let v = c.value(t)?;
    ensure!(c.blob_next == c.blob_ranges.len(), "kv3: {} of {} external blobs consumed", c.blob_next, c.blob_ranges.len());
    Ok(v)
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(v) => v.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Int(i) => Some(*i as f64),
            Value::UInt(u) => Some(*u as f64),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::UInt(u) => Some(*u as i64),
            Value::Double(d) => Some(*d as i64),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
