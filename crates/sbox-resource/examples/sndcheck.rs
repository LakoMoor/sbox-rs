use sbox_resource::vsnd;
use std::collections::BTreeMap;
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() { walk(&path, out) } else if path.to_string_lossy().ends_with(".vsnd_c") { out.push(path) }
        }
    }
}

fn main() {
    let mut files = vec![];
    walk(Path::new(&std::env::args().nth(1).unwrap()), &mut files);
    let mut stats: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        match vsnd::parse(&data) {
            Ok(s) => {
                let ok = match s.format {
                    vsnd::SoundFormat::Opus => s.decode_opus().map(|p| ((p.len() as f32 / s.channels as f32 / 48000.0) - s.duration).abs() < 0.2).unwrap_or(false),
                    vsnd::SoundFormat::Unknown(_) => false,
                    _ => true,
                };
                let e = stats.entry(format!("{:?}", s.format)).or_default();
                e.0 += 1;
                e.1 += ok as usize;
                if !ok { println!("FAIL {}", f.display()) }
            }
            Err(e) => { println!("PARSE ERR {}: {e:#}", f.display()); stats.entry("parse error".into()).or_default().0 += 1 }
        }
    }
    for (k, (n, ok)) in stats { println!("{k}: {ok}/{n}") }
}
