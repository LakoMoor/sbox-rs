use sbox_resource::{anim, kv3, Resource, Value};

fn main() -> anyhow::Result<()> {
    let model = std::fs::read(std::env::args().nth(1).unwrap())?;
    let res = Resource::parse(&model)?;
    let kv = kv3::parse(res.block_data("DATA").unwrap())?;
    let sk = kv.get("m_modelSkeleton").unwrap();
    let names: Vec<&str> = sk.get("m_boneName").and_then(Value::as_array).unwrap().iter().filter_map(Value::as_str).collect();
    let pos: Vec<[f32; 3]> = sk
        .get("m_bonePosParent")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|v| {
            let a = v.as_array().unwrap();
            [a[0].as_f64().unwrap() as f32, a[1].as_f64().unwrap() as f32, a[2].as_f64().unwrap() as f32]
        })
        .collect();
    println!("bones {} pos {}", names.len(), pos.len());
    let a = anim::parse(&std::fs::read(std::env::args().nth(2).unwrap())?)?;
    println!("anim tracks {}", a.tracks.len());
    // static-translation tracks should equal bind pos when not overridden by animation
    for off in 0..4i32 {
        let mut matches = 0;
        for (i, p) in pos.iter().enumerate() {
            let j = i as i32 + off;
            if j < 0 || j as usize >= a.tracks.len() {
                continue;
            }
            let t = &a.tracks[j as usize];
            if t.translation_static && (0..3).all(|k| (t.translation_start[k] - p[k]).abs() < 0.01) {
                matches += 1;
            }
        }
        println!("offset {off}: {matches} static tracks equal bind pos");
    }
    let m = sbox_resource::model::load(&model)?;
    let (mut direct, mut conj, mut tot) = (0, 0, 0);
    for (i, br) in m.skeleton.bind_rot.iter().enumerate() {
        let t = &a.tracks[i + 1];
        if !t.rotation_static {
            continue;
        }
        tot += 1;
        let q = t.constant_rotation;
        let d: f32 = (0..4).map(|k| q[k] * br[k]).sum::<f32>().abs();
        let c: f32 = (-q[0] * br[0] - q[1] * br[1] - q[2] * br[2] + q[3] * br[3]).abs();
        if d > 0.999 {
            direct += 1;
        }
        if c > 0.999 {
            conj += 1;
        }
    }
    println!("static rotation tracks: {tot}, equal to bind rot: direct {direct}, conjugate {conj}");
    for i in 0..6 {
        println!("bone {i} {} pos {:?}  track{i} start {:?}", names[i], pos[i], a.tracks[i].translation_start);
    }
    Ok(())
}
