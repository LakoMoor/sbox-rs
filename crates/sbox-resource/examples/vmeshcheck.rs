use sbox_resource::{mesh, vmat::GameFs};

fn main() {
    let game = std::env::args().nth(1).unwrap();
    let rel = std::env::args().nth(2).unwrap();
    let fs = GameFs::from_game_dir(std::path::Path::new(&game));
    let Some(b) = fs.read(&rel) else { return println!("not found: {rel}") };
    match mesh::load_embedded(&b) {
        Ok(m) => println!("ok: {} draw calls", m.draw_calls.len()),
        Err(e) => println!("err: {e:#}"),
    }
}
