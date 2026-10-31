//! `func_shatterglass`: a pane that breaks when a fast-moving physics body reaches it. It is replaced by a handful of
//! small dynamic shards that fall. The reference breaks glass on any damage event; here only the speed and position of
//! bodies are checked, since the physics world reports no contacts.

use sbox_math::{Transform, Vec3};
use sbox_physics::{Collider, Physics, Rigidbody, Shape};
use sbox_scene::{impl_component_any, Component, GoId, Handle, MaterialId, MeshId, ModelRenderer, Scene};

pub struct Shatterable {
    /// Half size of the pane's bounds (world axes) around its object's position.
    pub half: Vec3,
    /// A body must be this fast (units per second) to break it.
    pub min_speed: f32,
    pub shard_mesh: MeshId,
    pub shard_material: MaterialId,
    /// Edge of one shard.
    pub shard_size: f32,
}

/// Whether a body at `body` moving at `velocity` breaks a pane centred at `pane` with half size `half`.
pub fn breaks(pane: Vec3, half: Vec3, body: Vec3, velocity: Vec3, min_speed: f32) -> bool {
    let d = (body - pane).abs();
    // The body counts as touching when within a margin around the pane.
    velocity.length() >= min_speed && d.x <= half.x + 16.0 && d.y <= half.y + 16.0 && d.z <= half.z + 16.0
}

/// Shard centres: a grid over the pane's two larger axes (the thin axis stays at the centre).
pub fn shard_positions(centre: Vec3, half: Vec3, per_side: usize) -> Vec<Vec3> {
    let thin = if half.x <= half.y && half.x <= half.z { 0 } else if half.y <= half.z { 1 } else { 2 };
    let mut out = Vec::new();
    for i in 0..per_side {
        for j in 0..per_side {
            let f = |k: usize| (k as f32 + 0.5) / per_side as f32 * 2.0 - 1.0;
            let axes: Vec<usize> = (0..3).filter(|&a| a != thin).collect();
            let mut o = Vec3::ZERO;
            o[axes[0]] = half[axes[0]] * f(i);
            o[axes[1]] = half[axes[1]] * f(j);
            out.push(centre + o);
        }
    }
    out
}

impl Component for Shatterable {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let pane = scene.world_transform(h.go).position;
        let hit = scene.resource::<Physics>().is_some_and(|p| {
            scene.iter_components::<Rigidbody>().any(|(go, rb)| {
                go != h.go && rb.motion_enabled && !rb.kinematic && match (p.body_position(go), p.body_velocity(go)) {
                    (Some(pos), Some(v)) => breaks(pane, self.half, pos, v, self.min_speed),
                    _ => false,
                }
            })
        });
        if !hit {
            return;
        }
        let (mesh, material, size) = (self.shard_mesh, self.shard_material, self.shard_size);
        for c in shard_positions(pane, self.half, 4) {
            let shard: GoId = scene.create_object("shard");
            scene.set_local_transform(shard, Transform::from_position(c));
            let mut m = ModelRenderer::new(mesh);
            m.material = material;
            scene.add_component(shard, m);
            scene.add_component(shard, Collider { shape: Shape::Box { center: Vec3::ZERO, size: Vec3::splat(size) }, is_trigger: false });
            scene.add_component(shard, Rigidbody::default());
        }
        scene.destroy_object(h.go);
    }
    impl_component_any!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_fast_nearby_bodies_break_the_pane() {
        let (pane, half) = (Vec3::ZERO, Vec3::new(2.0, 40.0, 30.0));
        assert!(breaks(pane, half, Vec3::new(10.0, 0.0, 0.0), Vec3::new(300.0, 0.0, 0.0), 150.0));
        assert!(!breaks(pane, half, Vec3::new(10.0, 0.0, 0.0), Vec3::new(50.0, 0.0, 0.0), 150.0), "too slow");
        assert!(!breaks(pane, half, Vec3::new(10.0, 200.0, 0.0), Vec3::new(300.0, 0.0, 0.0), 150.0), "too far");
    }

    #[test]
    fn shards_cover_the_two_large_axes_and_stay_thin() {
        let s = shard_positions(Vec3::new(5.0, 0.0, 0.0), Vec3::new(2.0, 40.0, 30.0), 4);
        assert_eq!(s.len(), 16);
        assert!(s.iter().all(|p| p.x == 5.0), "the thin axis stays at the pane");
        let (ymin, ymax) = s.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
        assert!(ymin < -25.0 && ymax > 25.0, "spread across Y: {ymin}..{ymax}");
    }

    #[test]
    fn a_thrown_ball_shatters_the_pane_into_shards() {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        scene.resource_mut::<Physics>().unwrap().gravity = Vec3::ZERO;
        let pane = scene.create_object("pane");
        scene.add_component(pane, Shatterable { half: Vec3::new(2.0, 40.0, 30.0), min_speed: 150.0, shard_mesh: MeshId(0), shard_material: MaterialId(0), shard_size: 8.0 });
        let ball = scene.create_object("ball");
        scene.set_local_transform(ball, Transform::from_position(Vec3::new(-200.0, 0.0, 0.0)));
        scene.add_component(ball, Rigidbody::default());
        scene.add_component(ball, Collider { shape: Shape::Sphere { center: Vec3::ZERO, radius: 10.0 }, is_trigger: false });
        for _ in 0..3 {
            scene.tick(1.0 / 50.0);
        }
        assert_eq!(scene.iter_components::<Shatterable>().count(), 1, "a resting ball breaks nothing");
        scene.with_resource::<Physics, _>(|p, s| p.set_body_state(s, ball, Vec3::new(-200.0, 0.0, 0.0), Some(Vec3::new(400.0, 0.0, 0.0))));
        for _ in 0..80 {
            scene.tick(1.0 / 50.0);
        }
        assert_eq!(scene.iter_components::<Shatterable>().count(), 0, "the pane is gone");
        assert_eq!(scene.iter_components::<ModelRenderer>().count(), 16, "replaced by shards");
    }
}
