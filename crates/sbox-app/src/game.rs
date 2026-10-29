//! Rust ports of game-side C# components (the scripting strategy: game code is reimplemented against the Rust scene API).

use sbox_math::{Angles, RotationExt, Vec3};
use sbox_scene::{impl_component_any, CameraComponent, Component, Handle, Input, Scene};

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
    fn sound_point_without_audio_device_is_harmless() {
        let ev = sbox_resource::sound_event::SoundEvent { sounds: vec![], volume: 1.0, pitch: 1.0, min_distance: 10.0, max_distance: 100.0, ui: false };
        let mut scene = Scene::new();
        let go = scene.create_object("s");
        scene.add_component(go, SoundPoint::new(ev, Vec::new(), false, 1.0));
        for _ in 0..3 {
            scene.tick(1.0 / 60.0);
        }
    }

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

/// Drives a skinned model from a `.vanmgrph_c` graph: evaluates it every frame, samples the referenced
/// clips (`clips/<sequence>.anim_c`) and blends them into the bone matrices.
pub struct GraphAnimator {
    pub graph: sbox_resource::animgraph::Graph,
    pub instance: sbox_resource::animgraph::Instance,
    pub fs: sbox_resource::vmat::GameFs,
    pub skeleton: sbox_resource::model::Skeleton,
    pub anim_skel: Option<sbox_resource::model::AnimSkeleton>,
    pub parts: Vec<sbox_scene::GoId>,
    clips: std::collections::HashMap<String, Option<sbox_resource::anim::Animation>>,
}

impl GraphAnimator {
    pub fn new(
        graph: sbox_resource::animgraph::Graph,
        fs: sbox_resource::vmat::GameFs,
        skeleton: sbox_resource::model::Skeleton,
        parts: Vec<sbox_scene::GoId>,
    ) -> Self {
        let instance = sbox_resource::animgraph::Instance::new(&graph);
        Self { graph, instance, fs, skeleton, anim_skel: None, parts, clips: Default::default() }
    }

    fn clip(&mut self, name: &str) -> Option<&sbox_resource::anim::Animation> {
        if !self.clips.contains_key(name) {
            let lower = name.to_lowercase();
            let loaded = ["clips", "clips_human"]
                .iter()
                .find_map(|dir| self.fs.read(&format!("{dir}/{lower}.anim")))
                .and_then(|d| sbox_resource::anim::parse(&d).map_err(|e| log::warn!("clip {name}: {e:#}")).ok());
            if loaded.is_none() {
                log::warn!("clip not found: {name}");
            }
            if self.anim_skel.is_none() {
                if let Some(a) = &loaded {
                    self.anim_skel = self.fs.read(&a.skeleton).and_then(|d| sbox_resource::model::load_skel(&d).ok());
                }
            }
            self.clips.insert(name.to_string(), loaded);
        }
        self.clips[name].as_ref()
    }
}

impl Component for GraphAnimator {
    fn on_update(&mut self, scene: &mut Scene, _h: Handle) {
        let mut lens: std::collections::HashMap<String, f32> = Default::default();
        let samples = {
            let mut inst = std::mem::replace(&mut self.instance, sbox_resource::animgraph::Instance::new(&self.graph));
            let graph = std::mem::take(&mut self.graph);
            let mut len = |n: &str| -> f32 {
                *lens.entry(n.to_string()).or_insert_with(|| self.clip(n).map_or(1.0, |a| a.duration))
            };
            let s = inst.evaluate(&graph, scene.delta, &mut len);
            self.graph = graph;
            self.instance = inst;
            s
        };
        let mut poses = Vec::new();
        for s in &samples {
            if let Some(a) = self.clip(&s.sequence) {
                poses.push((a.sample(s.time), s.weight));
            }
        }
        let Some(skel) = self.anim_skel.as_ref() else { return };
        if poses.is_empty() {
            return;
        }
        let blended = sbox_resource::animgraph::blend_poses(&poses);
        let mats = self.skeleton.skin_matrices(skel, &blended);
        for &go in &self.parts {
            if let Some(m) = scene.component_mut::<sbox_scene::ModelRenderer>(go) {
                m.bones = mats.clone();
            }
        }
    }
    impl_component_any!();
}

/// `Sandbox.SoundPointComponent`: plays a sound event at the object's position, attenuated by distance to the camera.
pub struct SoundPoint {
    pub event: sbox_resource::sound_event::SoundEvent,
    pub sound_file: Vec<u8>,
    pub looping: bool,
    pub volume: f32,
    sink: Option<rodio::Sink>,
}

impl SoundPoint {
    pub fn new(event: sbox_resource::sound_event::SoundEvent, sound_file: Vec<u8>, looping: bool, volume: f32) -> Self {
        Self { event, sound_file, looping, volume, sink: None }
    }
}

impl Component for SoundPoint {
    fn on_start(&mut self, scene: &mut Scene, _h: Handle) {
        if let Some(h) = scene.resource::<crate::audio::AudioHandle>() {
            self.sink = crate::audio::start(&h.0, &self.sound_file, self.looping);
        }
    }
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let Some(sink) = &self.sink else { return };
        let pos = scene.world_transform(h.go).position;
        let listener = scene.iter_components::<CameraComponent>().next().map(|(go, _)| scene.world_transform(go).position);
        let gain = listener.map_or(1.0, |l| self.event.attenuation(l.distance(pos)));
        sink.set_volume(self.volume * self.event.volume * gain);
    }
    impl_component_any!();
}

/// `TemporaryEffect`: destroys its object after `DestroyAfterSeconds` (child effects are not waited for yet).
pub struct TemporaryEffect {
    pub destroy_after: f32,
    age: f32,
}

impl TemporaryEffect {
    pub fn new(destroy_after: f32) -> Self {
        Self { destroy_after, age: 0.0 }
    }
}

impl Component for TemporaryEffect {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        self.age += scene.delta;
        if self.age >= self.destroy_after {
            scene.destroy_object(h.go);
        }
    }
    impl_component_any!();
}

#[cfg(test)]
mod effect_tests {
    use super::*;

    #[test]
    fn temporary_effect_destroys_its_object() {
        let mut scene = Scene::new();
        let go = scene.create_object("boom");
        scene.add_component(go, TemporaryEffect::new(0.5));
        for _ in 0..20 {
            scene.tick(1.0 / 60.0);
        }
        assert!(scene.exists(go), "still alive at 0.33s");
        for _ in 0..30 {
            scene.tick(1.0 / 60.0);
        }
        assert!(!scene.exists(go), "destroyed after 0.5s");
    }
}
