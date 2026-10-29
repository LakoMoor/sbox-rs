//! `MeshComponent` (Hammer / in-editor polygon meshes): decodes the serialised half-edge `Topology`
//! blob (base64 of a gzip stream of little-endian i32s) into triangles grouped by material.
//!
//! Blob layout: `vertex_count, half_edge_count, face_count`, then one outgoing half-edge index per
//! vertex, then `half_edge_count` records of `(vertex, twin, next, face)`, then one half-edge index
//! per face. Half-edge `i` is parameterised by `TextureCoord[i]`.

use anyhow::{bail, ensure, Result};
use base64::Engine;
use sbox_math::Vec3;
use sbox_render::{MeshData, Vertex};
use std::io::Read;

pub struct Face {
    /// Half-edge indices around the face, in `next` order.
    pub half_edges: Vec<usize>,
    /// Vertex index of each half-edge.
    pub vertices: Vec<usize>,
}

pub struct Topology {
    pub vertex_count: usize,
    pub faces: Vec<Face>,
}

pub fn decode(topology_b64: &str) -> Result<Topology> {
    let raw = base64::engine::general_purpose::STANDARD.decode(topology_b64.trim())?;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut bytes)?;
    ensure!(bytes.len() % 4 == 0 && bytes.len() >= 12, "bad topology size {}", bytes.len());
    let ints: Vec<i32> = bytes.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
    let (nv, ne, nf) = (ints[0] as usize, ints[1] as usize, ints[2] as usize);
    ensure!(ints.len() == 3 + nv + ne * 4 + nf, "topology length {} does not match {nv} vertices, {ne} half-edges, {nf} faces", ints.len());
    let records = &ints[3 + nv..3 + nv + ne * 4];
    let vertex = |e: usize| records[e * 4] as usize;
    let next = |e: usize| records[e * 4 + 2] as usize;
    let face_of = |e: usize| records[e * 4 + 3] as usize;
    let first_edges = &ints[3 + nv + ne * 4..];
    let mut faces = Vec::with_capacity(nf);
    for (f, &start) in first_edges.iter().enumerate() {
        let start = start as usize;
        ensure!(start < ne, "face {f} starts at bad half-edge {start}");
        let mut half_edges = vec![start];
        let mut e = next(start);
        while e != start {
            ensure!(half_edges.len() <= ne, "face {f} loop does not close");
            ensure!(face_of(e) == f, "half-edge {e} is not on face {f}");
            half_edges.push(e);
            e = next(e);
        }
        let vertices = half_edges.iter().map(|&e| vertex(e)).collect();
        faces.push(Face { half_edges, vertices });
    }
    Ok(Topology { vertex_count: nv, faces })
}

fn parse_floats<const N: usize>(s: &str) -> Option<[f32; N]> {
    let mut it = s.split(',').map(|v| v.trim().parse::<f32>());
    let mut out = [0.0; N];
    for o in &mut out {
        *o = it.next()?.ok()?;
    }
    Some(out)
}

/// Triangulate the component's faces into one mesh per material slot: `(material index, mesh)`.
pub fn build(p: &serde_json::Value) -> Result<Vec<(usize, MeshData)>> {
    let Some(topo) = p.get("Mesh").and_then(|m| m.get("Topology")).and_then(|t| t.as_str()) else { bail!("no Topology") };
    let mesh = p.get("Mesh").unwrap();
    let topo = decode(topo)?;
    let strings = |k: &str| -> Vec<String> { mesh.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default() };
    let positions: Vec<Vec3> = strings("Positions").iter().filter_map(|s| parse_floats::<3>(s)).map(Vec3::from).collect();
    ensure!(positions.len() == topo.vertex_count, "{} positions for {} vertices", positions.len(), topo.vertex_count);
    let uvs: Vec<[f32; 2]> = strings("TextureCoord").iter().map(|s| parse_floats::<2>(s).unwrap_or([0.0; 2])).collect();
    let material_index: Vec<usize> = mesh.get("MaterialIndex").and_then(|v| v.as_array()).map(|a| a.iter().map(|v| v.as_i64().unwrap_or(0).max(0) as usize).collect()).unwrap_or_default();

    let mut groups: Vec<(usize, MeshData)> = Vec::new();
    for (fi, face) in topo.faces.iter().enumerate() {
        if face.vertices.len() < 3 {
            continue;
        }
        let pts: Vec<Vec3> = face.vertices.iter().map(|&v| positions[v]).collect();
        let normal = (pts[1] - pts[0]).cross(pts[2] - pts[0]).normalize_or_zero();
        let mat = material_index.get(fi).copied().unwrap_or(0);
        let g = match groups.iter().position(|(m, _)| *m == mat) {
            Some(i) => i,
            None => {
                groups.push((mat, MeshData { vertices: vec![], indices: vec![] }));
                groups.len() - 1
            }
        };
        let md = &mut groups[g].1;
        let base = md.vertices.len() as u32;
        for (k, &pt) in pts.iter().enumerate() {
            let uv = uvs.get(face.half_edges[k]).copied().unwrap_or([0.0; 2]);
            md.vertices.push(Vertex { position: pt.to_array(), normal: normal.to_array(), uv, joints: [0; 4], weights: [0.0; 4] });
        }
        for k in 1..pts.len() as u32 - 1 {
            md.indices.extend([base, base + k, base + k + 1]);
        }
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Topology of a six-faced box from `menu-vr.scene`.
    const BOX: &str = "H4sIAAAAAAAACl2QCQ7CMAwEp4Qj3NBy/f+laMRGsohUVbFn1+t04ANs+Z0VsM69AwfgBFyAKZ9MPXdgk3qLnsI+Um/h/me9i2ZXfFvYOazafTJN0XRgKZmPyWtfXvYVL73PZRd52WdYPa7xtC8veys7u6t17/7t6Tn0ZtFjvIXZR895zjKjPWf4NiOrGt+ivpXZ3XnozaOndVn9vvVkkTjEAQAA";

    #[test]
    fn decodes_box_topology() {
        let t = decode(BOX).unwrap();
        assert_eq!((t.vertex_count, t.faces.len()), (8, 6));
        assert!(t.faces.iter().all(|f| f.vertices.len() == 4));
        // Every vertex of a box belongs to three faces.
        let mut uses = vec![0; 8];
        t.faces.iter().flat_map(|f| &f.vertices).for_each(|&v| uses[v] += 1);
        assert!(uses.iter().all(|&u| u == 3), "{uses:?}");
    }

    #[test]
    fn face_normals_point_away_from_the_centre() {
        let positions = ["-912,-960,224", "912,-960,224", "912,960,224", "-912,960,224", "-912,960,-224", "912,960,-224", "912,-960,-224", "-912,-960,-224"];
        let j = serde_json::json!({ "Mesh": { "Topology": BOX, "Positions": positions, "MaterialIndex": [1, 0, 0, 1, 0, 0] } });
        let groups = build(&j).unwrap();
        assert_eq!(groups.len(), 2);
        let tris: usize = groups.iter().map(|(_, m)| m.indices.len() / 3).sum();
        assert_eq!(tris, 12);
        for (_, m) in &groups {
            for tri in m.indices.chunks(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(m.vertices[i as usize].position));
                let n = (b - a).cross(c - a);
                assert!(n.dot((a + b + c) / 3.0) > 0.0, "triangle winds inwards");
            }
        }
    }
}
