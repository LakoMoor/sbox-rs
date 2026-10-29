use sbox_resource::{vbib, Resource};
use std::collections::BTreeMap;
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(&path, out)
            } else if path.to_string_lossy().ends_with(".vmdl_c") {
                out.push(path)
            }
        }
    }
}

fn main() {
    let mut files = vec![];
    walk(Path::new(&std::env::args().nth(1).unwrap()), &mut files);
    let (mut ok, mut none) = (0, 0);
    let mut fmts: BTreeMap<String, usize> = BTreeMap::new();
    let mut errs: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        let res = Resource::parse(&data).unwrap();
        let Some(b) = res.block_data("MBUF") else {
            none += 1;
            continue;
        };
        match vbib::parse(b) {
            Ok(v) => {
                ok += 1;
                for vb in &v.vertex_buffers {
                    for a in &vb.attributes {
                        *fmts.entry(format!("{} fmt{}", a.name, a.format)).or_default() += 1;
                    }
                }
            }
            Err(e) => {
                let k: String = e.to_string().chars().take(50).collect();
                errs.entry(k).or_insert((0, f.display().to_string())).0 += 1;
            }
        }
    }
    println!("vmdl {} ok {ok} no-MBUF {none}", files.len());
    println!("{fmts:#?}");
    for (m, (n, ex)) in errs {
        println!("{n:5} {m}  e.g. {ex}");
    }
}
