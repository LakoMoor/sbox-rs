//! Reduced particle system: `ParticleEffect` (simulation) + sphere / cone emitters + sprite renderer.
//!
//! Values in scene files are either numbers or small objects (`Range`, `Curve`, `CurveRange`); they are
//! evaluated against particle life, a per-particle seed, or time. Curve tangents are ignored (linear segments).
//! Not supported: collisions, followers, sprite sheet animation, trails/stretch, local space, force/orbital fields
//! beyond a constant force direction, depth feathering.

use sbox_math::{Angles, Transform, Vec3};
use sbox_render::{MaterialDesc, MeshData, Renderer, Vertex};
use sbox_scene::{impl_component_any, parse_color, CameraComponent, Component, GoId, Handle, ModelRenderer, Scene};
use serde_json::Value;

/// A scalar that may vary over particle life or between particles.
#[derive(Clone, Debug)]
pub enum PVal {
    Const(f32),
    Range(f32, f32),
    Curve(Vec<(f32, f32)>),
    CurveRange(Vec<(f32, f32)>, Vec<(f32, f32)>),
}

fn parse_frames(v: &Value) -> Vec<(f32, f32)> {
    let (frames, lo, hi) = match v {
        Value::Array(a) => (a.as_slice(), 0.0, 1.0),
        Value::Object(o) => {
            let ry = o.get("rangey").and_then(Value::as_str).map(|s| s.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>());
            let (lo, hi) = ry.filter(|r| r.len() >= 2).map_or((0.0, 1.0), |r| (r[0], r[1]));
            (o.get("frames").and_then(Value::as_array).map_or(&[][..], Vec::as_slice), lo, hi)
        }
        _ => return Vec::new(),
    };
    let mut pts: Vec<(f32, f32)> = frames
        .iter()
        .filter_map(|f| Some((f.get("x")?.as_f64()? as f32, lo + f.get("y")?.as_f64()? as f32 * (hi - lo))))
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts
}

fn sample(pts: &[(f32, f32)], t: f32) -> f32 {
    match pts {
        [] => 0.0,
        [only] => only.1,
        _ => {
            if t <= pts[0].0 {
                return pts[0].1;
            }
            for w in pts.windows(2) {
                if t <= w[1].0 {
                    let f = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
                    return w[0].1 + (w[1].1 - w[0].1) * f;
                }
            }
            pts[pts.len() - 1].1
        }
    }
}

impl PVal {
    pub fn parse(v: Option<&Value>, default: f32) -> PVal {
        match v {
            Some(Value::Number(n)) => PVal::Const(n.as_f64().unwrap_or(default as f64) as f32),
            Some(Value::Object(o)) => {
                let nums = |k: &str| -> Vec<f32> {
                    o.get(k).and_then(Value::as_str).map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default()
                };
                match o.get("Type").and_then(Value::as_str) {
                    Some("Range") => {
                        let c = nums("Constants");
                        if c.len() >= 2 {
                            PVal::Range(c[0], c[1])
                        } else {
                            PVal::Const(o.get("ConstantA").and_then(Value::as_f64).unwrap_or(default as f64) as f32)
                        }
                    }
                    Some("Curve") => PVal::Curve(parse_frames(o.get("CurveA").unwrap_or(&Value::Null))),
                    Some("CurveRange") => PVal::CurveRange(parse_frames(o.get("CurveA").unwrap_or(&Value::Null)), parse_frames(o.get("CurveB").unwrap_or(&Value::Null))),
                    _ => PVal::Const(default),
                }
            }
            _ => PVal::Const(default),
        }
    }

    pub fn eval(&self, life: f32, seed: f32) -> f32 {
        match self {
            PVal::Const(c) => *c,
            PVal::Range(a, b) => a + (b - a) * seed,
            PVal::Curve(p) => sample(p, life),
            PVal::CurveRange(a, b) => {
                let (x, y) = (sample(a, life), sample(b, life));
                x + (y - x) * seed
            }
        }
    }
}

/// xorshift-based hash giving a uniform value in [0, 1).
pub fn hash01(mut x: u32) -> f32 {
    // murmur3 finaliser: full avalanche, so consecutive seeds give unrelated values
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub life: f32,
    pub seed: u32,
    pub roll: f32,
}

/// Simulation settings and live particles of one `Sandbox.ParticleEffect`.
pub struct ParticleEffect {
    pub max: usize,
    pub lifetime: PVal,
    pub velocity: [PVal; 3],
    pub damping: f32,
    pub force: Option<Vec3>,
    pub constant_movement: [PVal; 3],
    pub scale: PVal,
    pub alpha: PVal,
    pub brightness: PVal,
    pub roll: PVal,
    pub color_a: [f32; 4],
    pub color_b: [f32; 4],
    pub tint: [f32; 4],
    pub particles: Vec<Particle>,
    counter: u32,
}

impl ParticleEffect {
    pub fn from_json(p: &Value) -> Self {
        let vec3 = |k: &str| {
            let v = p.get(k);
            [0, 1, 2].map(|i| PVal::parse(v.and_then(|v| v.get(["X", "Y", "Z"][i])), 0.0))
        };
        let grad = p.get("Gradient");
        let (ca, cb) = match grad.and_then(|g| g.get("Type")).and_then(Value::as_str) {
            Some("Range") => (parse_color(grad.and_then(|g| g.get("ConstantA")), [1.0; 4]), parse_color(grad.and_then(|g| g.get("ConstantB")), [1.0; 4])),
            _ => {
                let c = parse_color(grad.and_then(|g| g.get("ConstantA")), [1.0; 4]);
                (c, c)
            }
        };
        let force = p.get("Force").and_then(Value::as_bool).unwrap_or(false);
        let fd = sbox_scene::parse_vec3(p.get("ForceDirection"), Vec3::ZERO);
        Self {
            max: p.get("MaxParticles").and_then(Value::as_u64).unwrap_or(1000) as usize,
            lifetime: PVal::parse(p.get("Lifetime"), 1.0),
            velocity: vec3("InitialVelocity"),
            damping: p.get("Damping").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            force: force.then_some(fd * p.get("ForceScale").and_then(Value::as_f64).unwrap_or(1.0) as f32),
            constant_movement: vec3("ConstantMovement"),
            scale: PVal::parse(p.get("Scale"), 1.0),
            alpha: PVal::parse(p.get("Alpha"), 1.0),
            brightness: PVal::parse(p.get("Brightness"), 1.0),
            roll: PVal::parse(p.get("Roll"), 0.0),
            color_a: ca,
            color_b: cb,
            tint: parse_color(p.get("Tint"), [1.0; 4]),
            particles: Vec::new(),
            counter: 1,
        }
    }

    pub fn spawn(&mut self, pos: Vec3, dir: Vec3, speed: f32) {
        if self.particles.len() >= self.max {
            return;
        }
        self.counter = self.counter.wrapping_add(1);
        let seed = self.counter.wrapping_mul(0x9E3779B1);
        let r = |k: u32| hash01(seed.wrapping_add(k.wrapping_mul(0x85EBCA6B)));
        let v = Vec3::new(self.velocity[0].eval(0.0, r(1)), self.velocity[1].eval(0.0, r(2)), self.velocity[2].eval(0.0, r(3)));
        self.particles.push(Particle {
            pos,
            vel: dir * speed + v,
            age: 0.0,
            life: self.lifetime.eval(0.0, r(4)).max(0.01),
            seed,
            roll: self.roll.eval(0.0, r(5)),
        });
    }

    pub fn step(&mut self, dt: f32) {
        let (damping, force) = (self.damping, self.force.unwrap_or(Vec3::ZERO));
        let cm = &self.constant_movement;
        for p in &mut self.particles {
            p.age += dt;
            let life = p.age / p.life;
            let r = |k: u32| hash01(p.seed.wrapping_add(k.wrapping_mul(0x85EBCA6B)));
            p.vel += force * dt;
            p.vel *= (1.0 - damping * dt).max(0.0);
            let drift = Vec3::new(cm[0].eval(life, r(6)), cm[1].eval(life, r(7)), cm[2].eval(life, r(8)));
            p.pos += (p.vel + drift) * dt;
        }
        self.particles.retain(|p| p.age < p.life);
    }
}

/// Emitter shapes. `cone_angle` > 0 for cone emitters (degrees, around the object's +Z... forward axis).
pub struct ParticleEmitter {
    pub burst: u32,
    pub rate: f32,
    pub velocity: f32,
    pub radius: f32,
    pub delay: f32,
    pub duration: f32,
    pub looping: bool,
    pub cone_angle: Option<f32>,
    time: f32,
    burst_done: bool,
    carry: f32,
}

impl ParticleEmitter {
    pub fn from_json(p: &Value, cone: bool) -> Self {
        let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
        Self {
            burst: f("Burst", 0.0) as u32,
            rate: f("Rate", 0.0),
            velocity: f("Velocity", 0.0),
            radius: f("Radius", 0.0),
            delay: f("Delay", 0.0),
            duration: f("Duration", 1.0),
            looping: p.get("Loop").and_then(Value::as_bool).unwrap_or(false),
            cone_angle: cone.then(|| f("ConeAngle", 30.0)),
            time: 0.0,
            burst_done: false,
            carry: 0.0,
        }
    }
}

impl Component for ParticleEmitter {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let dt = scene.delta;
        self.time += dt;
        let xf = scene.world_transform(h.go);
        let n = if !self.burst_done && self.time >= self.delay {
            self.burst_done = true;
            self.burst
        } else {
            0
        };
        let t = self.time - self.delay;
        let active = t >= 0.0 && (self.looping || t <= self.duration);
        self.carry += if active { self.rate * dt } else { 0.0 };
        let extra = self.carry as u32;
        self.carry -= extra as f32;
        let (cone, radius, speed, up) = (self.cone_angle, self.radius, self.velocity, xf.rotation * Vec3::X);
        let seed_base = (scene.time * 1000.0) as u32;
        let Some(fx) = scene.component_mut::<ParticleEffect>(h.go) else { return };
        for i in 0..n + extra {
            let s = seed_base.wrapping_add(i.wrapping_mul(2654435761)).wrapping_add(fx.counter);
            let r = |k: u32| hash01(s.wrapping_add(k.wrapping_mul(0x27D4EB2F)));
            let dir = match cone {
                Some(angle) => {
                    // random direction within the cone around the emitter's forward axis
                    let a = r(1) * angle.to_radians();
                    let phi = r(2) * std::f32::consts::TAU;
                    let local = Vec3::new(a.cos(), a.sin() * phi.cos(), a.sin() * phi.sin());
                    xf.rotation * local
                }
                None => {
                    let z = r(1) * 2.0 - 1.0;
                    let phi = r(2) * std::f32::consts::TAU;
                    let rr = (1.0 - z * z).sqrt();
                    Vec3::new(rr * phi.cos(), rr * phi.sin(), z)
                }
            };
            let _ = up;
            let offset = dir * radius * r(3).cbrt();
            fx.spawn(xf.position + offset, dir, speed);
        }
    }
    impl_component_any!();
}

/// Draws the effect's particles as camera-facing quads, using a pool of child objects.
pub struct ParticleSpriteRenderer {
    pub scale: f32,
    pool: Vec<GoId>,
    /// One quad per sheet frame (a single entry for plain sprites).
    pub meshes: Vec<sbox_scene::MeshId>,
    pub material: sbox_scene::MaterialId,
    pub brightness_applies: bool,
}

impl ParticleSpriteRenderer {
    /// `texture` is the sprite's first frame; `additive` selects additive blending.
    pub fn new(r: &mut Renderer, tex: Option<sbox_render::TextureId>, sheet: &[[f32; 2]], scale: f32, additive: bool) -> Self {
        let material = r.create_material_ex(&MaterialDesc { color: tex, unlit: true, translucent: true, additive, ..Default::default() });
        let v = |y: f32, z: f32, u: f32, vv: f32| Vertex { position: [0.0, y, z], normal: [1.0, 0.0, 0.0], uv: [u, vv], joints: [0; 4], weights: [0.0; 4] };
        let cells: Vec<([f32; 2], f32)> = if sheet.is_empty() {
            vec![([0.0, 0.0], 1.0)]
        } else {
            let size = 1.0 / (sheet.len() as f32).sqrt().ceil();
            sheet.iter().map(|o| (*o, size)).collect()
        };
        let meshes = cells
            .iter()
            .map(|&([u0, v0], s)| {
                r.upload_mesh(&MeshData {
                    vertices: vec![v(0.5, 0.5, u0, v0), v(-0.5, 0.5, u0 + s, v0), v(-0.5, -0.5, u0 + s, v0 + s), v(0.5, -0.5, u0, v0 + s)],
                    indices: vec![0, 1, 2, 0, 2, 3],
                })
            })
            .collect();
        Self { scale, pool: Vec::new(), meshes, material, brightness_applies: additive }
    }
}

impl Component for ParticleSpriteRenderer {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let cam = scene.iter_components::<CameraComponent>().next().map(|(go, _)| scene.world_transform(go).position);
        // Snapshot render state first (the effect lives on the same object).
        let nframes = self.meshes.len();
        let draws: Vec<(Vec3, f32, f32, [f32; 4], usize)> = match scene.component::<ParticleEffect>(h.go) {
            Some(fx) => fx
                .particles
                .iter()
                .map(|p| {
                    let life = p.age / p.life;
                    let r = |k: u32| hash01(p.seed.wrapping_add(k.wrapping_mul(0x85EBCA6B)));
                    let size = fx.scale.eval(life, r(9)) * self.scale;
                    let alpha = fx.alpha.eval(life, r(10)).clamp(0.0, 1.0) * fx.tint[3];
                    let c: [f32; 4] = std::array::from_fn(|i| fx.color_a[i] + (fx.color_b[i] - fx.color_a[i]) * r(11));
                    let b = if self.brightness_applies { fx.brightness.eval(life, r(12)).clamp(0.0, 4.0) } else { 1.0 };
                    (p.pos, size, p.roll, [c[0] * fx.tint[0] * b, c[1] * fx.tint[1] * b, c[2] * fx.tint[2] * b, alpha * c[3]], ((life.clamp(0.0, 0.9999)) * nframes as f32) as usize)
                })
                .collect(),
            None => return,
        };
        while self.pool.len() < draws.len() {
            let go = scene.create_object_in(Some(h.go), "particle");
            let mut m = ModelRenderer::new(self.meshes[0]);
            m.material = self.material;
            m.cast_shadows = false;
            scene.add_component(go, m);
            self.pool.push(go);
        }
        for (i, &go) in self.pool.iter().enumerate() {
            match draws.get(i) {
                Some(&(pos, size, roll, color, frame)) => {
                    let to_cam = cam.map_or(Vec3::X, |c| (c - pos).normalize_or_zero());
                    let pitch = -to_cam.z.clamp(-1.0, 1.0).asin().to_degrees();
                    let yaw = to_cam.y.atan2(to_cam.x).to_degrees();
                    scene.set_world_transform(go, Transform::new(pos, Angles::new(pitch, yaw, roll).to_rotation(), Vec3::splat(size.max(0.001))));
                    if let Some(m) = scene.component_mut::<ModelRenderer>(go) {
                        m.tint = color;
                        m.mesh = self.meshes[frame.min(self.meshes.len() - 1)];
                    }
                }
                None => {
                    if let Some(m) = scene.component_mut::<ModelRenderer>(go) {
                        m.tint[3] = 0.0;
                    }
                }
            }
        }
    }
    impl_component_any!();
}

/// Steps all effects. Runs before the renderers / emitters because it is its own component on the effect object.
impl Component for ParticleEffect {
    fn on_update(&mut self, scene: &mut Scene, _h: Handle) {
        self.step(scene.delta);
    }
    impl_component_any!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_evaluate() {
        let v = PVal::parse(Some(&json!({"Type": "Range", "Evaluation": "Seed", "Constants": "1,4,0,0"})), 0.0);
        assert!((v.eval(0.0, 0.5) - 2.5).abs() < 1e-5);
        let c = PVal::parse(Some(&json!({"Type": "Curve", "CurveA": [{"x": 0, "y": 0}, {"x": 1, "y": 10}]})), 0.0);
        assert!((c.eval(0.25, 0.0) - 2.5).abs() < 1e-5);
        let r = PVal::parse(Some(&json!({"Type": "Curve", "CurveA": {"rangey": "0,200", "frames": [{"x": 0, "y": 1}, {"x": 1, "y": 0}]}})), 0.0);
        assert!((r.eval(0.5, 0.0) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn burst_directions_are_not_a_lattice() {
        let mut scene = Scene::new();
        let go = scene.create_object("fx");
        scene.add_component(go, ParticleEffect::from_json(&json!({"MaxParticles": 100, "Lifetime": 5.0})));
        scene.add_component(go, ParticleEmitter::from_json(&json!({"Burst": 25, "Velocity": 100.0, "Radius": 0.0}), false));
        scene.tick(1.0 / 60.0);
        scene.tick(1.0 / 60.0);
        let fx = scene.component::<ParticleEffect>(go).unwrap();
        let dirs: Vec<Vec3> = fx.particles.iter().map(|p| p.vel.normalize()).collect();
        println!("{:?}", &dirs[..6]);
        let mut distinct = std::collections::HashSet::new();
        for d in &dirs {
            distinct.insert(((d.x * 20.0).round() as i32, (d.y * 20.0).round() as i32));
        }
        assert!(distinct.len() >= 20, "only {} distinct directions of 25", distinct.len());
    }

    #[test]
    fn burst_emits_simulates_and_expires() {
        let mut scene = Scene::new();
        let go = scene.create_object("fx");
        let fx = ParticleEffect::from_json(&json!({"MaxParticles": 50, "Lifetime": 0.5, "Damping": 0.0}));
        scene.add_component(go, fx);
        let em = ParticleEmitter::from_json(&json!({"Burst": 5, "Velocity": 100.0, "Radius": 0.0, "Duration": 1.0}), false);
        scene.add_component(go, em);
        scene.tick(1.0 / 60.0);
        scene.tick(1.0 / 60.0);
        let fx = scene.component::<ParticleEffect>(go).unwrap();
        assert_eq!(fx.particles.len(), 5, "burst of five");
        assert!(fx.particles.iter().all(|p| p.pos.length() > 0.5), "particles have moved away from the origin");
        for _ in 0..60 {
            scene.tick(1.0 / 60.0);
        }
        assert!(scene.component::<ParticleEffect>(go).unwrap().particles.is_empty(), "all expired after the lifetime");
    }
}
