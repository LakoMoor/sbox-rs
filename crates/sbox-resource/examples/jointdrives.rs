//! Count shipped joints by type and whether they carry linear limits / motors / springs: jointdrives <game dir>
use sbox_resource::{kv3, vmat::GameFs, Resource, Value};
use std::collections::BTreeMap;

fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() { walk(&path, out) } else if path.to_string_lossy().ends_with(".vmdl_c") { out.push(path) }
        }
    }
}

fn main() {
    let game = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let mut files = vec![];
    walk(&game, &mut files);
    let mut fs = GameFs::from_game_dir(game.clone());
    fs.mount_map_packages(&game.join("core/maps"));
    let mut c: BTreeMap<String, usize> = Default::default();
    for f in files {
        let Ok(data) = std::fs::read(&f) else { continue };
        let Ok(res) = Resource::parse(&data) else { continue };
        let Some(kv) = res.block_data("DATA").and_then(|b| kv3::parse(b).ok()) else { continue };
        let Some(phys) = sbox_resource::phys::tree(&res, &kv, &|p| fs.read(p)) else { continue };
        for j in phys.get("m_joints").and_then(Value::as_array).unwrap_or(&[]) {
            let ty = j.get("m_nType").and_then(Value::as_i64).unwrap_or(-1);
            *c.entry(format!("type {ty}")).or_default() += 1;
            let f = |k: &str| j.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let b = |k: &str| j.get(k).and_then(Value::as_bool).unwrap_or(false);
            if b("m_bEnableLinearLimit") { *c.entry(format!("type {ty} linear limit")).or_default() += 1 }
            if f("m_flLinearFrequency") > 0.0 { *c.entry(format!("type {ty} linear spring (frequency)")).or_default() += 1 }
            if f("m_flAngularFrequency") > 0.0 { *c.entry(format!("type {ty} angular spring (frequency)")).or_default() += 1 }
            if f("m_flLinearStrength") > 0.0 { *c.entry(format!("type {ty} linear strength")).or_default() += 1 }
            if f("m_flAngularStrength") > 0.0 { *c.entry(format!("type {ty} angular strength")).or_default() += 1 }
            if b("m_bEnableLinearMotor") && j.get("m_vLinearTargetVelocity").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x.as_f64().unwrap_or(0.0) != 0.0)) { *c.entry(format!("type {ty} moving linear motor")).or_default() += 1 }
            if b("m_bEnableAngularMotor") && j.get("m_vAngularTargetVelocity").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x.as_f64().unwrap_or(0.0) != 0.0)) { *c.entry(format!("type {ty} moving angular motor")).or_default() += 1 }
        }
    }
    for (k, v) in c { println!("{v:5} {k}") }
}
