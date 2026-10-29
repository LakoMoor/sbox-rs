use sbox_resource::mesh;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let m = mesh::load_embedded(&std::fs::read(path)?)?;
    println!("bounds {:?} .. {:?}", m.mins, m.maxs);
    for dc in &m.draw_calls {
        let lens: Vec<f32> = dc.vertices.iter().map(|v| (v.normal[0].powi(2) + v.normal[1].powi(2) + v.normal[2].powi(2)).sqrt()).collect();
        let (mn, mx) = lens.iter().fold((9.0f32, 0.0f32), |a, &l| (a.0.min(l), a.1.max(l)));
        let maxi = dc.indices.iter().max().copied().unwrap_or(0);
        println!("{} verts {} idx {} (max idx {maxi}) |n| in [{mn:.3},{mx:.3}] first {:?}", dc.material, dc.vertices.len(), dc.indices.len(), dc.vertices[0]);
    }
    Ok(())
}
