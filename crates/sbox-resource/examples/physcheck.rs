//! Print the collision shapes of a model: physcheck <game dir> <model.vmdl_c>
use sbox_resource::{model, vmat::GameFs};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::PathBuf::from(&a[1]);
    let mut fs = GameFs::from_game_dir(game.clone());
    fs.mount_map_packages(&game.join("core/maps"));
    let d = fs.read(&a[2]).expect("model");
    let m = model::load_with(&d, &|p| fs.read(p)).unwrap();
    match &m.physics {
        Some(list) => list.iter().for_each(|s| println!("{s:?}")),
        None => println!("no usable physics"),
    }
}
