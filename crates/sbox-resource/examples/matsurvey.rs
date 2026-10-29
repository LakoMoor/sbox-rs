use sbox_resource::vmat;
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
    let mut files = vec![];
    walk(Path::new(&std::env::args().nth(1).unwrap()), &mut files);
    let (mut tex, mut flt, mut shaders): (BTreeMap<String, usize>, BTreeMap<String, usize>, BTreeMap<String, usize>) = Default::default();
    let mut n = 0;
    for f in &files {
        if let Ok(m) = vmat::parse(&std::fs::read(f).unwrap()) {
            n += 1;
            for k in m.textures.keys() { *tex.entry(k.clone()).or_default() += 1 }
            for k in m.floats.keys() { *flt.entry(k.clone()).or_default() += 1 }
            *shaders.entry(m.shader.clone()).or_default() += 1;
        }
    }
    println!("{n} materials");
    let top = |m: &BTreeMap<String, usize>, label: &str| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        println!("-- {label}");
        for (k, c) in v.iter().take(18) { println!("{c:6} {k}") }
    };
    top(&tex, "textures");
    top(&flt, "floats");
    top(&shaders, "shaders");
}
