//! Rust ports of game-side C# components (the scripting strategy: game code is reimplemented against the Rust scene API).

use sbox_math::{Angles, RotationExt, Vec3};
use sbox_scene::{impl_component_any, Component, Handle, Input, Scene};

/// Port of `samples/sweeper/code/CubeController.cs`.
pub struct CubeController {
    pub move_speed: f32,
}

impl Default for CubeController {
    fn default() -> Self {
        Self { move_speed: 100.0 }
    }
}

impl Component for CubeController {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let (look_x, move_dir) = match scene.resource_mut::<Input>() {
            Some(i) => {
                let (dx, _) = if i.mouse_captured { i.take_mouse() } else { (0.0, 0.0) };
                (-dx * 0.1, Vec3::new(i.axis("Forward", "Backward"), i.axis("Left", "Right"), 0.0))
            }
            None => return,
        };
        let mut t = scene.world_transform(h.go);
        let mut a = t.rotation.angles();
        a.yaw += look_x;
        a.pitch = 0.0;
        a.roll = 0.0;
        t.rotation = Angles::new(a.pitch, a.yaw, a.roll).to_rotation();
        t.position += t.rotation * (move_dir * self.move_speed * scene.delta);
        scene.set_world_transform(h.go, t);
    }
    impl_component_any!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_controller_moves_and_yaws() {
        let mut scene = Scene::new();
        scene.insert_resource(Input::default());
        let go = scene.create_object("cube");
        scene.add_component(go, CubeController::default());
        {
            let i = scene.resource_mut::<Input>().unwrap();
            i.set("Forward", true);
            i.mouse_captured = true;
            i.add_mouse(-900.0, 0.0); // -dx * 0.1 = +90 deg yaw
        }
        scene.tick(1.0 / 60.0); // first tick: on_start
        for _ in 0..60 {
            scene.tick(1.0 / 60.0);
        }
        let t = scene.world_transform(go);
        let yaw = t.rotation.angles().yaw;
        assert!((yaw - 90.0).abs() < 1.0, "yaw={yaw}");
        // after the 90 degree turn, forward is +Y: ~100 units in ~1s
        assert!(t.position.y > 80.0 && t.position.x.abs() < 5.0, "pos={:?}", t.position);
    }
}
