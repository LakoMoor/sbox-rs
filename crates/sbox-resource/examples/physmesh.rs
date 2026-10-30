//! Describe the triangle-mesh arrays of a model's physics: physmesh <game dir> <model.vmdl_c>
use sbox_resource::{kv3, vmat::GameFs, Resource, Value};

fn describe(v: &Value) -> String {
    match v {
        Value::Blob(b) => format!("Blob({} bytes) {:02x?}", b.len(), &b[..b.len().min(48)]),
        Value::Array(a) => format!("Array({}) first={:?}", a.len(), a.first()),
        v => format!("{v:?}"),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::PathBuf::from(&a[1]);
    let mut fs = GameFs::from_game_dir(game.clone());
    fs.mount_map_packages(&game.join("core/maps"));
    let d = fs.read(&a[2]).expect("model");
    let res = Resource::parse(&d).unwrap();
    let kv = kv3::parse(res.block_data("DATA").unwrap()).unwrap();
    let mut phys = res.block_data("PHYS").and_then(|b| kv3::parse(b).ok()).filter(|v| v.get("m_parts").and_then(Value::as_array).is_some_and(|p| !p.is_empty()));
    if phys.is_none() {
        for r in kv.get("m_refPhysicsData").and_then(Value::as_array).unwrap_or(&[]) {
            let p = format!("{}_c", r.as_str().unwrap());
            if let Some(pd) = fs.read(&p) { phys = Resource::parse(&pd).ok().and_then(|r| r.block_data("DATA").and_then(|b| kv3::parse(b).ok())); }
        }
    }
    let phys = phys.unwrap();
    for part in phys.get("m_parts").and_then(Value::as_array).unwrap_or(&[]) {
        for m in part.get("m_rnShape").and_then(|s| s.get("m_meshes")).and_then(Value::as_array).unwrap_or(&[]) {
            let mesh = m.get("m_Mesh").unwrap();
            if let Value::Object(o) = mesh {
                for (k, v) in o { println!("{k}: {}", describe(v)); }
            }
        }
    }
}
