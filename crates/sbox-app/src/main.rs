mod assets;
mod game;

use std::sync::Arc;
use std::time::Instant;

use sbox_math::{Angles, Quat, RotationExt, Transform, Vec3};
use assets::{Assets, SceneLoader};
use sbox_render::{MeshData, Renderer};
use sbox_scene::{impl_component_any, CameraComponent, Component, DirectionalLight, Handle, Input, ModelRenderer, PointLight, Scene};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// Spins its object around Z (degrees per second).
struct Spinner {
    speed: f32,
}

impl Component for Spinner {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let mut t = scene.local_transform(h.go);
        t.rotation = Quat::from_rotation_z((self.speed * scene.delta).to_radians()) * t.rotation;
        scene.set_local_transform(h.go, t);
    }
    impl_component_any!();
}

/// Plays an animation on a skinned model: recomputes skinning matrices every frame.
struct Animator {
    anim: sbox_resource::anim::Animation,
    skeleton: sbox_resource::model::Skeleton,
    anim_skel: sbox_resource::model::AnimSkeleton,
    parts: Vec<sbox_scene::GoId>,
    time: f32,
}

impl Component for Animator {
    fn on_update(&mut self, scene: &mut Scene, _h: Handle) {
        self.time += scene.delta;
        let poses = self.anim.sample(self.time);
        let mats = self.skeleton.skin_matrices(&self.anim_skel, &poses);
        for &go in &self.parts {
            if let Some(m) = scene.component_mut::<ModelRenderer>(go) {
                m.bones = mats.clone();
            }
        }
    }
    impl_component_any!();
}

/// Simple fly camera: WASD + QE, arrow keys to look.
#[derive(Default)]
struct FlyCamera {
    keys: [bool; 10], // W A S D Q E Left Right Up Down
    angles: Angles,
}

impl Component for FlyCamera {
    fn on_start(&mut self, scene: &mut Scene, h: Handle) {
        self.angles = scene.world_transform(h.go).rotation.angles();
        self.angles.roll = 0.0;
    }
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let k = self.keys;
        let b = |i: usize| k[i] as i32 as f32;
        let dt = scene.delta;
        self.angles.yaw += (b(6) - b(7)) * 90.0 * dt;
        self.angles.pitch = (self.angles.pitch + (b(9) - b(8)) * 90.0 * dt).clamp(-89.0, 89.0);
        let rot = self.angles.to_rotation();
        let mut t = scene.local_transform(h.go);
        t.rotation = rot;
        let wish = rot.forward() * (b(0) - b(2)) + rot.left() * (b(1) - b(3)) + Vec3::Z * (b(5) - b(4));
        t.position += wish * 300.0 * dt;
        scene.set_local_transform(h.go, t);
    }
    impl_component_any!();
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    scene: Scene,
    camera: Option<sbox_scene::GoId>,
    last: Instant,
}

impl App {
    fn build_scene(scene: &mut Scene, r: &mut Renderer, model: Option<&str>, anim: Option<&str>, scene_file: Option<&str>) -> sbox_scene::GoId {
        if let Some(path) = scene_file {
            return Self::build_file_scene(scene, r, path);
        }
        if let Some(path) = model {
            return Self::build_model_scene(scene, r, path, anim);
        }
        let cube = r.upload_mesh(&MeshData::cuboid(Vec3::splat(64.0)));
        let ground = r.upload_mesh(&MeshData::plane(2000.0));
        let ball = r.upload_mesh(&MeshData::sphere(32.0, 32, 16));

        let g = scene.create_object("ground");
        let mut m = ModelRenderer::new(ground);
        m.tint = [0.35, 0.4, 0.35, 1.0];
        scene.add_component(g, m);

        for i in 0..5 {
            let go = scene.create_object(&format!("cube{i}"));
            scene.set_local_transform(go, Transform::from_position(Vec3::new(300.0, (i as f32 - 2.0) * 120.0, 40.0)));
            let mut m = ModelRenderer::new(if i % 2 == 0 { cube } else { ball });
            m.tint = [0.3 + 0.15 * i as f32, 0.8 - 0.12 * i as f32, 0.5, 1.0];
            scene.add_component(go, m);
            scene.add_component(go, Spinner { speed: 30.0 + 20.0 * i as f32 });
        }

        let sun = scene.create_object("sun");
        scene.set_local_transform(sun, Transform::new(Vec3::ZERO, Angles::new(50.0, 30.0, 0.0).to_rotation(), Vec3::ONE));
        scene.add_component(sun, DirectionalLight::default());

        let lamp = scene.create_object("lamp");
        scene.set_local_transform(lamp, Transform::from_position(Vec3::new(250.0, 0.0, 150.0)));
        scene.add_component(lamp, PointLight { color: [1.0, 0.5, 0.2], radius: 500.0, intensity: 1.5 });

        let cam = scene.create_object("camera");
        scene.set_local_transform(cam, Transform::from_position(Vec3::new(-150.0, 0.0, 120.0)));
        scene.add_component(cam, CameraComponent::default());
        scene.add_component(cam, FlyCamera::default());
        cam
    }
}

impl App {
    /// Load a `.scene_c` (JSON) file into the scene.
    fn build_file_scene(scene: &mut Scene, r: &mut Renderer, path: &str) -> sbox_scene::GoId {
        use sbox_resource::{vmat, Resource};
        let bytes = std::fs::read(path).expect("read scene");
        let res = Resource::parse(&bytes).expect("scene container");
        let json = std::str::from_utf8(res.block_data("DATA").expect("scene DATA")).expect("scene utf8");
        let game_dir = std::path::Path::new(path).ancestors().find(|p| p.file_name().map_or(false, |n| n == "game"));
        let mut assets = Assets::new(game_dir.map(vmat::GameFs::from_game_dir));
        sbox_physics::Physics::install(scene);
        let mut loader = SceneLoader { r, assets: &mut assets, camera: None };
        let (roots, stats) = sbox_scene::load_scene_json(scene, json, &mut loader).expect("load scene");
        let camera = loader.camera;
        log::info!("scene: {} roots, {} objects, {} components", roots.len(), stats.objects, stats.components);
        if !stats.unknown_components.is_empty() {
            let mut v: Vec<_> = stats.unknown_components.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1));
            log::info!("unsupported components: {:?}", &v[..v.len().min(12)]);
        }
        let cam = camera.unwrap_or_else(|| {
            let c = scene.create_object("camera");
            scene.set_local_transform(c, Transform::from_position(Vec3::new(-400.0, 0.0, 200.0)));
            scene.add_component(c, CameraComponent::default());
            c
        });
        scene.insert_resource(Input::default());
        if std::env::args().any(|a| a == "--play") {
            let eye = sbox_physics::PlayerController::default().eye_height;
            let t = scene.world_transform(cam);
            let player = scene.create_object("player");
            scene.set_local_transform(player, Transform::from_position(t.position - Vec3::Z * eye));
            let a = t.rotation.angles();
            scene.add_component(
                player,
                sbox_physics::PlayerController { camera: Some(cam), yaw: a.yaw, pitch: a.pitch, ..Default::default() },
            );
        } else {
            scene.add_component(cam, FlyCamera::default());
        }
        cam
    }

    /// Show a single compiled model (`.vmdl_c`) with a camera framed on its bounds.
    fn build_model_scene(scene: &mut Scene, r: &mut Renderer, path: &str, anim_path: Option<&str>) -> sbox_scene::GoId {
        use sbox_resource::{anim, model, vmat};
        let bytes = std::fs::read(path).expect("read model");
        let md = model::load(&bytes).expect("load model");
        let meshes: Vec<_> = md.default_meshes().collect();
        log::info!("{path}: {} meshes ({} shown), {} bones", md.meshes.len(), meshes.len(), md.skeleton.len());

        let game_dir = std::path::Path::new(path).ancestors().find(|p| p.file_name().map_or(false, |n| n == "game"));
        let fs = game_dir.map(vmat::GameFs::from_game_dir);
        let mut assets = Assets::new(fs);
        let loaded = assets.load_model_bytes(r, path, &bytes).expect("model has no drawable meshes");
        let _ = &md;
        let center = (loaded.mins + loaded.maxs) * 0.5;
        let radius = ((loaded.maxs - loaded.mins).length() * 0.5).max(1.0);

        let root = scene.create_object("model");
        scene.set_local_transform(root, Transform::from_position(-center));
        let parts = assets.spawn(scene, root, &loaded, [1.0; 4], None);
        let fs = assets.fs.take();

        let args: Vec<String> = std::env::args().collect();
        if let (Some(gp), Some(fs)) = (args.iter().position(|a| a == "--graph").and_then(|i| args.get(i + 1)), fs.as_ref()) {
            let g = sbox_resource::animgraph::Graph::parse(&std::fs::read(gp).expect("read graph")).expect("parse graph");
            log::info!("{gp}: {} nodes, {} parameters", g.nodes.len(), g.params.len());
            let mut ga = game::GraphAnimator::new(g, vmat::GameFs::from_game_dir(game_dir.expect("game dir")), md.skeleton.clone(), parts.clone());
            for (i, a) in args.iter().enumerate() {
                if a == "--param" {
                    if let Some((k, v)) = args.get(i + 1).and_then(|p| p.split_once('=')) {
                        ga.instance.set(&ga.graph, k, v.parse().unwrap_or(0.0));
                    }
                }
            }
            scene.add_component(root, ga);
        }
        if let Some(ap) = anim_path {
            let a = anim::parse(&std::fs::read(ap).expect("read anim")).expect("parse anim");
            log::info!("{ap}: {} frames, {:.2}s, {} tracks", a.frames, a.duration, a.tracks.len());
            let anim_skel = fs
                .as_ref()
                .and_then(|fs| fs.read(&a.skeleton))
                .and_then(|d| model::load_skel(&d).map_err(|e| log::warn!("skeleton: {e:#}")).ok())
                .unwrap_or_else(|| {
                    log::warn!("animation skeleton {} not found; using model bone order", a.skeleton);
                    model::AnimSkeleton { names: md.skeleton.names.clone(), parents: md.skeleton.parents.clone() }
                });
            scene.add_component(root, Animator { anim: a, skeleton: md.skeleton.clone(), anim_skel, parts, time: 0.0 });
        }

        let sun = scene.create_object("sun");
        scene.set_local_transform(sun, Transform::new(Vec3::ZERO, Angles::new(45.0, 35.0, 0.0).to_rotation(), Vec3::ONE));
        scene.add_component(sun, DirectionalLight::default());

        let cam = scene.create_object("camera");
        scene.set_local_transform(cam, Transform::from_position(Vec3::new(-radius * 3.0, 0.0, 0.0)));
        scene.add_component(cam, CameraComponent { z_near: (radius * 0.01).max(0.1), z_far: radius * 100.0, ..Default::default() });
        scene.add_component(cam, FlyCamera::default());
        cam
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            el.create_window(Window::default_attributes().with_title("sbox-rs").with_inner_size(winit::dpi::LogicalSize::new(1280, 720)))
                .expect("window"),
        );
        let size = window.inner_size();
        let mut renderer = Renderer::new(window.clone(), (size.width, size.height)).expect("renderer");
        let args: Vec<String> = std::env::args().skip(1).collect();
        let model = args.iter().find(|a| !a.starts_with("--") && a.ends_with(".vmdl_c")).cloned();
        self.camera = Some(Self::build_scene(&mut self.scene, &mut renderer, model.as_deref(), args.iter().position(|a| a == "--anim").and_then(|i| args.get(i + 1)).map(String::as_str), args.iter().position(|a| a == "--scene").and_then(|i| args.get(i + 1)).map(String::as_str)));
        if let Some(i) = args.iter().position(|a| a == "--screenshot") {
            let out = args.get(i + 1).expect("--screenshot <file.png>");
            let secs: f32 = args.iter().position(|a| a == "--time").and_then(|j| args.get(j + 1)).and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let (w, h) = (1280, 720);
            for _ in 0..(secs * 60.0) as u32 {
                self.scene.tick(1.0 / 60.0);
            }
            if let (Some(c), Some(pitch)) = (self.camera, args.iter().position(|a| a == "--cam-pitch").and_then(|j| args.get(j + 1)).and_then(|v| v.parse::<f32>().ok())) {
                let mut t = self.scene.world_transform(c);
                let a = t.rotation.angles();
                t.rotation = Angles::new(pitch, a.yaw, 0.0).to_rotation();
                self.scene.set_world_transform(c, t);
            }
            if let Some(c) = self.camera {
                let t = self.scene.world_transform(c);
                log::info!("camera at {:?} angles {:?}", t.position, t.rotation.angles());
            }
            let rgba = renderer.render_offscreen(&self.scene, w, h).expect("offscreen render");
            let file = std::io::BufWriter::new(std::fs::File::create(out).expect("create png"));
            let mut enc = png::Encoder::new(file, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&rgba).unwrap();
            log::info!("wrote {out}");
            el.exit();
            return;
        }
        self.renderer = Some(renderer);
        self.window = Some(window);
        self.last = Instant::now();
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if let Some(input) = self.scene.resource_mut::<Input>() {
                input.add_mouse(delta.0 as f32, delta.1 as f32);
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(s.width, s.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape {
                        el.exit();
                    }
                    let action = match code {
                        KeyCode::KeyW => Some("Forward"),
                        KeyCode::KeyS => Some("Backward"),
                        KeyCode::KeyA => Some("Left"),
                        KeyCode::KeyD => Some("Right"),
                        KeyCode::Space => Some("Jump"),
                        KeyCode::ShiftLeft => Some("Run"),
                        _ => None,
                    };
                    if let (Some(a), Some(input)) = (action, self.scene.resource_mut::<Input>()) {
                        input.set(a, down);
                    }
                    let idx = match code {
                        KeyCode::KeyW => 0,
                        KeyCode::KeyA => 1,
                        KeyCode::KeyS => 2,
                        KeyCode::KeyD => 3,
                        KeyCode::KeyQ => 4,
                        KeyCode::KeyE => 5,
                        KeyCode::ArrowLeft => 6,
                        KeyCode::ArrowRight => 7,
                        KeyCode::ArrowUp => 8,
                        KeyCode::ArrowDown => 9,
                        _ => return,
                    };
                    if let Some(cam) = self.camera {
                        if let Some(f) = self.scene.component_mut::<FlyCamera>(cam) {
                            f.keys[idx] = down;
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let (Some(w), Some(input)) = (&self.window, self.scene.resource_mut::<Input>()) {
                    if std::env::args().any(|a| a == "--play") {
                        let ok = w.set_cursor_grab(winit::window::CursorGrabMode::Locked).is_ok()
                            || w.set_cursor_grab(winit::window::CursorGrabMode::Confined).is_ok();
                        if ok {
                            w.set_cursor_visible(false);
                            input.mouse_captured = true;
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                self.scene.tick(dt);
                if let Some(r) = &mut self.renderer {
                    if let Err(e) = r.render(&self.scene) {
                        log::error!("render: {e:#}");
                        el.exit();
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let el = EventLoop::new()?;
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App { window: None, renderer: None, scene: Scene::new(), camera: None, last: Instant::now() };
    el.run_app(&mut app)?;
    Ok(())
}
