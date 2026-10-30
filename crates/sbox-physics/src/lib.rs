//! Rigid-body physics for sbox-scene, backed by rapier. Replaces the Havok wrapper of the
//! closed Source 2 core. Units are s&box units (inches); gravity defaults to 800 u/s^2 along -Z.

use rapier3d::na::{Isometry3, Quaternion, Translation3, UnitQuaternion, Vector3};
use rapier3d::prelude::*;
use sbox_math::{Angles, Quat, Transform, Vec3};
use sbox_scene::{impl_component_any, Component, GoId, Input, Scene};
use std::collections::HashMap;

/// `Sandbox.Rigidbody` (subset).
pub struct Rigidbody {
    pub gravity: bool,
    pub motion_enabled: bool,
    pub mass_override: Option<f32>,
    /// The body follows the object's transform (set by the game) and pushes dynamic bodies, but is not moved by them.
    pub kinematic: bool,
}

impl Default for Rigidbody {
    fn default() -> Self {
        Self { gravity: true, motion_enabled: true, mass_override: None, kinematic: false }
    }
}
impl Component for Rigidbody {
    impl_component_any!();
}

/// First-person capsule controller (`Sandbox.PlayerController`, reduced).
/// Actions read from [`Input`]: Forward, Backward, Left, Right, Jump, Run, Duck.
pub struct PlayerController {
    pub walk_speed: f32,
    pub run_speed: f32,
    pub jump_speed: f32,
    pub radius: f32,
    pub height: f32,
    pub eye_height: f32,
    /// Walking speed and eye height while the `Duck` action is held.
    pub duck_speed: f32,
    pub duck_eye_height: f32,
    /// Eye height when standing; `eye_height` eases between this and the ducked height.
    pub stand_eye_height: f32,
    /// Mouse sensitivity in degrees per pixel.
    pub sensitivity: f32,
    /// Object moved to the eye position and rotated by yaw/pitch every step.
    pub camera: Option<GoId>,
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for PlayerController {
    fn default() -> Self {
        Self {
            walk_speed: 190.0,
            run_speed: 320.0,
            jump_speed: 320.0,
            radius: 16.0,
            height: 72.0,
            eye_height: 64.0,
            duck_speed: 90.0,
            duck_eye_height: 40.0,
            stand_eye_height: 64.0,
            sensitivity: 0.1,
            camera: None,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}
impl Component for PlayerController {
    impl_component_any!();
}

#[derive(Clone, Debug)]
pub enum Shape {
    /// Full extents in object-local units.
    Box { center: Vec3, size: Vec3 },
    Sphere { center: Vec3, radius: f32 },
    /// Infinite plane through the object origin, normal = object +Z.
    Plane,
    /// Convex hull of points in object-local units.
    Hull(Vec<Vec3>),
    /// Capsule between two object-local points.
    Capsule { a: Vec3, b: Vec3, radius: f32 },
    /// Several shapes acting as one collider (model collision data).
    Compound(Vec<Shape>),
    /// Triangle soup in object-local units. Concave and hollow: meant for static geometry (a dynamic body with one
    /// collides, but rapier gives it no sensible volume or mass).
    TriMesh { vertices: Vec<Vec3>, indices: Vec<[u32; 3]> },
}

impl Shape {
    /// Rapier shape and its local offset for a non-plane, non-compound shape at the given object scale.
    fn part(&self, scale: Vec3) -> Option<(Vec3, SharedShape)> {
        let p = |v: &Vec3| point![v.x * scale.x, v.y * scale.y, v.z * scale.z];
        Some(match self {
            Shape::Box { center, size } => {
                let h = *size * scale.abs() * 0.5;
                (*center * scale, SharedShape::cuboid(h.x.max(0.01), h.y.max(0.01), h.z.max(0.01)))
            }
            Shape::Sphere { center, radius } => (*center * scale, SharedShape::ball(radius * scale.abs().max_element())),
            Shape::Hull(points) => (Vec3::ZERO, SharedShape::convex_hull(&points.iter().map(p).collect::<Vec<_>>())?),
            Shape::Capsule { a, b, radius } => (Vec3::ZERO, SharedShape::capsule(p(a), p(b), radius * scale.abs().max_element())),
            Shape::TriMesh { vertices, indices } => {
                // Indices past the vertex list would panic inside parry: such a mesh is dropped.
                if indices.is_empty() || indices.iter().flatten().any(|&i| i as usize >= vertices.len()) {
                    return None;
                }
                (Vec3::ZERO, SharedShape::trimesh(vertices.iter().map(p).collect(), indices.clone()))
            }
            Shape::Plane | Shape::Compound(_) => return None,
        })
    }
}

/// `BoxCollider` / `SphereCollider` / `PlaneCollider`.
pub struct Collider {
    pub shape: Shape,
    pub is_trigger: bool,
}
impl Component for Collider {
    impl_component_any!();
}

pub struct Physics {
    pub gravity: Vec3,
    /// When false `step` does nothing (a network client lets the server move the bodies).
    pub enabled: bool,
    pipeline: PhysicsPipeline,
    params: IntegrationParameters,
    islands: IslandManager,
    broad: DefaultBroadPhase,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    /// Objects that own a rapier body.
    body_of: HashMap<GoId, RigidBodyHandle>,
    registered: std::collections::HashSet<GoId>,
    players: Vec<GoId>,
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, 0.0, -800.0),
            enabled: true,
            pipeline: PhysicsPipeline::new(),
            params: IntegrationParameters::default(),
            islands: IslandManager::new(),
            broad: DefaultBroadPhase::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            body_of: HashMap::new(),
            registered: Default::default(),
            players: Vec::new(),
        }
    }
}

fn iso(t: &Transform) -> Isometry3<f32> {
    Isometry3::from_parts(
        Translation3::new(t.position.x, t.position.y, t.position.z),
        UnitQuaternion::from_quaternion(Quaternion::new(t.rotation.w, t.rotation.x, t.rotation.y, t.rotation.z)),
    )
}

impl Physics {
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// Install physics into a scene: adds the world resource and the fixed-step system.
    pub fn install(scene: &mut Scene) {
        scene.insert_resource(Physics::default());
        scene.add_fixed_system(|scene| {
            scene.with_resource::<Physics, _>(|p, s| p.step(s));
        });
    }

    fn rigidbody_owner(scene: &Scene, mut go: GoId) -> Option<GoId> {
        loop {
            if scene.component::<Rigidbody>(go).is_some() {
                return Some(go);
            }
            go = scene.get(go)?.parent()?;
        }
    }

    fn register_new(&mut self, scene: &Scene) {
        let todo: Vec<GoId> = scene
            .all_objects()
            .filter(|g| !self.registered.contains(g) && scene.get(*g).map_or(false, |o| o.active()))
            .filter(|g| {
                scene.component::<Collider>(*g).is_some()
                    || scene.component::<Rigidbody>(*g).is_some()
                    || scene.component::<PlayerController>(*g).is_some()
            })
            .collect();
        for go in todo {
            self.registered.insert(go);
            let world = scene.world_transform(go);
            // Rigidbody on this object -> create its body
            if let Some(rb) = scene.component::<Rigidbody>(go) {
                let builder = if rb.kinematic {
                    RigidBodyBuilder::kinematic_position_based()
                } else if rb.motion_enabled {
                    RigidBodyBuilder::dynamic()
                } else {
                    RigidBodyBuilder::fixed()
                };
                let body = builder.position(iso(&world)).gravity_scale(if rb.gravity { 1.0 } else { 0.0 }).build();
                self.body_of.insert(go, self.bodies.insert(body));
            }
            if let Some(pc) = scene.component::<PlayerController>(go) {
                let body = RigidBodyBuilder::dynamic()
                    .position(iso(&Transform::new(world.position, Quat::IDENTITY, Vec3::ONE)))
                    .lock_rotations()
                    .can_sleep(false)
                    .build();
                let h = self.bodies.insert(body);
                let half = ((pc.height - 2.0 * pc.radius) * 0.5).max(0.01);
                let col = ColliderBuilder::capsule_z(half, pc.radius)
                    .translation(Vector3::new(0.0, 0.0, pc.height * 0.5))
                    .friction(0.0)
                    .build();
                self.colliders.insert_with_parent(col, h, &mut self.bodies);
                self.body_of.insert(go, h);
                self.players.push(go);
                continue;
            }
            let Some(c) = scene.component::<Collider>(go) else { continue };
            let (shape, extra) = match &c.shape {
                Shape::Plane => (ColliderBuilder::halfspace(Vector3::z_axis()), Vec3::ZERO),
                Shape::Compound(list) => {
                    let parts: Vec<_> = list
                        .iter()
                        .filter_map(|s| s.part(world.scale))
                        .map(|(o, sh)| (Isometry3::translation(o.x, o.y, o.z), sh))
                        .collect();
                    if parts.is_empty() {
                        continue;
                    }
                    (ColliderBuilder::compound(parts), Vec3::ZERO)
                }
                single => {
                    let Some((offset, sh)) = single.part(world.scale) else { continue };
                    (ColliderBuilder::new(sh), offset)
                }
            };
            let builder = shape.sensor(c.is_trigger);
            match Self::rigidbody_owner(scene, go).and_then(|o| self.body_of.get(&o).map(|h| (o, *h))) {
                Some((owner, handle)) => {
                    let local = scene.world_transform(owner).to_local(&Transform::new(world.position, world.rotation, Vec3::ONE));
                    let mut t = local;
                    t.position += local.rotation * extra;
                    let col = builder.position(iso(&t)).build();
                    self.colliders.insert_with_parent(col, handle, &mut self.bodies);
                }
                None => {
                    let mut t = Transform::new(world.position, world.rotation, Vec3::ONE);
                    t.position += world.rotation * extra;
                    self.colliders.insert(builder.position(iso(&t)).build());
                }
            }
        }
    }

    fn drive_players(&mut self, scene: &mut Scene) {
        if self.players.is_empty() {
            return;
        }
        let input = scene.resource::<Input>().cloned().unwrap_or_default();
        let (mx, my) = scene.resource_mut::<Input>().map(Input::take_mouse).unwrap_or((0.0, 0.0));
        for &go in &self.players {
            let Some(&h) = self.body_of.get(&go) else { continue };
            let Some(pc) = scene.component_mut::<PlayerController>(go) else { continue };
            if input.mouse_captured {
                pc.yaw -= mx * pc.sensitivity;
                pc.pitch = (pc.pitch + my * pc.sensitivity).clamp(-89.0, 89.0);
            }
            let yaw = pc.yaw.to_radians();
            let fwd = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
            let left = Vec3::new(-yaw.sin(), yaw.cos(), 0.0);
            let wish = fwd * input.axis("Forward", "Backward") + left * input.axis("Left", "Right");
            let ducking = input.down("Duck");
            let speed = if ducking { pc.duck_speed } else if input.down("Run") { pc.run_speed } else { pc.walk_speed };
            // Ease the camera towards the standing / ducked eye height.
            let target = if ducking { pc.duck_eye_height } else { pc.stand_eye_height };
            pc.eye_height += (target - pc.eye_height) * 0.3;
            let wish = wish.normalize_or_zero() * speed;
            let jump = pc.jump_speed;
            if let Some(b) = self.bodies.get_mut(h) {
                let v = *b.linvel();
                let grounded = v.z.abs() < 5.0;
                let vz = if input.pressed("Jump") && grounded { jump } else { v.z };
                b.set_linvel(Vector3::new(wish.x, wish.y, vz), true);
            }
        }
    }

    fn update_player_cameras(&self, scene: &mut Scene) {
        for &go in &self.players {
            let Some(pc) = scene.component::<PlayerController>(go) else { continue };
            let Some(cam) = pc.camera else { continue };
            let (eye, yaw, pitch) = (pc.eye_height, pc.yaw, pc.pitch);
            let pos = scene.world_transform(go).position + Vec3::new(0.0, 0.0, eye);
            scene.set_world_transform(cam, Transform::new(pos, Angles::new(pitch, yaw, 0.0).to_rotation(), Vec3::ONE));
        }
    }

    pub fn step(&mut self, scene: &mut Scene) {
        if !self.enabled {
            return;
        }
        self.register_new(scene);
        self.drive_players(scene);
        // Kinematic bodies are placed by the game: carry their object's transform into the simulation.
        for (&go, &h) in &self.body_of {
            if !scene.component::<Rigidbody>(go).is_some_and(|rb| rb.kinematic) {
                continue;
            }
            let w = scene.world_transform(go);
            if let Some(b) = self.bodies.get_mut(h) {
                b.set_next_kinematic_position(iso(&Transform::new(w.position, w.rotation, Vec3::ONE)));
            }
        }
        self.params.dt = scene.fixed_delta.max(1e-4);
        let g = Vector3::new(self.gravity.x, self.gravity.y, self.gravity.z);
        self.pipeline.step(
            &g,
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            None,
            &(),
            &(),
        );
        for (&go, &h) in &self.body_of {
            let Some(b) = self.bodies.get(h) else { continue };
            if !b.is_dynamic() || b.is_sleeping() {
                continue;
            }
            let (t, r) = (b.translation(), b.rotation());
            let mut w = scene.world_transform(go);
            w.position = Vec3::new(t.x, t.y, t.z);
            w.rotation = Quat::from_xyzw(r.i, r.j, r.k, r.w);
            scene.set_world_transform(go, w);
        }
        self.update_player_cameras(scene);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kinematic_body_pushes_a_dynamic_one_and_is_not_pushed_back() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        let wall = scene.create_object("wall");
        scene.set_world_transform(wall, Transform::from_position(Vec3::new(-100.0, 0.0, 25.0)));
        scene.add_component(wall, Rigidbody { kinematic: true, ..Default::default() });
        scene.add_component(wall, Collider { shape: Shape::Box { center: Vec3::ZERO, size: Vec3::new(20.0, 200.0, 50.0) }, is_trigger: false });
        let ball = scene.create_object("ball");
        scene.set_world_transform(ball, Transform::from_position(Vec3::new(0.0, 0.0, 25.0)));
        scene.add_component(ball, Rigidbody { gravity: false, ..Default::default() });
        scene.add_component(ball, Collider { shape: Shape::Sphere { center: Vec3::ZERO, radius: 25.0 }, is_trigger: false });
        // The game drives the wall towards +X at 100 u/s for 2 seconds.
        for _ in 0..100 {
            let mut t = scene.world_transform(wall);
            t.position.x += 2.0;
            scene.set_world_transform(wall, t);
            scene.tick(1.0 / 50.0);
        }
        let (w, b) = (scene.world_transform(wall).position.x, scene.world_transform(ball).position.x);
        assert!((w - 100.0).abs() < 0.5, "the wall goes where the game puts it, got {w}");
        assert!(b > 20.0, "the ball was pushed along, got {b}");
        assert!(b - w > 20.0, "and stays ahead of the wall, wall {w} ball {b}");
    }

    #[test]
    fn compound_hull_and_capsule_rest_on_plane() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        // A 40-unit cube hull (z 0..40) plus a capsule standing on top of it (z 40..80 plus caps).
        let h = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let cube: Vec<Vec3> = (0..8).map(|i| h(if i & 1 == 0 { -20.0 } else { 20.0 }, if i & 2 == 0 { -20.0 } else { 20.0 }, if i & 4 == 0 { 0.0 } else { 40.0 })).collect();
        let b = scene.create_object("prop");
        scene.set_local_transform(b, Transform::from_position(Vec3::new(0.0, 0.0, 100.0)));
        scene.add_component(b, Rigidbody::default());
        scene.add_component(b, Collider { shape: Shape::Compound(vec![Shape::Hull(cube), Shape::Capsule { a: h(0.0, 0.0, 50.0), b: h(0.0, 0.0, 70.0), radius: 5.0 }]), is_trigger: false });
        for _ in 0..400 {
            scene.tick(1.0 / 50.0);
        }
        let z = scene.world_transform(b).position.z;
        assert!(z.abs() < 1.5, "the hull's bottom (object z=0) should rest on the plane, got {z}");
    }

    #[test]
    fn a_ball_rests_on_a_triangle_mesh_floor_and_bad_meshes_are_dropped() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        // A 400x400 floor at z = 0 made of two triangles, facing up.
        let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let floor = scene.create_object("floor");
        scene.add_component(floor, Collider { shape: Shape::TriMesh { vertices: vec![v(-200.0, -200.0), v(200.0, -200.0), v(200.0, 200.0), v(-200.0, 200.0)], indices: vec![[0, 1, 2], [0, 2, 3]] }, is_trigger: false });
        let ball = scene.create_object("ball");
        scene.set_local_transform(ball, Transform::from_position(Vec3::new(50.0, 50.0, 150.0)));
        scene.add_component(ball, Rigidbody::default());
        scene.add_component(ball, Collider { shape: Shape::Sphere { center: Vec3::ZERO, radius: 20.0 }, is_trigger: false });
        // Out-of-range indices must not panic.
        let bad = scene.create_object("bad");
        scene.add_component(bad, Collider { shape: Shape::TriMesh { vertices: vec![Vec3::ZERO], indices: vec![[0, 1, 2]] }, is_trigger: false });
        for _ in 0..300 {
            scene.tick(1.0 / 50.0);
        }
        let z = scene.world_transform(ball).position.z;
        assert!((z - 20.0).abs() < 1.5, "ball should rest on the mesh at z=20, got {z}");
    }

    #[test]
    fn box_falls_and_rests_on_plane() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        let b = scene.create_object("box");
        scene.set_local_transform(b, Transform::from_position(Vec3::new(0.0, 0.0, 200.0)));
        scene.add_component(b, Rigidbody::default());
        scene.add_component(b, Collider { shape: Shape::Box { center: Vec3::ZERO, size: Vec3::splat(50.0) }, is_trigger: false });
        for _ in 0..300 {
            scene.tick(1.0 / 50.0);
        }
        let z = scene.world_transform(b).position.z;
        assert!((z - 25.0).abs() < 1.0, "box should rest at z=25, got {z}");
        assert_eq!(scene.resource::<Physics>().unwrap().body_count(), 1);
    }

    #[test]
    fn player_walks_forward_and_jumps() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        scene.insert_resource(Input::default());
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        let cam = scene.create_object("cam");
        let player = scene.create_object("player");
        scene.add_component(player, PlayerController { camera: Some(cam), ..Default::default() });
        for _ in 0..50 {
            scene.tick(1.0 / 50.0);
        }
        let z0 = scene.world_transform(player).position.z;
        assert!(z0.abs() < 1.0, "player should stand on the ground, z={z0}");
        scene.resource_mut::<Input>().unwrap().set("Forward", true);
        for _ in 0..50 {
            scene.tick(1.0 / 50.0);
        }
        let x = scene.world_transform(player).position.x;
        assert!(x > 100.0, "player should have walked forward ~190u, x={x}");
        scene.resource_mut::<Input>().unwrap().set("Jump", true);
        let mut max_z: f32 = 0.0;
        for _ in 0..40 {
            scene.tick(1.0 / 50.0);
            max_z = max_z.max(scene.world_transform(player).position.z);
        }
        assert!(max_z > 20.0, "player should have jumped, max z={max_z}");
        let c = scene.world_transform(cam).position;
        assert!(c.z > 50.0, "camera follows eye height, z={}", c.z);
    }

    #[test]
    fn ducking_slows_the_player_and_lowers_the_eye() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        scene.insert_resource(Input::default());
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        let cam = scene.create_object("cam");
        let player = scene.create_object("player");
        scene.add_component(player, PlayerController { camera: Some(cam), ..Default::default() });
        for _ in 0..25 {
            scene.tick(1.0 / 50.0);
        }
        {
            let input = scene.resource_mut::<Input>().unwrap();
            input.set("Forward", true);
            input.set("Duck", true);
        }
        for _ in 0..50 {
            scene.tick(1.0 / 50.0);
        }
        let x = scene.world_transform(player).position.x;
        assert!((60.0..120.0).contains(&x), "ducked player should cover ~90u in a second, x={x}");
        let eye = scene.world_transform(cam).position.z;
        assert!(eye < 50.0, "camera should drop towards the duck eye height, z={eye}");
    }

    #[test]
    fn child_collider_attaches_to_parent_body() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        let body = scene.create_object("body");
        scene.set_local_transform(body, Transform::from_position(Vec3::new(0.0, 0.0, 100.0)));
        scene.add_component(body, Rigidbody::default());
        let child = scene.create_object_in(Some(body), "col");
        scene.add_component(child, Collider { shape: Shape::Sphere { center: Vec3::ZERO, radius: 10.0 }, is_trigger: false });
        for _ in 0..200 {
            scene.tick(1.0 / 50.0);
        }
        let z = scene.world_transform(body).position.z;
        assert!((z - 10.0).abs() < 1.0, "sphere body should rest at z=10, got {z}");
    }
}
