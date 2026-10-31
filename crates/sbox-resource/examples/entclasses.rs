//! Count entity classnames over every `*.vents_c` in the VPKs under a game directory.
use sbox_resource::{vents, vpk::Vpk};
use std::collections::BTreeMap;

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
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for p in vpks {
        let Ok(v) = Vpk::open(&p) else { continue };
        for name in v.entries.keys().filter(|k| k.ends_with(".vents_c")) {
            let Some(d) = v.read(name) else { continue };
            for e in vents::load(&d).unwrap_or_default() {
                if let Some(w) = a.get(2) { if e.classname() == w { println!("{name}: {}", e.classname()); } }
                *counts.entry(e.classname().to_string()).or_default() += 1;
            }
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in v { println!("{n:5} {k}"); }
}
