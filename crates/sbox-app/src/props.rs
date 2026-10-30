//! Client-side prediction of props: the client simulates them with its own physics (so a prop the local player pushes
//! reacts at once) and pulls each one toward the server's reported position, snapping when the two have diverged.

use std::collections::HashMap;

use sbox_math::Vec3;
use sbox_net::replication::{correct_prop, NetworkId, PropCorrection, PLAYER_ID_BASE};
use sbox_net::EntityState;
use sbox_physics::Physics;
use sbox_scene::Scene;

/// Reconcile every networked prop (ids below the player range) with the latest server state. Returns how many were
/// eased and how many snapped, for logging and tests.
pub fn reconcile(scene: &mut Scene, server: &HashMap<u32, EntityState>, dt: f32) -> (usize, usize) {
    let props: Vec<_> = scene.iter_components::<NetworkId>().filter(|(_, n)| n.0 < PLAYER_ID_BASE).filter_map(|(go, n)| server.get(&n.0).map(|e| (go, Vec3::from(e.position)))).collect();
    let (mut eased, mut snapped) = (0, 0);
    scene.with_resource::<Physics, _>(|physics, scene| {
        for (go, target) in props {
            let Some(local) = physics.body_position(go) else { continue };
            match correct_prop(local, target, dt) {
                PropCorrection::Keep => {}
                PropCorrection::Ease(p) => {
                    physics.set_body_state(scene, go, p, None);
                    eased += 1;
                }
                PropCorrection::Snap(p) => {
                    physics.set_body_state(scene, go, p, Some(Vec3::ZERO));
                    snapped += 1;
                }
            }
        }
    });
    (eased, snapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sbox_physics::{Collider, Rigidbody, Shape};
    use sbox_math::Transform;

    fn world() -> (Scene, sbox_scene::GoId) {
        let mut scene = Scene::new();
        Physics::install(&mut scene);
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        let ball = scene.create_object("ball");
        scene.set_local_transform(ball, Transform::from_position(Vec3::new(0.0, 0.0, 100.0)));
        scene.add_component(ball, Rigidbody::default());
        scene.add_component(ball, Collider { shape: Shape::Sphere { center: Vec3::ZERO, radius: 20.0 }, is_trigger: false });
        scene.add_component(ball, NetworkId(7));
        (scene, ball)
    }

    fn state(x: f32, z: f32) -> HashMap<u32, EntityState> {
        HashMap::from([(7, EntityState { id: 7, position: [x, 0.0, z], rotation: [0.0, 0.0, 0.0, 1.0] })])
    }

    #[test]
    fn a_prop_falls_locally_and_is_pulled_toward_the_server_then_snaps_when_far() {
        let (mut scene, ball) = world();
        // The client predicts the fall on its own; the server says the ball is resting at z = 20 but 10 units to the side.
        let server = state(10.0, 20.0);
        for _ in 0..200 {
            scene.tick(1.0 / 50.0);
            reconcile(&mut scene, &server, 1.0 / 50.0);
        }
        let p = scene.world_transform(ball).position;
        assert!((p - Vec3::new(10.0, 0.0, 20.0)).length() < 1.5, "converged on the server's rest position, got {p:?}");
        // The server teleported it far away (a collision the client never saw): the client jumps there at once.
        let (eased, snapped) = reconcile(&mut scene, &state(500.0, 20.0), 1.0 / 50.0);
        assert_eq!((eased, snapped), (0, 1));
        assert!((scene.world_transform(ball).position.x - 500.0).abs() < 0.01);
        // Objects the server does not know are left alone; players are never touched.
        assert_eq!(reconcile(&mut scene, &HashMap::new(), 0.02), (0, 0));
    }
}
