use sbox_resource::{kv3, Resource};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let block = std::env::args().nth(2).unwrap();
    let data = std::fs::read(&path)?;
    let res = Resource::parse(&data)?;
    let b = res.block_data(&block).unwrap();
    let h = kv3::read_header(b)?;
    let tail_start = h.payload_start + h.compressed as usize;
    let tail = &b[tail_start..];
    println!("block len {} payload_start {} comp {} tail {} blocks {} total {}", b.len(), h.payload_start, h.compressed, tail.len(), h.block_count, h.block_total);
    println!("tail head: {:02x?}", &tail[..64.min(tail.len())]);
    let out = kv3::decompress(b, &h)?;
    // main buffer tail: after types, blob size table + trailer?
    let n = out.len();
    println!("main decompressed {n}; last 48: {:02x?}", &out[n - 48..]);
    Ok(())
}
