//! Collision shapes of a model: the `PHYS` block of a `vmdl_c` or the `vphys_c` it references (`m_refPhysicsData`).
//! Hulls, spheres, capsules and triangle meshes (`m_Vertices` f32x3 and `m_Triangles` u32x3 blobs) are read.

use crate::{kv3, Resource, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum PhysShape {
    /// Convex hull vertices in model space.
    Hull(Vec<[f32; 3]>),
    Sphere { center: [f32; 3], radius: f32 },
    Capsule { a: [f32; 3], b: [f32; 3], radius: f32 },
    /// Triangle soup in model space.
    Mesh { vertices: Vec<[f32; 3]>, indices: Vec<[u32; 3]> },
}

fn vec3(v: &Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
}

/// Bone transform of a part: `m_bindPose[i]` is a row-major 3x4 matrix (rotation rows with the translation in the last column).
fn bind(kv: &Value, part: usize) -> Option<[f32; 12]> {
    let m = kv.get("m_bindPose")?.as_array()?.get(part)?.as_array()?;
    let mut out = [0.0; 12];
    if m.len() != 12 {
        return None;
    }
    for (o, v) in out.iter_mut().zip(m) {
        *o = v.as_f64()? as f32;
    }
    Some(out)
}

fn place(m: &Option<[f32; 12]>, p: [f32; 3]) -> [f32; 3] {
    let Some(m) = m else { return p };
    let row = |r: usize| m[r * 4] * p[0] + m[r * 4 + 1] * p[1] + m[r * 4 + 2] * p[2] + m[r * 4 + 3];
    [row(0), row(1), row(2)]
}

/// Shapes of a physics KV3 tree (`m_parts[].m_rnShape`) in the model's rest pose: every part's shapes are moved from
/// its bone space with `m_bindPose[part]`. Joints (`m_joints`) are not read, so the parts stay rigidly together.
pub fn shapes(kv: &Value) -> Option<Vec<PhysShape>> {
    let parts = kv.get("m_parts").and_then(Value::as_array)?;
    let mut out = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        out.extend(part_shapes(part, &bind(kv, i))?);
    }
    (!out.is_empty()).then_some(out)
}

/// The shapes of one part, moved by `m_bind` (`None`: left in the part's own space).
fn part_shapes(part: &Value, m_bind: &Option<[f32; 12]>) -> Option<Vec<PhysShape>> {
    let m = *m_bind;
    let mut out = Vec::new();
    {
        let sh = part.get("m_rnShape")?;
        let list = |k: &str| sh.get(k).and_then(Value::as_array).unwrap_or(&[]);
        for h in list("m_hulls") {
            let verts: Vec<[f32; 3]> = h.get("m_Hull")?.get("m_Vertices").and_then(Value::as_array).unwrap_or(&[]).iter().filter_map(vec3).map(|v| place(&m, v)).collect();
            if verts.len() >= 4 {
                out.push(PhysShape::Hull(verts));
            }
        }
        for s in list("m_spheres") {
            let s = s.get("m_Sphere")?;
            out.push(PhysShape::Sphere { center: place(&m, vec3(s.get("m_vCenter")?)?), radius: s.get("m_flRadius")?.as_f64()? as f32 });
        }
        for c in list("m_capsules") {
            let c = c.get("m_Capsule")?;
            let ends = c.get("m_vCenter")?.as_array()?;
            out.push(PhysShape::Capsule { a: place(&m, vec3(ends.first()?)?), b: place(&m, vec3(ends.get(1)?)?), radius: c.get("m_flRadius")?.as_f64()? as f32 });
        }
        for m in list("m_meshes") {
            let Some(mesh) = m.get("m_Mesh") else { continue };
            let (Some(Value::Blob(vb)), Some(Value::Blob(tb))) = (mesh.get("m_Vertices"), mesh.get("m_Triangles")) else { continue };
            let f = |c: &[u8]| f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            let vertices: Vec<[f32; 3]> = vb.chunks_exact(12).map(|c| place(&m_bind, [f(&c[0..4]), f(&c[4..8]), f(&c[8..12])])).collect();
            let u = |c: &[u8]| u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            let indices: Vec<[u32; 3]> = tb.chunks_exact(12).map(|c| [u(&c[0..4]), u(&c[4..8]), u(&c[8..12])]).collect();
            if !indices.is_empty() && indices.iter().flatten().all(|&i| (i as usize) < vertices.len()) {
                out.push(PhysShape::Mesh { vertices, indices });
            }
        }
    }
    Some(out)
}

/// One rigid part of a model's physics: shapes in the part's own space, and where the part sits in the rest pose.
#[derive(Clone, Debug, PartialEq)]
pub struct PhysPart {
    pub shapes: Vec<PhysShape>,
    pub position: [f32; 3],
    /// Rest rotation as a quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub mass: f32,
}

/// A twist / swing joint between two parts (`m_joints[]`, `m_nType` 4). Frames are `(position, rotation [x, y, z, w])`
/// in the space of each part.
#[derive(Clone, Debug, PartialEq)]
pub struct PhysJoint {
    pub body1: usize,
    pub body2: usize,
    pub frame1: ([f32; 3], [f32; 4]),
    pub frame2: ([f32; 3], [f32; 4]),
    /// Largest swing in radians, if limited.
    pub swing: Option<f32>,
    /// Twist range `[min, max]` in radians, if limited.
    pub twist: Option<[f32; 2]>,
    pub collide: bool,
}

/// Rotation (xyzw) of a row-major 3x4 rest-pose matrix (translation in the last column).
fn matrix_rotation(m: &[f32; 12]) -> [f32; 4] {
    let (r00, r01, r02, r10, r11, r12, r20, r21, r22) = (m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]);
    let tr = r00 + r11 + r22;
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [(r21 - r12) / s, (r02 - r20) / s, (r10 - r01) / s, 0.25 * s]
    } else if r00 > r11 && r00 > r22 {
        let s = (1.0 + r00 - r11 - r22).sqrt() * 2.0;
        [0.25 * s, (r01 + r10) / s, (r02 + r20) / s, (r21 - r12) / s]
    } else if r11 > r22 {
        let s = (1.0 + r11 - r00 - r22).sqrt() * 2.0;
        [(r01 + r10) / s, 0.25 * s, (r12 + r21) / s, (r02 - r20) / s]
    } else {
        let s = (1.0 + r22 - r00 - r11).sqrt() * 2.0;
        [(r02 + r20) / s, (r12 + r21) / s, 0.25 * s, (r10 - r01) / s]
    };
    let n = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

/// The physics parts as separate bodies, each with its own-space shapes and rest pose (for a ragdoll).
pub fn parts(kv: &Value) -> Option<Vec<PhysPart>> {
    let list = kv.get("m_parts").and_then(Value::as_array)?;
    let mut out = Vec::new();
    for (i, part) in list.iter().enumerate() {
        let m = bind(kv, i);
        let (position, rotation) = match &m {
            Some(m) => ([m[3], m[7], m[11]], matrix_rotation(m)),
            None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
        };
        out.push(PhysPart { shapes: part_shapes(part, &None)?, position, rotation, mass: part.get("m_flMass").and_then(Value::as_f64).unwrap_or(0.0) as f32 });
    }
    (!out.is_empty()).then_some(out)
}

fn frame(v: &Value) -> Option<([f32; 3], [f32; 4])> {
    let a = v.as_array()?;
    let f = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x as f32);
    Some(([f(0)?, f(1)?, f(2)?], [f(4)?, f(5)?, f(6)?, f(7)?]))
}

/// Joints of a physics tree; types other than twist / swing are skipped.
pub fn joints(kv: &Value) -> Vec<PhysJoint> {
    let range = |j: &Value, key: &str, on: &str| -> Option<[f32; 2]> {
        if !j.get(on).and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        let r = j.get(key)?;
        Some([r.get("m_flMin")?.as_f64()? as f32, r.get("m_flMax")?.as_f64()? as f32])
    };
    kv.get("m_joints")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter(|j| j.get("m_nType").and_then(Value::as_i64) == Some(4))
        .filter_map(|j| {
            Some(PhysJoint {
                body1: j.get("m_nBody1")?.as_i64()? as usize,
                body2: j.get("m_nBody2")?.as_i64()? as usize,
                frame1: frame(j.get("m_Frame1")?)?,
                frame2: frame(j.get("m_Frame2")?)?,
                swing: range(j, "m_SwingLimit", "m_bEnableSwingLimit").map(|r| r[1]),
                twist: range(j, "m_TwistLimit", "m_bEnableTwistLimit"),
                collide: j.get("m_bEnableCollision").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// The physics tree of a model: its embedded `PHYS` block, else the first referenced `vphys_c` that has parts.
pub fn tree(model: &Resource, model_kv: &Value, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<Value> {
    let has_parts = |kv: &Value| kv.get("m_parts").and_then(Value::as_array).is_some_and(|p| !p.is_empty());
    if let Some(kv) = model.block_data("PHYS").and_then(|b| kv3::parse(b).ok()).filter(has_parts) {
        return Some(kv);
    }
    for r in model_kv.get("m_refPhysicsData").and_then(Value::as_array).unwrap_or(&[]) {
        let Some(path) = r.as_str() else { continue };
        let path = if path.ends_with("_c") { path.to_string() } else { format!("{path}_c") };
        let Some(data) = read(&path) else { continue };
        let Ok(res) = Resource::parse(&data) else { continue };
        if let Some(kv) = res.block_data("DATA").and_then(|b| kv3::parse(b).ok()).filter(has_parts) {
            return Some(kv);
        }
    }
    None
}

/// Parts and joints of a model that has jointed physics (a ragdoll), `None` otherwise.
pub fn ragdoll(model: &Resource, model_kv: &Value, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<(Vec<PhysPart>, Vec<PhysJoint>)> {
    let kv = tree(model, model_kv, read)?;
    let joints = joints(&kv);
    let parts = parts(&kv)?;
    let valid = joints.iter().all(|j| j.body1 < parts.len() && j.body2 < parts.len());
    (!joints.is_empty() && valid).then_some((parts, joints))
}

/// Collision shapes of a model: its embedded `PHYS` block first, then the referenced `vphys_c` files.
pub fn load(model: &Resource, model_kv: &Value, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<Vec<PhysShape>> {
    if let Some(s) = model.block_data("PHYS").and_then(|b| kv3::parse(b).ok()).and_then(|kv| shapes(&kv)) {
        return Some(s);
    }
    for r in model_kv.get("m_refPhysicsData").and_then(Value::as_array).unwrap_or(&[]) {
        let Some(path) = r.as_str() else { continue };
        let path = if path.ends_with("_c") { path.to_string() } else { format!("{path}_c") };
        let Some(data) = read(&path) else { continue };
        let Ok(res) = Resource::parse(&data) else { continue };
        if let Some(s) = res.block_data("DATA").and_then(|b| kv3::parse(b).ok()).and_then(|kv| shapes(&kv)) {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Vec<(&str, Value)>) -> Value {
        Value::Object(v.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    fn d(x: f64) -> Value { Value::Double(x) }
    fn v3(x: f64, y: f64, z: f64) -> Value { Value::Array(vec![d(x), d(y), d(z)]) }

    #[test]
    fn reads_hulls_spheres_and_capsules() {
        let hull = obj(vec![("m_Hull", obj(vec![("m_Vertices", Value::Array(vec![v3(0., 0., 0.), v3(1., 0., 0.), v3(0., 1., 0.), v3(0., 0., 1.)]))]))]);
        let sphere = obj(vec![("m_Sphere", obj(vec![("m_vCenter", v3(1., 2., 3.)), ("m_flRadius", d(4.))]))]);
        let capsule = obj(vec![("m_Capsule", obj(vec![("m_vCenter", Value::Array(vec![v3(0., 0., 0.), v3(0., 0., 10.)])), ("m_flRadius", d(2.))]))]);
        let shape = obj(vec![("m_hulls", Value::Array(vec![hull])), ("m_spheres", Value::Array(vec![sphere])), ("m_capsules", Value::Array(vec![capsule]))]);
        let kv = obj(vec![("m_parts", Value::Array(vec![obj(vec![("m_rnShape", shape)])]))]);
        let s = shapes(&kv).unwrap();
        assert_eq!(s.len(), 3);
        assert!(matches!(&s[0], PhysShape::Hull(v) if v.len() == 4));
        assert_eq!(s[1], PhysShape::Sphere { center: [1.0, 2.0, 3.0], radius: 4.0 });
        assert_eq!(s[2], PhysShape::Capsule { a: [0.0; 3], b: [0.0, 0.0, 10.0], radius: 2.0 });
    }

    #[test]
    fn reads_triangle_meshes_from_blobs() {
        let f = |v: &[f32]| Value::Blob(v.iter().flat_map(|x| x.to_le_bytes()).collect());
        let u = |v: &[u32]| Value::Blob(v.iter().flat_map(|x| x.to_le_bytes()).collect());
        let mesh = |tris: &[u32]| obj(vec![("m_Mesh", obj(vec![("m_Vertices", f(&[0., 0., 0., 1., 0., 0., 0., 1., 0.])), ("m_Triangles", u(tris))]))]);
        // The second mesh indexes a vertex that is not there and is dropped.
        let shape = obj(vec![("m_meshes", Value::Array(vec![mesh(&[0, 1, 2]), mesh(&[0, 1, 3])]))]);
        let kv = obj(vec![("m_parts", Value::Array(vec![obj(vec![("m_rnShape", shape)])]))]);
        let s = shapes(&kv).unwrap();
        assert_eq!(s, vec![PhysShape::Mesh { vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], indices: vec![[0, 1, 2]] }]);
    }

    #[test]
    fn parts_keep_their_own_space_and_joints_are_read() {
        let sphere = obj(vec![("m_rnShape", obj(vec![("m_spheres", Value::Array(vec![obj(vec![("m_Sphere", obj(vec![("m_vCenter", v3(5., 0., 0.)), ("m_flRadius", d(1.))]))])]))])), ("m_flMass", d(3.))]);
        // Rotated 90 degrees about Z, moved to (1, 2, 3).
        let rot = Value::Array([0.0, -1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0].iter().map(|&x| d(x)).collect());
        let frame = Value::Array([1., 2., 3., 1., 0., 0., 0., 1.].iter().map(|&x| d(x)).collect());
        let joint = |ty: i64| obj(vec![
            ("m_nType", Value::Int(ty)), ("m_nBody1", Value::Int(0)), ("m_nBody2", Value::Int(1)),
            ("m_Frame1", frame.clone()), ("m_Frame2", frame.clone()), ("m_bEnableCollision", Value::Bool(false)),
            ("m_bEnableSwingLimit", Value::Bool(true)), ("m_SwingLimit", obj(vec![("m_flMin", d(0.)), ("m_flMax", d(0.5))])),
            ("m_bEnableTwistLimit", Value::Bool(false)), ("m_TwistLimit", obj(vec![("m_flMin", d(-1.)), ("m_flMax", d(1.))])),
        ]);
        let kv = obj(vec![("m_parts", Value::Array(vec![sphere.clone(), sphere])), ("m_bindPose", Value::Array(vec![rot.clone(), rot])), ("m_joints", Value::Array(vec![joint(4), joint(2)]))]);
        let p = parts(&kv).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].shapes, vec![PhysShape::Sphere { center: [5.0, 0.0, 0.0], radius: 1.0 }], "shapes stay in the part's space");
        assert_eq!(p[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(p[0].mass, 3.0);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((p[0].rotation[2] - h).abs() < 1e-5 && (p[0].rotation[3] - h).abs() < 1e-5, "90 degrees about Z, got {:?}", p[0].rotation);
        let j = joints(&kv);
        assert_eq!(j.len(), 1, "only the twist / swing joint is kept");
        assert_eq!((j[0].body1, j[0].body2, j[0].swing, j[0].twist, j[0].collide), (0, 1, Some(0.5), None, false));
        assert_eq!(j[0].frame1, ([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn parts_are_placed_with_their_bind_pose() {
        let sphere = |x: f64| obj(vec![("m_rnShape", obj(vec![("m_spheres", Value::Array(vec![obj(vec![("m_Sphere", obj(vec![("m_vCenter", v3(x, 0., 0.)), ("m_flRadius", d(1.))]))])]))]))]);
        // Part 1 is rotated 90 degrees about Z (x -> y) and moved up by 10.
        let rot = Value::Array([0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 10.0].iter().map(|&x| d(x)).collect());
        let ident = Value::Array([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0].iter().map(|&x| d(x)).collect());
        let kv = obj(vec![("m_parts", Value::Array(vec![sphere(5.), sphere(5.)])), ("m_bindPose", Value::Array(vec![ident, rot]))]);
        let s = shapes(&kv).unwrap();
        assert_eq!(s[0], PhysShape::Sphere { center: [5.0, 0.0, 0.0], radius: 1.0 });
        assert_eq!(s[1], PhysShape::Sphere { center: [0.0, 5.0, 10.0], radius: 1.0 });
    }
}
