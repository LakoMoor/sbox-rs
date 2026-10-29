use sbox_resource::{kv3, Resource};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("path");
    let block = std::env::args().nth(2).unwrap_or("MDAT".into());
    let data = std::fs::read(&path)?;
    let res = Resource::parse(&data)?;
    println!("hv={} v={} blocks={:?}", res.header_version, res.version, res.blocks.iter().map(|b| (b.name(), b.len)).collect::<Vec<_>>());
    let b = res.block_data(&block).expect("block");
    let h = kv3::read_header(b)?;
    println!(
        "ver={} method={} dict={} frame={} bytes={} ints={} eights={} types={} objs={} arrs={} unc={} comp={} blocks={} total={}",
        h.version, h.method, h.dict_id, h.frame_size, h.n_bytes, h.n_ints, h.n_eights, h.types_size, h.n_objects, h.n_arrays,
        h.uncompressed, h.compressed, h.block_count, h.block_total
    );
    let out = kv3::decompress(b, &h)?;
    println!("decompressed {} bytes", out.len());
    for (i, ch) in out.chunks(32).enumerate() {
        let hex: String = ch.iter().map(|b| format!("{b:02x} ")).collect();
        let asc: String = ch.iter().map(|&b| if (32..127).contains(&b) { b as char } else { '.' }).collect();
        println!("{:04x}: {hex:<96} {asc}", i * 32);
    }
    Ok(())
}
