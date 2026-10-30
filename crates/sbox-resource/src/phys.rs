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
        let m = bind(kv, i);
        let m_bind = m;
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
    (!out.is_empty()).then_some(out)
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
