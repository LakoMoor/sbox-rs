use sbox_math::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub joints: [u8; 4],
    pub weights: [f32; 4],
}

pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Axis-aligned box centred at the origin, in s&box units (X fwd, Y left, Z up).
    pub fn cuboid(size: Vec3) -> Self {
        let h = size * 0.5;
        // (normal, u axis, v axis)
        let faces = [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::NEG_Y, Vec3::Z),
            (Vec3::Y, Vec3::NEG_X, Vec3::Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::Y, Vec3::NEG_X),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ];
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (n, u, v) in faces {
            let base = vertices.len() as u32;
            for (su, sv, uv) in [(-1.0, -1.0, [0.0, 1.0]), (1.0, -1.0, [1.0, 1.0]), (1.0, 1.0, [1.0, 0.0]), (-1.0, 1.0, [0.0, 0.0])] {
                let p = (n + u * su + v * sv) * h;
                vertices.push(Vertex { position: p.to_array(), normal: n.to_array(), uv, joints: [0; 4], weights: [0.0; 4] });
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Self { vertices, indices }
    }

    /// Flat quad in the XY plane facing +Z.
    pub fn plane(size: f32) -> Self {
        let h = size * 0.5;
        let vertices = [(-h, -h, [0.0, 1.0]), (h, -h, [1.0, 1.0]), (h, h, [1.0, 0.0]), (-h, h, [0.0, 0.0])]
            .into_iter()
            .map(|(x, y, uv)| Vertex { position: [x, y, 0.0], normal: [0.0, 0.0, 1.0], uv, joints: [0; 4], weights: [0.0; 4] })
            .collect();
        Self { vertices, indices: vec![0, 1, 2, 0, 2, 3] }
    }

    pub fn sphere(radius: f32, segments: u32, rings: u32) -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for r in 0..=rings {
            let phi = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..=segments {
                let theta = std::f32::consts::TAU * s as f32 / segments as f32;
                let n = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
                vertices.push(Vertex {
                    position: (n * radius).to_array(),
                    normal: n.to_array(),
                    uv: [s as f32 / segments as f32, r as f32 / rings as f32],
                    joints: [0; 4],
                    weights: [0.0; 4],
                });
            }
        }
        let stride = segments + 1;
        for r in 0..rings {
            for s in 0..segments {
                let a = r * stride + s;
                let b = a + stride;
                indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        Self { vertices, indices }
    }
}

impl MeshData {
    /// Merge all draw calls of a Source 2 render mesh into one mesh.
    pub fn from_render_mesh(m: &sbox_resource::mesh::RenderMesh) -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for dc in &m.draw_calls {
            let base = vertices.len() as u32;
            vertices.extend(dc.vertices.iter().map(|v| Vertex { position: v.position, normal: v.normal, uv: v.uv, joints: v.joints, weights: v.weights }));
            indices.extend(dc.indices.iter().map(|i| i + base));
        }
        Self { vertices, indices }
    }
}

impl MeshData {
    pub fn from_draw_call(dc: &sbox_resource::mesh::DrawCall) -> Self {
        Self {
            vertices: dc.vertices.iter().map(|v| Vertex { position: v.position, normal: v.normal, uv: v.uv, joints: v.joints, weights: v.weights }).collect(),
            indices: dc.indices.clone(),
        }
    }
}
