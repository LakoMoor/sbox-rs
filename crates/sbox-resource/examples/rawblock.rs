use sbox_resource::Resource;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1]).unwrap();
    let res = Resource::parse(&data).unwrap();
    print!("{}", String::from_utf8_lossy(res.block_data(a.get(2).map_or("DATA", |s| s)).unwrap()));
}
