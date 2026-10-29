//! Print the non-geometry keys of a model's physics tree: phystree <game dir> <model.vmdl_c>
use sbox_resource::{kv3, vmat::GameFs, Resource, Value};

fn show(v: &Value, ind: usize, key: &str) {
    let pad = "  ".repeat(ind);
    match v {
        Value::Object(o) => {
            println!("{pad}{key} {{");
            for (k, v) in o { if !matches!(k.as_str(), "m_Vertices" | "m_Planes" | "m_Edges" | "m_Faces" | "m_Nodes" | "m_Triangles") { show(v, ind + 1, k) } }
            println!("{pad}}}");
        }
        Value::Array(a) if a.iter().any(|x| matches!(x, Value::Object(_))) => {
            println!("{pad}{key} [{}]", a.len());
            for (i, x) in a.iter().enumerate() { show(x, ind + 1, &format!("#{i}")) }
        }
        v => println!("{pad}{key} = {}", format!("{v:?}").chars().take(110).collect::<String>()),
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
    show(&phys.unwrap(), 0, "phys");
}
