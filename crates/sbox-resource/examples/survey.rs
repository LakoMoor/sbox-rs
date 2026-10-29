use sbox_resource::{kv3, Resource};
use std::collections::BTreeMap;
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().map_or(false, |x| x.to_string_lossy().ends_with("_c")) {
                out.push(path);
            }
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).unwrap();
    let mut files = vec![];
    walk(Path::new(&root), &mut files);
    let mut ok = 0;
    let mut errs: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let mut container_fail = 0;
    for f in &files {
        let Ok(data) = std::fs::read(f) else { continue };
        let res = match Resource::parse(&data) {
            Ok(r) => r,
            Err(e) => {
                container_fail += 1;
                let key: String = format!("container: {e}").chars().take(50).collect();
                errs.entry(key).or_insert((0, f.display().to_string())).0 += 1;
                continue;
            }
        };
        for b in &res.blocks {
            let d = &data[b.start..b.start + b.len];
            if d.len() < 4 || d[1] != b'3' || d[2] != b'V' || d[3] != b'K' {
                continue;
            }
            if let Ok(h) = kv3::read_header(d) {
                *versions.entry(format!("v{} method{}", h.version, h.method)).or_default() += 1;
            }
            match kv3::parse(d) {
                Ok(_) => ok += 1,
                Err(e) => {
                    let msg = format!("{} [{}]", e, b.name());
                    let ent = errs.entry(msg).or_insert((0, f.display().to_string()));
                    ent.0 += 1;
                }
            }
        }
    }
    println!("files={} container_fail={} kv3_ok={}", files.len(), container_fail, ok);
    println!("versions: {versions:?}");
    for (m, (n, ex)) in errs {
        println!("{n:6} {m}   e.g. {ex}");
    }
}
