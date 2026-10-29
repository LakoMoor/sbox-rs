//! Dump every entity key-values blob of every `*.vents_c` in the VPKs under a game directory.
use sbox_resource::{kv3, vpk::Vpk, Resource, Value};

fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() { walk(&path, out) } else if path.extension().is_some_and(|x| x == "vpk") { out.push(path) }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut vpks = vec![];
    walk(std::path::Path::new(&a[1]), &mut vpks);
    let out = std::path::Path::new(&a[2]);
    std::fs::create_dir_all(out).unwrap();
    let mut n = 0;
    for p in vpks {
        let Ok(v) = Vpk::open(&p) else { continue };
        for name in v.entries.keys().filter(|k| k.ends_with(".vents_c")) {
            let Some(d) = v.read(name) else { continue };
            let Ok(res) = Resource::parse(&d) else { continue };
            let Some(data) = res.block_data("DATA") else { continue };
            let Ok(kv) = kv3::parse(data) else { continue };
            for e in kv.get("m_entityKeyValues").and_then(Value::as_array).unwrap_or(&[]) {
                if let Some(Value::Blob(b)) = e.get("m_keyValuesData") {
                    std::fs::write(out.join(format!("{n:04}.bin")), b).unwrap();
                    n += 1;
                }
            }
        }
    }
    println!("{n} blobs");
}
