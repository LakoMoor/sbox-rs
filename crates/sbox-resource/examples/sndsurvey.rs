use sbox_resource::Resource;
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
    let mut groups: BTreeMap<String, (usize, String, usize)> = BTreeMap::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        let res = Resource::parse(&data).unwrap();
        let d = res.block_data("DATA").unwrap();
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let packed = u(0);
        let start = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let sr = packed & 0xFFFF;
        let hi = packed >> 16;
        let samples = u(8);
        let dur = f32::from_bits(u(12));
        let payload = data.len() - start;
        let bps = (payload as f32 * 8.0 / dur / 1000.0) as i32;
        let key = format!("sr={sr} hi={hi:#x}");
        let e = groups.entry(key).or_insert((0, f.display().to_string(), bps as usize));
        e.0 += 1;
        let _ = samples;
    }
    for (k, (n, ex, bps)) in groups { println!("{n:5} {k} ~{bps}kbps e.g. {ex}") }
}
