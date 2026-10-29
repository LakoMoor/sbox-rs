//! Collision shapes of a model: the `PHYS` block of a `vmdl_c` or the `vphys_c` it references (`m_refPhysicsData`).
//! Hulls, spheres and capsules are read; triangle meshes are not.

use crate::{kv3, Resource, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum PhysShape {
    /// Convex hull vertices in model space.
    Hull(Vec<[f32; 3]>),
    Sphere { center: [f32; 3], radius: f32 },
    Capsule { a: [f32; 3], b: [f32; 3], radius: f32 },
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
