//! Survey the physics data of every model: physsurvey <game dir>
use sbox_resource::{kv3, vmat::GameFs, vpk::Vpk, Resource, Value};

fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() { walk(&path, out) } else if path.extension().is_some_and(|x| x == "vpk" || x == "vmdl_c") { out.push(path) }
    }
}

fn len(v: &Value, k: &str) -> usize { v.get(k).and_then(Value::as_array).map_or(0, <[Value]>::len) }

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut files = vec![];
    walk(std::path::Path::new(&a[1]), &mut files);
    let mut fs = GameFs::from_game_dir(std::path::PathBuf::from(&a[1]));
    fs.mount_map_packages(&std::path::PathBuf::from(&a[1]).join("core/maps"));
    let mut models: Vec<(String, Vec<u8>)> = vec![];
    for p in files {
        if p.extension().is_some_and(|x| x == "vmdl_c") {
            models.push((p.display().to_string(), std::fs::read(&p).unwrap()));
        } else if let Ok(v) = Vpk::open(&p) {
            for n in v.entries.keys().filter(|k| k.ends_with(".vmdl_c")) {
                if let Some(d) = v.read(n) { models.push((n.clone(), d)); }
            }
        }
    }
    let (mut with, mut total) = (0, 0);
    for (name, d) in &models {
        total += 1;
        let Ok(res) = Resource::parse(d) else { continue };
        let mut phys: Vec<Value> = vec![];
        if let Some(b) = res.block_data("PHYS") { if let Ok(v) = kv3::parse(b) { phys.push(v) } }
        if let Some(kv) = res.block_data("DATA").and_then(|b| kv3::parse(b).ok()) {
            for r in kv.get("m_refPhysicsData").and_then(Value::as_array).unwrap_or(&[]) {
                if let Some(s) = r.as_str() {
                    let c = if s.ends_with("_c") { s.to_string() } else { format!("{s}_c") };
                    if let Some(pd) = fs.read(&c) { if let Some(b) = Resource::parse(&pd).ok().and_then(|r| r.block_data("DATA").map(<[u8]>::to_vec)) { if let Ok(v) = kv3::parse(&b) { phys.push(v) } } }
                }
            }
        }
        for v in &phys {
            let parts = v.get("m_parts").and_then(Value::as_array).unwrap_or(&[]);
            if parts.is_empty() { continue }
            with += 1;
            let (mut h, mut s, mut c, mut m) = (0, 0, 0, 0);
            for p in parts {
                if let Some(sh) = p.get("m_rnShape") { h += len(sh, "m_hulls"); s += len(sh, "m_spheres"); c += len(sh, "m_capsules"); m += len(sh, "m_meshes"); }
            }
            println!("{name}: parts={} hulls={h} spheres={s} capsules={c} meshes={m} joints={} constraints={}", parts.len(), len(v, "m_joints"), len(v, "m_constraints2"));
        }
    }
    println!("{with}/{total} models with physics parts");
}
