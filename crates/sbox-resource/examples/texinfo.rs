fn main() {
    let a: Vec<String> = std::env::args().collect();
    let fs = sbox_resource::vmat::GameFs::from_game_dir(&a[1]);
    let d = fs.read(&a[2]).expect("file");
    let t = sbox_resource::vtex::parse(&d).unwrap();
    println!("{}x{} {:?} mips={}", t.width, t.height, t.format, t.mips.len());
    let px = &t.mips[0];
    println!("first bytes {:?}", &px[..16.min(px.len())]);
}
