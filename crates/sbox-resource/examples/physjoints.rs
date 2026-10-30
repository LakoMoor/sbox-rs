//! Print the joints of a model's physics compactly: physjoints <game dir> <model.vmdl_c>
use sbox_resource::{kv3, vmat::GameFs, Resource, Value};

fn short(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(short).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!("{{{}}}", o.iter().map(|(k, v)| format!("{k}: {}", short(v))).collect::<Vec<_>>().join(", ")),
        Value::Double(d) => format!("{d:.3}"),
        v => format!("{v:?}"),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::PathBuf::from(&a[1]);
    let mut fs = GameFs::from_game_dir(game.clone());
    fs.mount_map_packages(&game.join("core/maps"));
    let data = std::fs::read(&a[2]).ok().or_else(|| fs.read(&a[2])).expect("model");
    let res = Resource::parse(&data).unwrap();
    let kv = kv3::parse(res.block_data("DATA").unwrap()).unwrap();
    let mut phys = res.block_data("PHYS").and_then(|b| kv3::parse(b).ok()).filter(|v| v.get("m_parts").and_then(Value::as_array).is_some_and(|p| !p.is_empty()));
    if phys.is_none() {
        for r in kv.get("m_refPhysicsData").and_then(Value::as_array).unwrap_or(&[]) {
            let p = format!("{}_c", r.as_str().unwrap());
            if let Some(pd) = fs.read(&p) { phys = Resource::parse(&pd).ok().and_then(|r| r.block_data("DATA").and_then(|b| kv3::parse(b).ok())); }
        }
    }
    let phys = phys.unwrap();
    for (i, j) in phys.get("m_joints").and_then(Value::as_array).unwrap_or(&[]).iter().enumerate().take(4) {
        println!("#{i} {}", short(j));
    }
    println!("boneNames {}", phys.get("m_boneNames").map(short).unwrap_or_default());
}
