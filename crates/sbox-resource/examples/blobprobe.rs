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
    let al = |x: usize, a: usize| x.div_ceil(a) * a;
    let e = al(h.n_ints as usize * 4, 8) + h.n_eights as usize * 8;
    let n0 = out.len();
    let extra = n0 as i64 - (e + h.types_size as usize) as i64 - 4 * h.block_count as i64 - 4;
    println!("extra bytes after trailer: {extra}, frames of 16384: {}", (h.block_total as usize).div_ceil(16384));
    {
        let sizes_off = e + h.types_size as usize;
        let sizes: Vec<u32> = (0..h.block_count as usize)
            .map(|i| u32::from_le_bytes(out[sizes_off + i * 4..sizes_off + i * 4 + 4].try_into().unwrap()))
            .collect();
        let cs_off = sizes_off + 4 * h.block_count as usize + 4;
        let comp: Vec<usize> = (0..h.block_count as usize)
            .map(|i| u16::from_le_bytes([out[cs_off + i * 2], out[cs_off + i * 2 + 1]]) as usize)
            .collect();
        println!("sizes[..4]={:?} comp[..4]={:?} sum comp {}", &sizes[..4.min(sizes.len())], &comp[..4.min(comp.len())], comp.iter().sum::<usize>());
        let mut pos = 0;
        let mut history: Vec<u8> = Vec::new();
        let (mut okc, mut bad) = (0, 0);
        for (s, c) in sizes.iter().zip(&comp) {
            if pos + c > tail.len() {
                bad += 1;
                break;
            }
            let dict = &history[history.len().saturating_sub(65536)..];
            match lz4_flex::block::decompress_with_dict(&tail[pos..pos + c], *s as usize, dict) {
                Ok(o) => {
                    okc += 1;
                    history.extend_from_slice(&o);
                }
                Err(_) => bad += 1,
            }
            pos += c;
        }
        println!("per-blob lz4: ok {okc} bad {bad}, consumed {pos} of {}", tail.len());
    }
    let n = out.len();
    println!("main decompressed {n}; last 48: {:02x?}", &out[n - 48..]);
    Ok(())
}
