//! Render mesh extraction: combines the KV3 mesh description (`MDAT` / `DATA`)
//! with the decoded vertex/index buffers (`MBUF`).

use crate::vbib::{Attribute, Vbib, VertexBuffer};
use crate::{kv3, Resource, Value};
use anyhow::{anyhow, bail, ensure, Result};

#[derive(Clone, Copy, Debug, Default)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct DrawCall {
    pub material: String,
    pub vertices: Vec<MeshVertex>,
    /// Indices relative to `vertices`.
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct RenderMesh {
    pub draw_calls: Vec<DrawCall>,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
}

pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if man == 0 {
            sign << 31
        } else {
            // subnormal
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            (sign << 31) | ((e as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 31 {
        (sign << 31) | 0x7f80_0000 | (man << 13)
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}

fn f32_at(d: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}
fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

/// Octahedral decode of a point in [-1,1]^2 to a unit vector.
fn oct_decode(x: f32, y: f32) -> [f32; 3] {
    let z = 1.0 - x.abs() - y.abs();
    let (mut nx, mut ny) = (x, y);
    if z < 0.0 {
        nx = (1.0 - y.abs()) * if x >= 0.0 { 1.0 } else { -1.0 };
        ny = (1.0 - x.abs()) * if y >= 0.0 { 1.0 } else { -1.0 };
    }
    let l = (nx * nx + ny * ny + z * z).sqrt().max(1e-6);
    [nx / l, ny / l, z / l]
}

/// Unpack a normal from the given attribute bytes. With `compressed`
/// (`m_bUseCompressedNormalTangent`) the first two components are an octahedral
/// normal and the rest holds the tangent frame.
fn decode_normal(fmt: u32, b: &[u8], compressed: bool) -> [f32; 3] {
    match fmt {
        6 => [f32_at(b, 0), f32_at(b, 4), f32_at(b, 8)],
        24 => {
            // R10G10B10A2_UNORM
            let v = u32::from_le_bytes(b[0..4].try_into().unwrap());
            let c = |s: u32| ((v >> s) & 0x3ff) as f32 / 1023.0 * 2.0 - 1.0;
            if compressed {
                oct_decode(c(0), c(10))
            } else {
                [c(0), c(10), c(20)]
            }
        }
        28 => {
            let c = |i: usize| b[i] as f32 / 255.0 * 2.0 - 1.0;
            if compressed {
                oct_decode(c(0), c(1))
            } else {
                [c(0), c(1), c(2)]
            }
        }
        _ => [0.0, 0.0, 1.0],
    }
}

fn decode_uv(fmt: u32, b: &[u8]) -> [f32; 2] {
    match fmt {
        16 => [f32_at(b, 0), f32_at(b, 4)],
        34 => [f16_to_f32(u16_at(b, 0)), f16_to_f32(u16_at(b, 2))],
        28 => [b[0] as f32 / 255.0, b[1] as f32 / 255.0],
        _ => [0.0, 0.0],
    }
}

fn attr<'a>(vb: &'a VertexBuffer, name: &str) -> Option<&'a Attribute> {
    vb.attributes.iter().find(|a| a.name.eq_ignore_ascii_case(name) && a.semantic_index == 0)
}

fn read_vertex(vb: &VertexBuffer, i: usize, compressed: bool) -> MeshVertex {
    let base = i * vb.stride as usize;
    let v = &vb.data[base..base + vb.stride as usize];
    let mut out = MeshVertex { normal: [0.0, 0.0, 1.0], ..Default::default() };
    if let Some(a) = attr(vb, "POSITION") {
        let o = a.offset as usize;
        out.position = [f32_at(v, o), f32_at(v, o + 4), f32_at(v, o + 8)];
    }
    if let Some(a) = attr(vb, "NORMAL") {
        out.normal = decode_normal(a.format, &v[a.offset as usize..], compressed);
    }
    if let Some(a) = attr(vb, "TEXCOORD") {
        out.uv = decode_uv(a.format, &v[a.offset as usize..]);
    }
    out
}

fn vec3(v: &Value) -> [f32; 3] {
    let a = v.as_array().unwrap_or(&[]);
    let g = |i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [g(0), g(1), g(2)]
}

/// Build a render mesh from a `CRenderMesh` KV3 and its buffers.
pub fn build(mesh_kv: &Value, vbib: &Vbib) -> Result<RenderMesh> {
    let mut out = RenderMesh { mins: [f32::MAX; 3], maxs: [f32::MIN; 3], ..Default::default() };
    let objects = mesh_kv.get("m_sceneObjects").and_then(Value::as_array).ok_or_else(|| anyhow!("no m_sceneObjects"))?;
    for so in objects {
        if let Some(m) = so.get("m_vMinBounds") {
            let m = vec3(m);
            (0..3).for_each(|i| out.mins[i] = out.mins[i].min(m[i]));
        }
        if let Some(m) = so.get("m_vMaxBounds") {
            let m = vec3(m);
            (0..3).for_each(|i| out.maxs[i] = out.maxs[i].max(m[i]));
        }
        for dc in so.get("m_drawCalls").and_then(Value::as_array).unwrap_or(&[]) {
            let geti = |k: &str| dc.get(k).and_then(Value::as_i64).unwrap_or(0);
            if geti("m_nInstanceCount") > 1 && false {
                continue;
            }
            let base_vertex = geti("m_nBaseVertex") as usize;
            let vertex_count = geti("m_nVertexCount") as usize;
            let start_index = geti("m_nStartIndex") as usize;
            let index_count = geti("m_nIndexCount") as usize;
            let compressed = matches!(dc.get("m_bUseCompressedNormalTangent"), Some(Value::Bool(true)));
            let vb_idx = dc
                .get("m_vertexBuffers")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|v| v.get("m_hBuffer"))
                .and_then(Value::as_i64)
                .unwrap_or(0) as usize;
            let ib_idx = dc.get("m_indexBuffer").and_then(|v| v.get("m_hBuffer")).and_then(Value::as_i64).unwrap_or(0) as usize;
            let vb = vbib.vertex_buffers.get(vb_idx).ok_or_else(|| anyhow!("bad vertex buffer {vb_idx}"))?;
            let ib = vbib.index_buffers.get(ib_idx).ok_or_else(|| anyhow!("bad index buffer {ib_idx}"))?;
            ensure!(base_vertex + vertex_count <= vb.count as usize, "draw call exceeds vertex buffer");
            ensure!(start_index + index_count <= ib.count as usize, "draw call exceeds index buffer");
            match dc.get("m_nPrimitiveType").and_then(Value::as_str) {
                Some("RENDER_PRIM_TRIANGLES") | None => {}
                Some(other) => bail!("unsupported primitive type {other}"),
            }
            let vertices = (0..vertex_count).map(|i| read_vertex(vb, base_vertex + i, compressed)).collect();
            let sz = ib.index_size as usize;
            let indices = (start_index..start_index + index_count)
                .map(|i| {
                    if sz == 2 {
                        u16_at(&ib.data, i * 2) as u32
                    } else {
                        u32::from_le_bytes(ib.data[i * 4..i * 4 + 4].try_into().unwrap())
                    }
                })
                .collect();
            out.draw_calls.push(DrawCall {
                material: dc.get("m_material").and_then(Value::as_str).unwrap_or("").to_string(),
                vertices,
                indices,
            });
        }
    }
    Ok(out)
}

/// Load the embedded mesh of a `.vmdl_c` / `.vmesh_c` file.
pub fn load_embedded(file: &[u8]) -> Result<RenderMesh> {
    let res = Resource::parse(file)?;
    let kv = if let Some(m) = res.block_data("MDAT") {
        kv3::parse(m)?
    } else {
        kv3::parse(res.block_data("DATA").ok_or_else(|| anyhow!("no mesh data block"))?)?
    };
    let mbuf = res.block_data("MBUF").ok_or_else(|| anyhow!("no MBUF block"))?;
    let vbib = crate::vbib::parse(mbuf)?;
    build(&kv, &vbib)
}
