use sbox_resource::vtex;
use std::collections::BTreeMap;
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() { walk(&path, out) } else if path.to_string_lossy().ends_with(".vtex_c") { out.push(path) }
        }
    }
}

fn main() {
    let mut files = vec![];
    walk(Path::new(&std::env::args().nth(1).unwrap()), &mut files);
    let mut ok = BTreeMap::new();
    let mut errs: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        match vtex::parse(&data) {
            Ok(t) => *ok.entry(format!("{:?}{}", t.format, if t.is_cube() { " cube" } else { "" })).or_insert(0) += 1,
            Err(e) => {
                let k: String = e.to_string().chars().take(45).collect();
                errs.entry(k).or_insert((0, f.display().to_string())).0 += 1;
            }
        }
    }
    println!("total {} ok {:?}", files.len(), ok);
    for (m, (n, ex)) in errs { println!("{n:5} {m}  e.g. {ex}") }
}
