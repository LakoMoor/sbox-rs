use sbox_resource::{vmat, vtex};
use std::collections::BTreeMap;
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() { walk(&path, out) } else if path.to_string_lossy().ends_with(".vmat_c") { out.push(path) }
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).unwrap();
    let fs = vmat::GameFs::from_game_dir(&root);
    let mut files = vec![];
    walk(Path::new(&root), &mut files);
    let mut fmts: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        let Ok(m) = vmat::parse(&std::fs::read(f).unwrap()) else { continue };
        for key in ["g_tNormal", "g_tRma", "g_tMetalness"] {
            if let Some(t) = m.textures.get(key) {
                if let Some(Ok(tx)) = fs.read(t).map(|d| vtex::parse(&d)) {
                    *fmts.entry(format!("{key} {:?}", tx.format)).or_default() += 1;
                }
            }
        }
    }
    for (k, v) in fmts { println!("{v:5} {k}") }
}
