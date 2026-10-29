use sbox_resource::mesh;
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() { walk(&path, out) } else if path.to_string_lossy().ends_with(".vmdl_c") { out.push(path) }
        }
    }
}

fn main() {
    let mut files = vec![];
    walk(Path::new(&std::env::args().nth(1).unwrap()), &mut files);
    let (mut ok, mut bad_norm, mut tris) = (0, 0, 0usize);
    let mut errs = std::collections::BTreeMap::<String, (usize, String)>::new();
    for f in &files {
        match mesh::load_embedded(&std::fs::read(f).unwrap()) {
            Ok(m) => {
                ok += 1;
                for dc in &m.draw_calls {
                    tris += dc.indices.len() / 3;
                    if dc.vertices.iter().any(|v| { let l = (v.normal[0].powi(2) + v.normal[1].powi(2) + v.normal[2].powi(2)).sqrt(); (l - 1.0).abs() > 0.05 }) {
                        bad_norm += 1;
                    }
                }
            }
            Err(e) => { let k: String = e.to_string().chars().take(55).collect(); errs.entry(k).or_insert((0, f.display().to_string())).0 += 1; }
        }
    }
    println!("vmdl {} loaded {ok}, drawcalls with non-unit normals {bad_norm}, triangles {tris}", files.len());
    for (m, (n, ex)) in errs { println!("{n:5} {m}  e.g. {ex}") }
}
