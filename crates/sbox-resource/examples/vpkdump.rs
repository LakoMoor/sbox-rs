use sbox_resource::{kv3, vpk::Vpk, Resource, Value};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let v = Vpk::open(&a[1]).unwrap();
    let data = v.read(&a[2]).expect("entry");
    let res = Resource::parse(&data).unwrap();
    println!("blocks {:?}", res.blocks.iter().map(|b| (b.name(), b.len)).collect::<Vec<_>>());
    let kv = kv3::parse(res.block_data(a.get(3).map_or("DATA", |s| s.as_str())).unwrap()).unwrap();
    println!("{kv:#?}");
    let _: Option<&Value> = None;
}
