//! Print every key (by known name) of entities of a class in a VPK: entdump1 <vpk> <classname>
use sbox_resource::{vents, vpk::Vpk};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let v = Vpk::open(std::path::Path::new(&a[1])).unwrap();
    let names = ["origin","angles","scales","targetname","box_mins","box_maxs","fogstrength","falloffexponent","shape","fadeinstart","fadeinend","drawdistance","anisotropy","indirectstrength","color","rendercolor","enabled","skyname"];
    for name in v.entries.keys().filter(|k| k.ends_with(".vents_c")) {
        for e in vents::load(&v.read(name).unwrap()).unwrap_or_default() {
            if !e.classname().starts_with(a[2].as_str()) { continue }
            println!("-- {}", e.classname());
            for n in names {
                if let Some(x) = e.get(n) { println!("  {n} = {x:?}"); }
            }
        }
    }
}
