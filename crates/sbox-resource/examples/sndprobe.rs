use sbox_resource::Resource;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let data = std::fs::read(&path).unwrap();
    let res = Resource::parse(&data).unwrap();
    println!("file {} bytes, first u32 (data start) = {}", data.len(), u32::from_le_bytes(data[0..4].try_into().unwrap()));
    for b in &res.blocks {
        println!("block {} len {}", b.name(), b.len);
    }
    if let Some(d) = res.block_data("DATA") {
        println!("DATA {} bytes: {:02x?}", d.len(), &d[..d.len().min(96)]);
    }
}
