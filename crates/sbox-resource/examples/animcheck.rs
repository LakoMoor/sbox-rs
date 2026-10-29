//! Validates anim decoding: successive frames of an animated rotation/translation track should be close.
use sbox_resource::anim;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let a = anim::parse(&std::fs::read(path)?)?;
    println!("{} frames {} dur {:.2}s tracks {} skeleton {}", a.frames, a.frames, a.duration, a.tracks.len(), a.skeleton);
    let (mut worst_rot, mut worst_pos, mut n) = (1.0f32, 0.0f32, 0);
    for f in 0..a.frames.saturating_sub(1) {
        let (p, q) = (a.sample_frame(f), a.sample_frame(f + 1));
        for (i, t) in a.tracks.iter().enumerate() {
            if !t.rotation_static {
                let dot: f32 = (0..4).map(|k| p[i].rotation[k] * q[i].rotation[k]).sum::<f32>().abs();
                worst_rot = worst_rot.min(dot);
                n += 1;
            }
            if !t.translation_static {
                let d: f32 = (0..3).map(|k| (p[i].position[k] - q[i].position[k]).powi(2)).sum::<f32>().sqrt();
                worst_pos = worst_pos.max(d);
            }
        }
    }
    // raw dump of first animated-rotation track
    let mut off = 0usize;
    for (i, t) in a.tracks.iter().enumerate() {
        if !t.rotation_static {
            println!("track {i}: translation_static={}", t.translation_static);
            break;
        }
        off += (!t.translation_static) as usize * 3 + (!t.scale_static) as usize;
        let _ = off;
    }
    for f in 0..8 {
        let r = a.raw_frame(f);
        // track 1 is the first animated one: read offset 0
        println!("f{f} raw {:04x} {:04x} {:04x}  | {:016b} {:016b} {:016b}", r[0], r[1], r[2], r[0], r[1], r[2]);
    }
    for (i, t) in a.tracks.iter().enumerate().filter(|(_, t)| !t.rotation_static).take(12) {
        let mut worst = 1.0f32;
        for f in 0..a.frames - 1 {
            let (p, q) = (a.sample_frame(f)[i].rotation, a.sample_frame(f + 1)[i].rotation);
            worst = worst.min((0..4).map(|k| p[k] * q[k]).sum::<f32>().abs());
        }
        let r = a.raw_frame(0);
        let _ = t;
        println!("track {i}: worst dot {worst:.4}, q0 {:?}", a.sample_frame(0)[i].rotation);
        let _ = r;
    }
    println!("compared {n} rotation pairs: worst |dot| = {worst_rot:.4} (1.0 = identical), worst translation step = {worst_pos:.3}");
    Ok(())
}
