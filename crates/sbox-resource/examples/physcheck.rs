//! Compare the bounds of a model's collision shapes with its mesh bounds: physcheck <game dir> <model.vmdl_c>
use sbox_resource::{model, phys::PhysShape, vmat::GameFs};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::PathBuf::from(&a[1]);
    let mut fs = GameFs::from_game_dir(game.clone());
    fs.mount_map_packages(&game.join("core/maps"));
    let d = fs.read(&a[2]).expect("model");
    let m = model::load_with(&d, &|p| fs.read(p)).unwrap();
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for mm in m.default_meshes() {
        for k in 0..3 { mn[k] = mn[k].min(mm.mesh.mins[k]); mx[k] = mx[k].max(mm.mesh.maxs[k]); }
    }
    println!("mesh   {mn:.1?} {mx:.1?}");
    let Some(list) = &m.physics else { return println!("no usable physics") };
    let (mut pn, mut px) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut add = |p: &[f32; 3], r: f32| for k in 0..3 { pn[k] = pn[k].min(p[k] - r); px[k] = px[k].max(p[k] + r); };
    for s in list {
        match s {
            PhysShape::Hull(v) => v.iter().for_each(|p| add(p, 0.0)),
            PhysShape::Sphere { center, radius } => add(center, *radius),
            PhysShape::Capsule { a, b, radius } => { add(a, *radius); add(b, *radius) }
        }
    }
    println!("shapes {pn:.1?} {px:.1?} ({} shapes)", list.len());
}
