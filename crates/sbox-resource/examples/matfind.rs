use std::path::Path;
fn walk(p: &Path, flag: &str, n: &mut usize) {
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() { walk(&path, flag, n) } else if path.to_string_lossy().ends_with(".vmat_c") && *n < 8 {
            if let Ok(m) = sbox_resource::vmat::parse(&std::fs::read(&path).unwrap()) {
                if m.flag(flag) { *n += 1; println!("{}", path.display()) }
            }
        }
    }
}
fn main() { let a: Vec<_> = std::env::args().collect(); walk(Path::new(&a[1]), &a[2], &mut 0) }
