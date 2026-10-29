use std::sync::Arc;
use std::time::Instant;

use sbox_math::{Angles, Quat, RotationExt, Transform, Vec3};
use sbox_render::{MeshData, Renderer};
use sbox_scene::{impl_component_any, CameraComponent, Component, DirectionalLight, Handle, ModelRenderer, PointLight, Scene};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
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

/// Simple fly camera: WASD + QE, arrow keys to look.
#[derive(Default)]
struct FlyCamera {
    keys: [bool; 10], // W A S D Q E Left Right Up Down
    angles: Angles,
}

impl Component for FlyCamera {
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
    fn build_scene(scene: &mut Scene, r: &mut Renderer, model: Option<&str>) -> sbox_scene::GoId {
        if let Some(path) = model {
            return Self::build_model_scene(scene, r, path);
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
    /// Show a single compiled model (`.vmdl_c`) with a camera framed on its bounds.
    fn build_model_scene(scene: &mut Scene, r: &mut Renderer, path: &str) -> sbox_scene::GoId {
        let bytes = std::fs::read(path).expect("read model");
        let rm = sbox_resource::mesh::load_embedded(&bytes).expect("load model");
        log::info!("{path}: {} draw calls, bounds {:?}..{:?}", rm.draw_calls.len(), rm.mins, rm.maxs);
        let mesh = r.upload_mesh(&MeshData::from_render_mesh(&rm));
        let center = (Vec3::from(rm.mins) + Vec3::from(rm.maxs)) * 0.5;
        let radius = ((Vec3::from(rm.maxs) - Vec3::from(rm.mins)).length() * 0.5).max(1.0);

        let go = scene.create_object("model");
        scene.set_local_transform(go, Transform::from_position(-center));
        let mut m = ModelRenderer::new(mesh);
        m.tint = [0.75, 0.75, 0.78, 1.0];
        scene.add_component(go, m);

        let sun = scene.create_object("sun");
        scene.set_local_transform(sun, Transform::new(Vec3::ZERO, Angles::new(45.0, 35.0, 0.0).to_rotation(), Vec3::ONE));
        scene.add_component(sun, DirectionalLight::default());

        let cam = scene.create_object("camera");
        scene.set_local_transform(cam, Transform::from_position(Vec3::new(-radius * 2.5, 0.0, 0.0)));
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
        let model = args.iter().find(|a| !a.starts_with("--") && a.ends_with("_c")).cloned();
        self.camera = Some(Self::build_scene(&mut self.scene, &mut renderer, model.as_deref()));
        if let Some(i) = args.iter().position(|a| a == "--screenshot") {
            let out = args.get(i + 1).expect("--screenshot <file.png>");
            let secs: f32 = args.iter().position(|a| a == "--time").and_then(|j| args.get(j + 1)).and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let (w, h) = (1280, 720);
            for _ in 0..(secs * 60.0) as u32 {
                self.scene.tick(1.0 / 60.0);
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
