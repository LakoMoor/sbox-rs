//! Print bind poses of a model's physics + the model mesh bounds: physbind <game dir> <model.vmdl_c>
use sbox_resource::{kv3, vmat::GameFs, Resource, Value, model};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::PathBuf::from(&a[1]);
    let mut fs = GameFs::from_game_dir(game.clone());
    fs.mount_map_packages(&game.join("core/maps"));
    let d = fs.read(&a[2]).expect("model");
    let res = Resource::parse(&d).unwrap();
    let m = model::load_with(&d, &|p| fs.read(p)).unwrap();
    for mm in m.default_meshes() { println!("mesh bounds {:?} {:?}", mm.mesh.mins, mm.mesh.maxs); }
    println!("skeleton {:?} parents {:?}", m.skeleton.names, m.skeleton.parents);
    let kv = kv3::parse(res.block_data("DATA").unwrap()).unwrap();
    let mut phys = res.block_data("PHYS").and_then(|b| kv3::parse(b).ok()).filter(|v| v.get("m_parts").and_then(Value::as_array).is_some_and(|p| !p.is_empty()));
    if phys.is_none() {
        for r in kv.get("m_refPhysicsData").and_then(Value::as_array).unwrap_or(&[]) {
            let p = format!("{}_c", r.as_str().unwrap());
            if let Some(pd) = fs.read(&p) { phys = Resource::parse(&pd).ok().and_then(|r| r.block_data("DATA").and_then(|b| kv3::parse(b).ok())); }
        }
    }
    let p = phys.unwrap();
    println!("boneNames {:?}", p.get("m_boneNames").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>()));
    println!("indexNames {:?} boneParents {:?}", p.get("m_indexNames"), p.get("m_boneParents"));
    for (i, b) in p.get("m_bindPose").and_then(Value::as_array).unwrap_or(&[]).iter().enumerate() {
        let f: Vec<f32> = b.as_array().unwrap().iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
        println!("bind[{i}] len={} {f:?}", f.len());
    }
    for (i, part) in p.get("m_parts").and_then(Value::as_array).unwrap_or(&[]).iter().enumerate() {
        let mut mn = [f32::MAX; 3]; let mut mx = [f32::MIN; 3];
        for h in part.get("m_rnShape").and_then(|s| s.get("m_hulls")).and_then(Value::as_array).unwrap_or(&[]) {
            for v in h.get("m_Hull").and_then(|h| h.get("m_Vertices")).and_then(Value::as_array).unwrap_or(&[]) {
                let v: Vec<f32> = v.as_array().unwrap().iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
                for k in 0..3 { mn[k] = mn[k].min(v[k]); mx[k] = mx[k].max(v[k]); }
            }
        }
        println!("part {i} local hull bounds {mn:?} {mx:?}");
    }
    for j in p.get("m_joints").and_then(Value::as_array).unwrap_or(&[]) {
        println!("joint type {:?} b1 {:?} b2 {:?} f1 {:?} f2 {:?}", j.get("m_nType"), j.get("m_nBody1"), j.get("m_nBody2"), j.get("m_Frame1"), j.get("m_Frame2"));
    }
}
