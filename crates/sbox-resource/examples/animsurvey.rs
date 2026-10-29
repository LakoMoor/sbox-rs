use sbox_resource::anim;
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(&path, out)
            } else if path.to_string_lossy().ends_with(".anim_c") {
                out.push(path)
            }
        }
    }
}

fn main() {
    let mut files = vec![];
    walk(Path::new(&std::env::args().nth(1).unwrap()), &mut files);
    let (mut ok, mut discontinuous) = (0, 0);
    let mut errs = std::collections::BTreeMap::<String, (usize, String)>::new();
    for f in &files {
        match anim::parse(&std::fs::read(f).unwrap()) {
            Ok(a) => {
                ok += 1;
                let mut worst = 1.0f32;
                for fr in 0..a.frames.saturating_sub(1) {
                    let (p, q) = (a.sample_frame(fr), a.sample_frame(fr + 1));
                    for (i, t) in a.tracks.iter().enumerate() {
                        if !t.rotation_static {
                            worst = worst.min((0..4).map(|k| p[i].rotation[k] * q[i].rotation[k]).sum::<f32>().abs());
                        }
                    }
                }
                if worst < 0.9 {
                    discontinuous += 1;
                    println!("discontinuous {worst:.3} {}", f.display());
                }
            }
            Err(e) => {
                let k: String = e.to_string().chars().take(60).collect();
                errs.entry(k).or_insert((0, f.display().to_string())).0 += 1;
            }
        }
    }
    println!("anim {} parsed {ok}, discontinuous rotation {discontinuous}", files.len());
    for (m, (n, ex)) in errs {
        println!("{n:5} {m} e.g. {ex}");
    }
}
