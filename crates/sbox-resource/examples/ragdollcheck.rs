//! Count ragdoll parts and joints per model: ragdollcheck <game dir> <model.vmdl_c>
use sbox_resource::{kv3, vmat::GameFs, Resource};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::PathBuf::from(&a[1]);
    let mut fs = GameFs::from_game_dir(game.clone());
    fs.mount_map_packages(&game.join("core/maps"));
    let data = fs.read(&a[2]).expect("model");
    let res = Resource::parse(&data).unwrap();
    let kv = kv3::parse(res.block_data("DATA").unwrap()).unwrap();
    match sbox_resource::phys::ragdoll(&res, &kv, &|p| fs.read(p)) {
        Some((parts, joints)) => {
            println!("{} parts, {} joints", parts.len(), joints.len());
            for (i, p) in parts.iter().enumerate().take(4) { println!("  part {i}: {} shapes at {:?} rot {:?} mass {}", p.shapes.len(), p.position, p.rotation, p.mass); }
            for j in joints.iter().take(3) { println!("  joint {}-{} swing {:?} twist {:?}", j.body1, j.body2, j.swing, j.twist); }
        }
        None => println!("no ragdoll"),
    }
}
