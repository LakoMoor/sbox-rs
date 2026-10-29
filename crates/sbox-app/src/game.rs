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
    weight_lists: std::collections::HashMap<String, sbox_resource::weightlist::WeightList>,
    clip_index: std::collections::HashMap<String, String>,
    /// Skeletons of loaded clips by virtual path, and the path of the one all poses are expressed in.
    skels: std::collections::HashMap<String, sbox_resource::model::AnimSkeleton>,
    main_skel: Option<String>,
}

impl GraphAnimator {
    pub fn new(
        graph: sbox_resource::animgraph::Graph,
        fs: sbox_resource::vmat::GameFs,
        skeleton: sbox_resource::model::Skeleton,
        parts: Vec<sbox_scene::GoId>,
    ) -> Self {
        let instance = sbox_resource::animgraph::Instance::new(&graph);
        // Weight lists live in the model source next to the graph; only the citizen ships one.
        let weight_lists = fs
            .read_raw("models/citizen/prefabs/citizen_weightlistlist.vmdl_prefab")
            .and_then(|b| String::from_utf8(b).ok())
            .map(|t| sbox_resource::weightlist::parse(&t))
            .unwrap_or_default();
        let clip_index = fs.index("anim");
let mut me = Self { graph, instance, fs, skeleton, anim_skel: None, parts, clips: Default::default(), weight_lists, clip_index, skels: Default::default(), main_skel: None };
        me.clip("IdlePose_Default"); // human.skel (with eye bones) is a superset of citizen_human.skel
        me
    }

    fn clip(&mut self, name: &str) -> Option<&sbox_resource::anim::Animation> {
        if !self.clips.contains_key(name) {
            let lower = name.to_lowercase();
            let loaded = self
                .clip_index
                .get(&lower)
                .and_then(|path| self.fs.read(path))
                .and_then(|d| sbox_resource::anim::parse(&d).map_err(|e| log::warn!("clip {name}: {e:#}")).ok());
            if loaded.is_none() {
                log::warn!("clip not found: {name}");
            }
            if let Some(a) = &loaded {
                if !self.skels.contains_key(&a.skeleton) {
                    if let Some(sk) = self.fs.read(&a.skeleton).and_then(|d| sbox_resource::model::load_skel(&d).ok()) {
                        // The first skeleton seen becomes the main one (`new` preloads a locomotion clip so this is deterministic).
                        if self.main_skel.is_none() {
                            self.main_skel = Some(a.skeleton.clone());
                            self.anim_skel = Some(sk.clone());
                        }
                        self.skels.insert(a.skeleton.clone(), sk);
                    }
                }
            }
            self.clips.insert(name.to_string(), loaded);
        }
        self.clips[name].as_ref()
    }

    /// Sample a clip and express the pose in the main skeleton's bone order.
    fn sample_clip(&mut self, name: &str, t: f32) -> Option<(Vec<sbox_resource::anim::BonePose>, bool)> {
        let (pose, additive, skel_path) = {
            let a = self.clip(name)?;
            (a.sample(t), a.additive, a.skeleton.clone())
        };
        if self.main_skel.as_deref() == Some(skel_path.as_str()) || self.main_skel.is_none() {
            return Some((pose, additive));
        }
        // `citizen_human.skel` lacks the two eye bones of `human.skel`: remap by bone name, missing bones keep the bind pose.
        let (main, own) = (self.anim_skel.as_ref()?, self.skels.get(&skel_path)?);
        let own_idx: std::collections::HashMap<&str, usize> = own.names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let model_idx: std::collections::HashMap<&str, usize> = self.skeleton.names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let remapped = main
            .names
            .iter()
            .map(|n| match own_idx.get(n.as_str()).and_then(|&i| pose.get(i)) {
                Some(p) => *p,
                None => {
                    let b = model_idx.get(n.as_str());
                    sbox_resource::anim::BonePose {
                        position: b.map_or([0.0; 3], |&i| self.skeleton.bind_pos[i]),
                        rotation: b.map_or([0.0, 0.0, 0.0, 1.0], |&i| self.skeleton.bind_rot[i]),
                        scale: 1.0,
                    }
                }
            })
            .collect();
        Some((remapped, additive))
    }
}

impl Component for GraphAnimator {
    fn on_update(&mut self, scene: &mut Scene, _h: Handle) {
        let pose = {
            let mut inst = std::mem::replace(&mut self.instance, sbox_resource::animgraph::Instance::new(&self.graph));
            let graph = std::mem::take(&mut self.graph);
            let this = std::cell::RefCell::new(&mut *self);
            let mut sample = |n: &str, t: f32| this.borrow_mut().sample_clip(n, t);
            let mut len = |n: &str| this.borrow_mut().clip(n).map_or(1.0, |a| a.duration);
            let mut weights = |name: &str| {
                let me = this.borrow();
                let skel = me.anim_skel.as_ref()?;
                me.weight_lists.get(name).map(|wl| wl.per_bone(&skel.names))
            };
            let pose = inst.evaluate_pose(&graph, scene.delta, &mut sbox_resource::animgraph::Clips { sample: &mut sample, len: &mut len, weights: &mut weights });
            drop(this);
            self.graph = graph;
            self.instance = inst;
            pose
        };
        let Some(skel) = self.anim_skel.as_ref() else { return };
        let Some(pose) = pose else { return };
        let blended = pose.bones;
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
