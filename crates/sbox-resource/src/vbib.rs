//! Vertex / index buffer block (`MBUF`, a.k.a. VBIB) of model and mesh resources.
//!
//! ```text
//! u32 vb_offset, u32 vb_count, u32 ib_offset, u32 ib_count        (offsets relative to own field)
//! vertex buffer header (24 bytes):
//!   u32 vertex_count, u32 stride, u32 attr_offset, u32 attr_count, u32 data_offset, u32 data_size
//! index buffer header (24 bytes):
//!   u32 index_count, u32 index_size, u32 _, u32 _, u32 data_offset, u32 data_size
//! attribute (56 bytes): char name[32], u32 semantic_index, u32 dxgi_format, u32 offset, u32 slot, u32 slot_type, u32 step_rate
//! ```
//! When `data_size` is smaller than `count * stride`, the data is meshoptimizer-encoded.

use anyhow::{bail, ensure, Result};

#[derive(Clone, Debug)]
pub struct Attribute {
    pub name: String,
    pub semantic_index: u32,
    /// DXGI_FORMAT value.
    pub format: u32,
    pub offset: u32,
    pub slot: u32,
}

#[derive(Clone, Debug)]
pub struct VertexBuffer {
    pub count: u32,
    pub stride: u32,
    pub attributes: Vec<Attribute>,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct IndexBuffer {
    pub count: u32,
    /// 2 or 4 bytes per index.
    pub index_size: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Vbib {
    pub vertex_buffers: Vec<VertexBuffer>,
    pub index_buffers: Vec<IndexBuffer>,
}

fn u32_at(d: &[u8], o: usize) -> Result<usize> {
    ensure!(o + 4 <= d.len(), "vbib truncated at {o}");
    Ok(u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize)
}

pub fn parse(d: &[u8]) -> Result<Vbib> {
    let vb_off = u32_at(d, 0)?;
    let vb_count = u32_at(d, 4)?;
    let ib_off = 8 + u32_at(d, 8)?;
    let ib_count = u32_at(d, 12)?;
    let mut out = Vbib::default();

    for i in 0..vb_count {
        let h = vb_off + i * 24;
        let count = u32_at(d, h)?;
        let stride = u32_at(d, h + 4)?;
        let attr_off = h + 8 + u32_at(d, h + 8)?;
        let attr_count = u32_at(d, h + 12)?;
        let data_off = h + 16 + u32_at(d, h + 16)?;
        let data_size = u32_at(d, h + 20)?;
        ensure!(data_off + data_size <= d.len(), "vertex data out of range");

        let mut attributes = Vec::new();
        for a in 0..attr_count {
            let p = attr_off + a * 56;
            ensure!(p + 56 <= d.len(), "attribute out of range");
            let name_bytes = &d[p..p + 32];
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
            attributes.push(Attribute {
                name: String::from_utf8_lossy(&name_bytes[..end]).into_owned(),
                semantic_index: u32_at(d, p + 32)? as u32,
                format: u32_at(d, p + 36)? as u32,
                offset: u32_at(d, p + 40)? as u32,
                slot: u32_at(d, p + 44)? as u32,
            });
        }

        let src = &d[data_off..data_off + data_size];
        let raw_size = count * stride;
        let data = if data_size == raw_size {
            src.to_vec()
        } else {
            let mut out = vec![0u8; raw_size];
            // SAFETY: buffer sizes are exactly count*stride / src.len().
            let rc = unsafe {
                meshopt::ffi::meshopt_decodeVertexBuffer(out.as_mut_ptr().cast(), count, stride, src.as_ptr(), src.len())
            };
            if rc != 0 {
                bail!("meshopt vertex decode failed ({rc}) count={count} stride={stride}");
            }
            out
        };
        out.vertex_buffers.push(VertexBuffer { count: count as u32, stride: stride as u32, attributes, data });
    }

    for i in 0..ib_count {
        let h = ib_off + i * 24;
        let count = u32_at(d, h)?;
        let index_size = u32_at(d, h + 4)?;
        let data_off = h + 16 + u32_at(d, h + 16)?;
        let data_size = u32_at(d, h + 20)?;
        ensure!(data_off + data_size <= d.len(), "index data out of range");
        ensure!(index_size == 2 || index_size == 4, "bad index size {index_size}");
        let src = &d[data_off..data_off + data_size];
        let raw_size = count * index_size;
        let data = if data_size == raw_size {
            src.to_vec()
        } else {
            let mut out = vec![0u8; raw_size];
            let rc = unsafe {
                meshopt::ffi::meshopt_decodeIndexBuffer(out.as_mut_ptr().cast(), count, index_size, src.as_ptr(), src.len())
            };
            if rc != 0 {
                bail!("meshopt index decode failed ({rc}) count={count}");
            }
            out
        };
        out.index_buffers.push(IndexBuffer { count: count as u32, index_size: index_size as u32, data });
    }
    Ok(out)
}
