fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let load = |p: &str| sbox_resource::model::load_skel(&std::fs::read(p).unwrap()).unwrap();
    let (x, y) = (load(&a[0]), load(&a[1]));
    for (i, n) in y.names.iter().enumerate() {
        if !x.names.contains(n) {
            println!("only in 2nd: {i} {n} parent {:?}", y.names.get(y.parents[i].max(0) as usize));
        }
    }
    for (i, n) in x.names.iter().enumerate() {
        if !y.names.contains(n) {
            println!("only in 1st: {i} {n}");
        }
    }
    println!("same order for common: {}", x.names.iter().zip(&y.names).filter(|(p, q)| p == q).count());
}
