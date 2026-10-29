fn main() {
    let a = sbox_resource::anim::parse(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    println!("additive={} frames={} dur={} tracks={} skeleton={}", a.additive, a.frames, a.duration, a.tracks.len(), a.skeleton);
}
