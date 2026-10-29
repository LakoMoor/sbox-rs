use sbox_resource::{vbib, Resource};

fn main() -> anyhow::Result<()> {
    let data = std::fs::read(std::env::args().nth(1).unwrap())?;
    let res = Resource::parse(&data)?;
    let v = vbib::parse(res.block_data("MBUF").unwrap())?;
    let vb = &v.vertex_buffers[0];
    let ib = &v.index_buffers[0];
    let pos = |i: usize| -> [f32; 3] {
        let b = i * vb.stride as usize;
        let f = |o: usize| f32::from_le_bytes(vb.data[b + o..b + o + 4].try_into().unwrap());
        [f(0), f(4), f(8)]
    };
    let n = vb.count as usize;
    let mut gn = vec![[0f32; 3]; n];
    for t in ib.data.chunks(6) {
        let i: Vec<usize> = (0..3).map(|k| u16::from_le_bytes([t[k * 2], t[k * 2 + 1]]) as usize).collect();
        let (a, b, c) = (pos(i[0]), pos(i[1]), pos(i[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cr = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        for &k in &i {
            for j in 0..3 {
                gn[k][j] += cr[j];
            }
        }
    }
    for i in (0..n).step_by(n / 14) {
        let b = i * vb.stride as usize + 16;
        let raw = u32::from_le_bytes(vb.data[b..b + 4].try_into().unwrap());
        let l = (gn[i][0].powi(2) + gn[i][1].powi(2) + gn[i][2].powi(2)).sqrt().max(1e-9);
        println!(
            "v{i:3} raw {:3} {:3} {:3} {} bytes {:02x?}  geo n = {:5.2} {:5.2} {:5.2}",
            raw & 0x3ff, (raw >> 10) & 0x3ff, (raw >> 20) & 0x3ff, raw >> 30,
            &vb.data[b..b + 4], gn[i][0] / l, gn[i][1] / l, gn[i][2] / l
        );
    }
    Ok(())
}
