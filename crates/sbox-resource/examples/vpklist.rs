fn main() {
    let v = sbox_resource::vpk::Vpk::open(std::env::args().nth(1).unwrap()).unwrap();
    let mut k: Vec<_> = v.entries.iter().collect();
    k.sort_by_key(|e| e.0);
    for (n, e) in k { println!("{:9} {n}", e.length as usize + e.preload.len()) }
}
