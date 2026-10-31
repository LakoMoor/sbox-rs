mod assets;
mod csharp;
mod audio;
mod families;
mod game;
mod layout;
mod meshcomp;
mod page;
mod props;
mod shatter;
mod worldpanel;
#[allow(dead_code)]
mod razor;
mod razor_ui;
mod style;
mod particles;
mod text;
mod ui;

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

/// Skins a model from ragdoll bodies: every bone with a physics part takes that body's motion, the others follow their
/// nearest ancestor that has one. Runs after the animators (it is added later), so it overrides their pose.
struct RagdollSkin {
    skeleton: sbox_resource::model::Skeleton,
    bodies: Vec<sbox_scene::GoId>,
    /// Skeleton bone of each body, if the part names one the skeleton has.
    bone_of: Vec<Option<usize>>,
    /// Inverse of each part's rest pose in model space.
    rest_inv: Vec<sbox_math::Mat4>,
    root: sbox_scene::GoId,
    parts: Vec<sbox_scene::GoId>,
}

impl Component for RagdollSkin {
    fn on_update(&mut self, scene: &mut Scene, _h: Handle) {
        use sbox_math::Mat4;
        let to_mat = |t: &Transform| Mat4::from_rotation_translation(t.rotation, t.position);
        let root_inv = to_mat(&scene.world_transform(self.root)).inverse();
        let mut by_bone: Vec<Option<Mat4>> = vec![None; self.skeleton.len()];
        for (i, &body) in self.bodies.iter().enumerate() {
            if let Some(b) = self.bone_of[i] {
                // Model space again: undo the model object's own transform, then move from the rest pose to the body.
                by_bone[b] = Some(root_inv * to_mat(&scene.world_transform(body)) * self.rest_inv[i]);
            }
        }
        let mut skin: Vec<Mat4> = Vec::with_capacity(self.skeleton.len());
        for i in 0..self.skeleton.len() {
            let m = by_bone[i].unwrap_or_else(|| match self.skeleton.parents.get(i) {
                Some(&p) if p >= 0 && (p as usize) < i => skin[p as usize],
                _ => Mat4::IDENTITY,
            });
            skin.push(m);
        }
        let mats: Vec<[[f32; 4]; 4]> = skin.iter().map(Mat4::to_cols_array_2d).collect();
        for &go in &self.parts {
            if let Some(m) = scene.component_mut::<ModelRenderer>(go) {
                m.bones = mats.clone();
            }
        }
    }
    impl_component_any!();
}

/// `--ragdoll` on a model: pressing R (the `Reload` action) turns the model into a ragdoll that drops to the floor.
struct RagdollSwitch {
    skeleton: sbox_resource::model::Skeleton,
    parts_data: Vec<sbox_resource::phys::PhysPart>,
    joints: Vec<sbox_resource::phys::PhysJoint>,
    parts: Vec<sbox_scene::GoId>,
    /// World height of the floor the ragdoll lands on (the model's lowest point).
    floor_z: f32,
    /// `--ragdoll-after <seconds>` drops the model without a key press (for screenshots).
    auto_after: Option<f32>,
    done: bool,
}

impl Component for RagdollSwitch {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        if self.done {
            return;
        }
        if let Some(t) = &mut self.auto_after {
            *t -= scene.delta;
        }
        if !scene.resource::<Input>().is_some_and(|i| i.pressed("Reload")) && !self.auto_after.is_some_and(|t| t <= 0.0) {
            return;
        }
        self.done = true;
        let root = h.go;
        let world = scene.world_transform(root);
        if scene.resource::<sbox_physics::Physics>().is_none() {
            sbox_physics::Physics::install(scene);
        }
        let floor = scene.create_object("ragdoll_floor");
        scene.set_local_transform(floor, Transform::from_position(Vec3::new(0.0, 0.0, self.floor_z)));
        scene.add_component(floor, sbox_physics::Collider { shape: sbox_physics::Shape::Plane, is_trigger: false });
        let bodies = assets::spawn_ragdoll(scene, &world, &self.parts_data, &self.joints);
        let by_name: std::collections::HashMap<&str, usize> = self.skeleton.names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let bone_of = self.parts_data.iter().map(|p| p.bone.as_deref().and_then(|b| by_name.get(b).copied())).collect();
        let rest_inv = self
            .parts_data
            .iter()
            .map(|p| sbox_math::Mat4::from_rotation_translation(sbox_math::Quat::from_xyzw(p.rotation[0], p.rotation[1], p.rotation[2], p.rotation[3]).normalize(), Vec3::from(p.position)).inverse())
            .collect();
        scene.add_component(root, RagdollSkin { skeleton: self.skeleton.clone(), bodies, bone_of, rest_inv, root, parts: self.parts.clone() });
        log::info!("ragdoll: {} bodies, {} joints", self.parts_data.len(), self.joints.len());
    }
    impl_component_any!();
}

/// Keeps a world panel facing the camera (`WorldPanel::look_at_camera`); the quad's front is its local -X side.
struct Billboard {
    camera: sbox_scene::GoId,
}

impl Component for Billboard {
    fn on_update(&mut self, scene: &mut Scene, h: Handle) {
        let to_camera = scene.world_transform(self.camera).position - scene.world_transform(h.go).position;
        let yaw = (-to_camera.y).atan2(-to_camera.x).to_degrees();
        let mut t = scene.local_transform(h.go);
        t.rotation = Angles::new(0.0, yaw, 0.0).to_rotation();
        scene.set_local_transform(h.go, t);
    }
    impl_component_any!();
}

/// The `--world-panel` page: what was drawn (to find buttons under a click) and what is needed to draw it again.
struct WorldPanelState {
    go: sbox_scene::GoId,
    page: page::Page,
    items: Vec<razor_ui::Item>,
    /// Size of the quad in world units and of its bitmap in pixels.
    size: [f32; 2],
    pixels: (u32, u32),
    font: ab_glyph::FontVec,
}

/// Simple fly camera: WASD + QE, arrow keys to look.
#[derive(Default)]
struct FlyCamera {
    keys: [bool; 10], // W A S D Q E Left Right Up Down
    angles: Angles,
    /// The camera is placed by the game (a network client's predicted player): arrow keys still look around, WASD / QE
    /// no longer fly.
    follow: bool,
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
        if !self.follow {
            t.position += wish * 300.0 * dt;
        }
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
    audio: Option<audio::Audio>,
    ui: Option<ui::Ui>,
    ui_font: Option<ab_glyph::FontVec>,
    /// Cursor position in physical pixels.
    cursor: (f32, f32),
    net: Option<Net>,
    mods: winit::keyboard::ModifiersState,
    player_mesh: Option<sbox_scene::MeshId>,
    clipboard: Option<arboard::Clipboard>,
    /// A left-button press started in a text field and has not been released.
    dragging: bool,
    /// The `--razor` page, if any, with its bound values and click handlers.
    page: Option<page::Page>,
    world_panel: Option<WorldPanelState>,
    /// Handlers for the RPCs this side accepts (host: `chat`; client: `notice`).
    rpcs: sbox_net::rpc::Registry<Scene>,
}

/// Calls both sides understand: a client's `chat(text)` is logged by the host and answered with `notice`; `notice(text)` is
/// logged by the client.
fn net_rpcs() -> sbox_net::rpc::Registry<Scene> {
    use sbox_net::rpc::{Arg, Kind, Registry};
    let mut r = Registry::<Scene>::default();
    r.register("chat", &[Kind::Str], |_, a| {
        if let [Arg::Str(t)] = a {
            log::info!("net: chat: {t}");
        }
    });
    r.register("notice", &[Kind::Str], |_, a| {
        if let [Arg::Str(t)] = a {
            log::info!("net: server says: {t}");
        }
    });
    r
}

/// `--auto-walk [seconds]`: hold Forward for that many seconds (default: for ever) after the start.
fn auto_walk_active(elapsed: f32) -> bool {
    let args: Vec<String> = std::env::args().collect();
    let Some(i) = args.iter().position(|a| a == "--auto-walk") else { return false };
    args.get(i + 1).and_then(|v| v.parse::<f32>().ok()).map_or(true, |secs| elapsed < secs)
}

/// `--host <addr>` serves the physics props to clients; `--join <addr>` mirrors a server's props.
enum Net {
    Host(sbox_net::Server, sbox_net::replication::RemotePlayers),
    Join(sbox_net::Client, Instant, sbox_net::replication::Prediction, sbox_net::replication::Interpolator),
}

impl App {
    fn build_scene(scene: &mut Scene, r: &mut Renderer, model: Option<&str>, anim: Option<&str>, scene_file: Option<&str>) -> sbox_scene::GoId {
        if let Some(i) = std::env::args().position(|a| a == "--map") {
            let name = std::env::args().nth(i + 1).expect("--map <maps/name/world.vwrld_c> --vpk <game dir>");
            return Self::build_map_scene(scene, r, &name);
        }
        if let Some(path) = scene_file {
            return Self::build_file_scene(scene, r, path);
        }
        if let Some(path) = model {
            return Self::build_model_scene(scene, r, path, anim);
        }
        let cube = r.upload_mesh(&MeshData::cuboid(Vec3::splat(64.0)));
        let ground = r.upload_mesh(&MeshData::plane(2000.0));
        let ball = r.upload_mesh(&MeshData::sphere(32.0, 32, 16));

        // With `--ssr` the demo uses glossy materials (the default roughness is far above the reflection cutoff).
        let glossy = std::env::args().any(|a| a == "--ssr").then(|| r.create_material_ex(&sbox_render::MaterialDesc { roughness: 0.12, ..Default::default() }));
        let g = scene.create_object("ground");
        let mut m = ModelRenderer::new(ground);
        if let Some(mat) = glossy {
            m.material = mat;
        }
        m.tint = [0.35, 0.4, 0.35, 1.0];
        scene.add_component(g, m);

        for i in 0..5 {
            let go = scene.create_object(&format!("cube{i}"));
            scene.set_local_transform(go, Transform::from_position(Vec3::new(300.0, (i as f32 - 2.0) * 120.0, 40.0)));
            let mut m = ModelRenderer::new(if i % 2 == 0 { cube } else { ball });
            if let Some(mat) = glossy {
                m.material = mat;
            }
            m.tint = [0.3 + 0.15 * i as f32, 0.8 - 0.12 * i as f32, 0.5, 1.0];
            scene.add_component(go, m);
            // `--phys-props` makes them physics props (instead of spinning decorations) so networked pushes can be tried.
            if std::env::args().any(|a| a == "--phys-props") {
                scene.add_component(go, sbox_physics::Rigidbody::default());
                let shape = if i % 2 == 0 { sbox_physics::Shape::Box { center: Vec3::ZERO, size: Vec3::splat(64.0) } } else { sbox_physics::Shape::Sphere { center: Vec3::ZERO, radius: 32.0 } };
                scene.add_component(go, sbox_physics::Collider { shape, is_trigger: false });
            } else {
                scene.add_component(go, Spinner { speed: 30.0 + 20.0 * i as f32 });
            }
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
    /// The UI font (Poppins) from the `game/` directory found in any path argument.
    fn load_ui_font(args: &[String]) -> Option<ab_glyph::FontVec> {
        let game = args.iter().flat_map(|a| std::path::Path::new(a).ancestors().map(std::path::Path::to_path_buf).collect::<Vec<_>>()).find(|p| p.file_name().is_some_and(|n| n == "game"))?;
        let fs = sbox_resource::vmat::GameFs::from_game_dir(game);
        text::font_files("Poppins", 600, false).iter().find_map(|p| fs.read_raw(p)).and_then(|d| ab_glyph::FontVec::try_from_vec(d).ok())
    }

    /// Rasterise `text` into a texture; returns it with its pixel size.
    fn text_texture(r: &mut Renderer, font: &ab_glyph::FontVec, text: &str, size: f32) -> (sbox_render::TextureId, [f32; 2]) {
        let st = text::TextStyle { text: text.to_string(), color: [1.0; 4], font: "Poppins".into(), size, weight: 600, italic: false, line_height: 1.0, align: -1, scale: 1.0 };
        let (img, w, h) = text::rasterise(font, &st);
        // `rasterise` supersamples by 4x; the on-screen size is a quarter of the texture.
        (r.upload_rgba8(&img, w, h, true), [w as f32 * 0.25, h as f32 * 0.25])
    }

    /// Pixel width `text` rasterises to.
    fn text_texture_width(font: &ab_glyph::FontVec, text: &str, size: f32) -> f32 {
        let st = text::TextStyle { text: text.to_string(), color: [1.0; 4], font: "Poppins".into(), size, weight: 600, italic: false, line_height: 1.0, align: -1, scale: 1.0 };
        text::rasterise(font, &st).1 as f32
    }

    /// `--world-panel <file.razor>` puts the page on a quad in the world, 600 units in front of the camera with its top edge 120 above eye level: the page is laid
    /// out at the `WorldPanel`'s size, drawn into a bitmap on the CPU and shown as an unlit translucent texture.
    /// `--world-panel-look` makes it face the camera. Needs a font from a path argument inside the game directory.
    fn build_world_panel(scene: &mut Scene, r: &mut Renderer, cam: sbox_scene::GoId, args: &[String]) -> Option<WorldPanelState> {
        let path = args.iter().position(|a| a == "--world-panel").and_then(|i| args.get(i + 1))?;
        let Some(font) = Self::load_ui_font(args) else {
            log::warn!("--world-panel: no font found (pass a path inside the game directory)");
            return None;
        };
        let doc = match std::fs::read_to_string(path).map_err(anyhow::Error::from).and_then(|src| razor::parse(&src)) {
            Ok(d) => d,
            Err(e) => {
                log::error!("--world-panel {path}: {e:#}");
                return None;
            }
        };
        let values: std::collections::HashMap<String, String> =
            args.windows(2).filter(|w| w[0] == "--bind").filter_map(|w| w[1].split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
        let panel = sbox_scene::WorldPanel { look_at_camera: args.iter().any(|a| a == "--world-panel-look"), ..Default::default() };
        let (pw, ph) = ((panel.panel_size[0] * panel.render_scale) as u32, (panel.panel_size[1] * panel.render_scale) as u32);
        let page = page::Page::new(doc, values);
        let items = page.items(pw as f32);
        let img = page::bitmap(&items, pw, ph, Some(&font));
        let tex = r.upload_rgba8(&img, pw, ph, true);
        let mat = r.create_material_ex(&sbox_render::MaterialDesc { color: Some(tex), unlit: true, translucent: true, ..Default::default() });
        // A vertical quad in the YZ plane facing -X, so a camera looking along +X sees it from the front. Its origin is the
        // middle of its top edge, so the page (which starts at the top) is what sits near eye level.
        let (hw, hh) = (panel.panel_size[0] * 0.5, panel.panel_size[1] * 0.5);
        let vertex = |y: f32, z: f32, u: f32, v: f32| sbox_render::Vertex { position: [0.0, y, z], normal: [-1.0, 0.0, 0.0], uv: [u, v], joints: [0; 4], weights: [0.0; 4] };
        let mesh = r.upload_mesh(&MeshData {
            vertices: vec![vertex(hw, -hh * 2.0, 0.0, 1.0), vertex(-hw, -hh * 2.0, 1.0, 1.0), vertex(-hw, 0.0, 1.0, 0.0), vertex(hw, 0.0, 0.0, 0.0)],
            indices: vec![0, 1, 2, 0, 2, 3],
        });
        let go = scene.create_object("world_panel");
        let at = scene.world_transform(cam);
        scene.set_local_transform(go, Transform::new(at.position + at.rotation * Vec3::new(600.0, 0.0, 120.0), Angles::new(0.0, 0.0, 0.0).to_rotation(), Vec3::ONE));
        let mut m = ModelRenderer::new(mesh);
        m.material = mat;
        m.cast_shadows = false;
        scene.add_component(go, m);
        if panel.look_at_camera {
            scene.add_component(go, Billboard { camera: cam });
        }
        let size = panel.panel_size;
        scene.add_component(go, panel);
        Some(WorldPanelState { go, page, items, size, pixels: (pw, ph), font })
    }

    /// A click at the cursor: if it lands on a button of the world panel, run the handler and redraw the panel.
    /// Returns whether the click was used.
    fn click_world_panel(&mut self, viewport: Option<(f32, f32)>) -> bool {
        let (Some(wp), Some(cam), Some(r)) = (&mut self.world_panel, self.camera, &mut self.renderer) else { return false };
        let fov = self.scene.component::<CameraComponent>(cam).map_or(60.0, |c| c.field_of_view);
        let Some((vw, vh)) = viewport.or_else(|| self.window.as_ref().map(|w| (w.inner_size().width as f32, w.inner_size().height as f32))) else { return false };
        let (origin, dir) = worldpanel::camera_ray(&self.scene.world_transform(cam), fov, vw, vh, self.cursor.0, self.cursor.1);
        let Some([u, v]) = worldpanel::panel_uv(&self.scene.world_transform(wp.go), wp.size, origin, dir) else { return false };
        let Some(id) = worldpanel::hit_button(&wp.items, u * wp.pixels.0 as f32, v * wp.pixels.1 as f32).map(String::from) else { return false };
        log::info!("world panel: clicked {id}");
        if wp.page.click(&id) == Some(page::Effect::Quit) {
            return true;
        }
        wp.items = wp.page.items(wp.pixels.0 as f32);
        let img = page::bitmap(&wp.items, wp.pixels.0, wp.pixels.1, Some(&wp.font));
        let tex = r.upload_rgba8(&img, wp.pixels.0, wp.pixels.1, true);
        let mat = r.create_material_ex(&sbox_render::MaterialDesc { color: Some(tex), unlit: true, translucent: true, ..Default::default() });
        if let Some(m) = self.scene.component_mut::<ModelRenderer>(wp.go) {
            m.material = mat;
        }
        true
    }

    /// `--hud "text"` shows a label in the top-left corner; `--ui-demo` adds a panel with Play / Quit buttons.
    fn build_ui(r: &mut Renderer, args: &[String]) -> Option<(ui::Ui, ab_glyph::FontVec, Option<page::Page>)> {
        let hud = args.iter().position(|a| a == "--hud").map(|i| args.get(i + 1).map_or("sbox-rs", String::as_str));
        let demo = args.iter().any(|a| a == "--ui-demo");
        let razor = args.iter().position(|a| a == "--razor").and_then(|i| args.get(i + 1));
        if hud.is_none() && !demo && razor.is_none() {
            return None;
        }
        let Some(font) = Self::load_ui_font(args) else {
            log::warn!("UI: no font found (pass a path inside the game directory)");
            return None;
        };
        let white = r.upload_rgba8(&[255; 4], 1, 1, true);
        let mut ui = ui::Ui::new(white);
        if let Some(text) = hud {
            let (texture, [w, h]) = Self::text_texture(r, &font, text, 28.0);
            ui.labels.push(ui::Label { texture, rect: [16.0, 16.0, w, h], tint: [1.0; 4] });
        }
        let mut page = None;
        if let Some(path) = razor {
            match std::fs::read_to_string(path).map_err(anyhow::Error::from).and_then(|src| razor::parse(&src)) {
                Ok(doc) => {
                    // `--bind Name=Value` (repeatable) supplies the data the page's `@Name` expressions and `@if` conditions read.
                    let values: std::collections::HashMap<String, String> = args
                        .windows(2)
                        .filter(|w| w[0] == "--bind")
                        .filter_map(|w| w[1].split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                        .collect();
                    let mut p = page::Page::new(doc, values);
                    p.populate(&mut ui, r, &font);
                    page = Some(p);
                }
                Err(e) => log::error!("--razor {path}: {e:#}"),
            }
        }
        if demo {
            // A 400x360 panel centred on a 1280x720 screen, laid out as a column: title, two buttons, a text field.
            let panel = [440.0, 180.0, 400.0, 360.0];
            ui.panels.push(ui::Panel { rect: panel, color: [0.05, 0.06, 0.1, 0.85] });
            let column = layout::Stack { axis: layout::Axis::Column, padding: 40.0, gap: 24.0, align: layout::Align::Stretch };
            use layout::Size::Fixed;
            let rects = column.place(panel, &[(Fixed(48.0), None), (Fixed(56.0), None), (Fixed(56.0), None), (Fixed(44.0), None)]);
            let (title, [tw, th]) = Self::text_texture(r, &font, "sbox-rs", 40.0);
            ui.labels.push(ui::Label { texture: title, rect: [rects[0][0] + (rects[0][2] - tw) * 0.5, rects[0][1] + (rects[0][3] - th) * 0.5, tw, th], tint: [1.0; 4] });
            for (i, (id, label)) in [("play", "Play"), ("quit", "Quit")].into_iter().enumerate() {
                ui.buttons.push(ui::Button {
                    id: id.into(),
                    rect: rects[1 + i],
                    color: [0.18, 0.2, 0.3, 1.0],
                    hover_color: [0.3, 0.4, 0.7, 1.0],
                    label: Some(Self::text_texture(r, &font, label, 28.0)),
                });
            }
            ui.fields.push(ui::TextField {
                id: "name".into(),
                rect: rects[3],
                text: String::new(),
                max_len: 24,
                color: [0.1, 0.11, 0.16, 1.0],
                focus_color: [0.16, 0.18, 0.28, 1.0],
                label: None,
                caret: 0,
                caret_x: 0.0,
                anchor: None,
                anchor_x: 0.0,
                offsets: vec![0.0],
            });
        }
        Some((ui, font, page))
    }

    /// Start networking from `--host <addr>` / `--join <addr>`. Rigidbody objects get ids in scene order, which is the same
    /// on both sides as long as both load the same map / scene.
    fn start_net(scene: &mut Scene, args: &[String]) -> Option<Net> {
        use sbox_net::replication::NetworkId;
        let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
        let (host, join) = (arg("--host"), arg("--join"));
        if host.is_none() && join.is_none() {
            return None;
        }
        // The demo scene has no physics world of its own: with `--phys-props` give it one and a floor to rest on.
        if scene.resource::<sbox_physics::Physics>().is_none() && args.iter().any(|a| a == "--phys-props") {
            sbox_physics::Physics::install(scene);
            let floor = scene.create_object("demo_floor");
            scene.add_component(floor, sbox_physics::Collider { shape: sbox_physics::Shape::Plane, is_trigger: false });
        }
        let bodies: Vec<_> = scene.all_objects().filter(|g| scene.component::<sbox_physics::Rigidbody>(*g).is_some()).collect();
        for (i, go) in bodies.into_iter().enumerate() {
            scene.add_component(go, NetworkId(i as u32));
        }
        if let Some(addr) = host {
            match sbox_net::Server::bind(&addr) {
                Ok(s) => {
                    log::info!("hosting on {}", s.local_addr().map(|a| a.to_string()).unwrap_or(addr));
                    return Some(Net::Host(s, Default::default()));
                }
                Err(e) => log::error!("--host {addr}: {e:#}"),
            }
        }
        if let Some(addr) = join {
            // The client keeps simulating props itself (prediction) and reconciles them with the server's.
            match sbox_net::Client::connect(&addr, "player") {
                Ok(c) => return Some(Net::Join(c, Instant::now(), Default::default(), Default::default())),
                Err(e) => log::error!("--join {addr}: {e:#}"),
            }
        }
        None
    }

    /// Per-frame networking: the host handles clients and broadcasts props, a client applies the latest snapshot.
    fn pump_net(&mut self, dt: f32) {
        use sbox_net::rpc::Arg;
        let mut spawn = self.camera.map_or(Vec3::Z * 64.0, |c| self.scene.world_transform(c).position);
        // `--spawn-z <z>` (host) drops new players to that height, e.g. to walk into the demo props.
        if let Some(z) = std::env::args().position(|a| a == "--spawn-z").and_then(|i| std::env::args().nth(i + 1)).and_then(|v| v.parse::<f32>().ok()) {
            spawn.z = z;
        }
        match &mut self.net {
            Some(Net::Host(server, players)) => {
                for e in server.poll() {
                    if !matches!(e, sbox_net::ServerEvent::Input { .. }) {
                        log::info!("net: {e:?}");
                    }
                    players.handle(&mut self.scene, &e, spawn);
                    match &e {
                        sbox_net::ServerEvent::Connected { client_id, name } => {
                            let _ = server.call_typed(*client_id, "notice", &[Arg::Str(format!("welcome, {name}"))]);
                        }
                        sbox_net::ServerEvent::Rpc { client_id, name, args } => match self.rpcs.dispatch(&mut self.scene, name, args) {
                            Ok(()) if name == "chat" => {
                                let _ = server.call_typed(*client_id, "notice", &[Arg::Str("chat received".into())]);
                            }
                            Ok(()) => {}
                            Err(err) => log::warn!("net: rejected rpc from {client_id}: {err}"),
                        },
                        _ => {}
                    }
                }
                server.broadcast(&sbox_net::replication::collect(&self.scene));
            }
            Some(Net::Join(client, last_try, prediction, interp)) => {
                use sbox_net::replication::{apply_except, apply_interpolated_players, PLAYER_ID_BASE};
                client.poll();
                for (name, args) in client.take_rpcs() {
                    if let Err(err) = self.rpcs.dispatch(&mut self.scene, &name, &args) {
                        log::warn!("net: rejected rpc {name:?}: {err}");
                    }
                }
                let mut own = None;
                if client.client_id.is_none() && last_try.elapsed().as_secs_f32() > 0.5 {
                    client.retry_connect("player");
                    *last_try = Instant::now();
                } else if let Some(id) = client.client_id {
                    own = Some(PLAYER_ID_BASE + id);
                    // Movement from the key actions and the look direction of the local camera, predicted locally.
                    let input = self.scene.resource::<Input>().cloned().unwrap_or_default();
                    let yaw = self.camera.map_or(0.0, |c| self.scene.world_transform(c).rotation.angles().yaw);
                    let predicted = prediction.step(client, [if auto_walk_active(self.scene.time as f32) { 1.0 } else { input.axis("Forward", "Backward") }, input.axis("Left", "Right")], yaw, 0.0, 0, dt);
                    if let Some(p) = predicted {
                        let go = self.scene.iter_components::<sbox_net::replication::NetworkId>().find(|(_, n)| Some(n.0) == own).map(|(go, _)| go);
                        if let Some(go) = go {
                            let mut t = self.scene.world_transform(go);
                            t.position = p;
                            self.scene.set_world_transform(go, t);
                        }
                        // The camera rides on the predicted player (its origin is the eye, as spawned).
                        if let Some(cam) = self.camera {
                            let mut c = self.scene.world_transform(cam);
                            c.position = p;
                            self.scene.set_world_transform(cam, c);
                            if let Some(f) = self.scene.component_mut::<FlyCamera>(cam) {
                                f.follow = true;
                            }
                        }
                    }
                }
                // Other objects: create what is missing, then show them slightly in the past, blended between snapshots.
                apply_except(&mut self.scene, client, 0.0, own);
                interp.advance(dt);
                interp.record(client);
                apply_interpolated_players(&mut self.scene, interp, own);
                props::reconcile(&mut self.scene, &client.entities, dt);
                for id in client.take_removed() {
                    let gone: Vec<_> = self.scene.iter_components::<sbox_net::replication::NetworkId>().filter(|(_, n)| n.0 == id).map(|(go, _)| go).collect();
                    for go in gone {
                        self.scene.destroy_object(go);
                    }
                }
            }
            None => {}
        }
        self.attach_player_models();
    }

    /// Give every networked player object (host and clients) a visible box body, coloured by its id.
    fn attach_player_models(&mut self) {
        use sbox_net::replication::{NetworkId, PLAYER_ID_BASE};
        let Some(r) = &mut self.renderer else { return };
        let bare: Vec<_> = self
            .scene
            .iter_components::<NetworkId>()
            .filter(|(go, n)| n.0 >= PLAYER_ID_BASE && self.scene.component::<ModelRenderer>(*go).is_none())
            .map(|(go, n)| (go, n.0))
            .collect();
        for (go, id) in bare {
            let mesh = *self.player_mesh.get_or_insert_with(|| r.upload_mesh(&MeshData::cuboid(Vec3::new(32.0, 32.0, 72.0))));
            let mut m = ModelRenderer::new(mesh);
            // A stable, distinct hue per player.
            let h = (id - PLAYER_ID_BASE) as f32 * 0.618_034;
            let c = |o: f32| 0.55 + 0.45 * ((h + o).fract() * std::f32::consts::TAU).cos();
            m.tint = [c(0.0), c(0.33), c(0.66), 1.0];
            self.scene.add_component(go, m);
            // Players are kinematic bodies on host and clients alike, so they shove props (the client predicts that push).
            if matches!(self.net, Some(Net::Host(..) | Net::Join(..))) {
                self.scene.add_component(go, sbox_physics::Rigidbody { kinematic: true, ..Default::default() });
                self.scene.add_component(go, sbox_physics::Collider { shape: sbox_physics::Shape::Box { center: Vec3::ZERO, size: Vec3::new(32.0, 32.0, 72.0) }, is_trigger: false });
            }
        }
    }

    /// Re-rasterise the labels of text fields that changed and refresh the overlay.
    fn refresh_ui(&mut self) {
        let (Some(ui), Some(font), Some(r)) = (&mut self.ui, &self.ui_font, &mut self.renderer) else { return };
        for i in ui.take_dirty() {
            let text = ui.fields[i].text.clone();
            ui.fields[i].label = (!text.is_empty()).then(|| Self::text_texture(r, font, &text, 24.0));
            // Pixel offset of every caret position, so mouse clicks can place the caret and the selection can be drawn.
            let chars: Vec<char> = text.chars().collect();
            let offsets: Vec<f32> = (0..=chars.len())
                .map(|n| if n == 0 { 0.0 } else { Self::text_texture_width(font, &chars[..n].iter().collect::<String>(), 24.0) })
                .collect();
            let f = &mut ui.fields[i];
            f.caret_x = offsets[f.caret.min(chars.len())];
            f.anchor_x = f.anchor.map_or(0.0, |a| offsets[a.min(chars.len())]);
            f.offsets = offsets;
        }
        r.overlay = ui.rects();
    }

    /// Load a compiled map: `--map maps/dev/preview_flat/world.vwrld_c --game <game dir>`.
    fn build_map_scene(scene: &mut Scene, r: &mut Renderer, world: &str) -> sbox_scene::GoId {
        use sbox_resource::vmat;
        let args: Vec<String> = std::env::args().collect();
        let game = args.iter().position(|a| a == "--game").and_then(|i| args.get(i + 1)).expect("--game <game dir>");
        let mut assets = Assets::new(Some(vmat::GameFs::from_game_dir(game)));
        let (n, mins, maxs, ents) = assets.load_map(scene, r, world);
        log::info!("map {world}: {n} objects, bounds {mins} .. {maxs}, {} props, sun {}", ents.props, ents.has_sun);
        if !ents.has_sun {
            let sun = scene.create_object("sun");
            scene.set_local_transform(sun, Transform::new(Vec3::ZERO, Angles::new(50.0, 30.0, 0.0).to_rotation(), Vec3::ONE));
            scene.add_component(sun, DirectionalLight::default());
        }
        let center = if n > 0 { (mins + maxs) * 0.5 } else { Vec3::ZERO };
        let extent = if n > 0 { (maxs - mins).length() } else { 500.0 };
        let cam = scene.create_object("camera");
        let (cam_pos, cam_angles) = match ents.player_start {
            Some((p, yaw)) => (p + Vec3::Z * 64.0, Angles::new(0.0, yaw, 0.0)),
            None => (center + Vec3::new(-extent * 0.4, 0.0, extent * 0.2), Angles::new(15.0, 0.0, 0.0)),
        };
        scene.set_local_transform(cam, Transform::new(cam_pos, cam_angles.to_rotation(), Vec3::ONE));
        scene.add_component(cam, CameraComponent::default());
        scene.add_component(cam, FlyCamera::default());
        scene.insert_resource(Input::default());
        cam
    }

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
        let game_dir = std::path::Path::new(path).ancestors().find(|p| p.file_name().map_or(false, |n| n == "game"));
        let fs = game_dir.map(vmat::GameFs::from_game_dir);
        let md = model::load_with(&bytes, &|p| fs.as_ref().and_then(|f| f.read(p))).expect("load model");
        let meshes: Vec<_> = md.default_meshes().collect();
        log::info!("{path}: {} meshes ({} shown), {} bones", md.meshes.len(), meshes.len(), md.skeleton.len());

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
        if let (Some(gp), Some(_)) = (args.iter().position(|a| a == "--graph").and_then(|i| args.get(i + 1)), fs.as_ref()) {
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
            scene.add_component(root, Animator { anim: a, skeleton: md.skeleton.clone(), anim_skel, parts: parts.clone(), time: 0.0 });
        }

        if args.iter().any(|a| a == "--ragdoll") {
            match &loaded.model.ragdoll {
                Some((p, j)) => {
                    scene.add_component(
                    root,
                    RagdollSwitch { skeleton: md.skeleton.clone(), parts_data: p.clone(), joints: j.clone(), parts: parts.clone(), floor_z: loaded.mins.z - center.z, auto_after: args.iter().position(|a| a == "--ragdoll-after").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()), done: false },
                    );
                }
                None => log::warn!("--ragdoll: {path} has no jointed physics"),
            }
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
        if let Some(cam) = self.camera {
            self.world_panel = Self::build_world_panel(&mut self.scene, &mut renderer, cam, &args);
        }
        // `--sky <material.vmat_c> --game <dir>` installs that sky cubemap; `--cubefog` also adds a CubemapFog to the camera.
        if let (Some(sky), Some(game)) = (args.iter().position(|a| a == "--sky").and_then(|i| args.get(i + 1)), args.iter().position(|a| a == "--game").and_then(|i| args.get(i + 1))) {
            Assets::new(Some(sbox_resource::vmat::GameFs::from_game_dir(std::path::PathBuf::from(game)))).load_sky(&mut renderer, sky, [1.0; 4]);
            if let (Some(cam), true) = (self.camera, args.iter().any(|a| a == "--cubefog")) {
                self.scene.add_component(cam, sbox_scene::CubemapFog::default());
            }
        }
        // `--ssr` turns on screen-space reflections without needing a scene file that has the component.
        if args.iter().any(|a| a == "--ssr") {
            if let Some(cam) = self.camera {
                self.scene.add_component(cam, sbox_scene::ScreenSpaceReflections::default());
            }
        }
        self.net = Self::start_net(&mut self.scene, &args);
        if let Some((ui, font, page)) = Self::build_ui(&mut renderer, &args) {
            self.ui = Some(ui);
            self.page = page;
            // A `ScreenPanel` in the scene sets the page's opacity and scale.
            if let (Some(p), Some((_, sp))) = (&mut self.page, self.scene.iter_components::<sbox_scene::ScreenPanel>().next()) {
                p.opacity = sp.opacity;
                p.scale = sp.scale;
                if let (Some(ui), Some(font)) = (&mut self.ui, &self.ui_font) {
                    p.populate(ui, &mut renderer, font);
                }
            }
            self.ui_font = Some(font);
        }
        if let Some(ui) = &self.ui {
            renderer.overlay = ui.rects();
        }
        if let Some(i) = args.iter().position(|a| a == "--screenshot") {
            let out = args.get(i + 1).expect("--screenshot <file.png>");
            let secs: f32 = args.iter().position(|a| a == "--time").and_then(|j| args.get(j + 1)).and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let (w, h) = (1280, 720);
            // With a network session the loop runs in real time and pumps it, so a screenshot can show what arrived.
            let networked = self.net.is_some();
            self.renderer = Some(renderer);
            for _ in 0..(secs * 60.0) as u32 {
                self.scene.tick(1.0 / 60.0);
                if networked {
                    self.pump_net(1.0 / 60.0);
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
            let mut renderer = self.renderer.take().expect("renderer");
            self.renderer = Some(renderer);
            // `--click <x> <y>` presses the left button at that window pixel (a world panel button, say) before the shot.
            if let Some(i) = args.iter().position(|a| a == "--click") {
                let g = |o: usize| args.get(i + o).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                self.cursor = (g(1), g(2));
                let used = self.click_world_panel(Some((1280.0, 720.0)));
                log::info!("click at {:?}: used = {used}", self.cursor);
            }
            let mut renderer = self.renderer.take().expect("renderer");
            let flag = |name: &str| args.iter().position(|a| a == name).and_then(|j| args.get(j + 1)).and_then(|v| v.parse::<f32>().ok());
            // `--cam-zoom <k>` moves the camera k times closer to the point it looks at (the model centre, or `--cam-target x y z`).
            if let (Some(c), Some(k)) = (self.camera, flag("--cam-zoom")) {
                let t = self.scene.world_transform(c);
                let target = args.iter().position(|a| a == "--cam-target").map_or(Vec3::ZERO, |i| {
                    let g = |o: usize| args.get(i + o).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                    Vec3::new(g(1), g(2), g(3))
                });
                let mut n = t;
                n.position = target + (t.position - target) / k.max(0.01);
                self.scene.set_world_transform(c, n);
            }
            if let (Some(c), Some(yaw)) = (self.camera, flag("--cam-yaw")) {
                // Swing the camera round the origin (where model viewers centre the model) and keep looking at it.
                let t = self.scene.world_transform(c);
                let r = (t.position.x * t.position.x + t.position.y * t.position.y).sqrt();
                let rad = yaw.to_radians();
                let mut n = t;
                n.position = Vec3::new(-rad.cos() * r, -rad.sin() * r, t.position.z);
                n.rotation = Angles::new(t.rotation.angles().pitch, yaw, 0.0).to_rotation();
                self.scene.set_world_transform(c, n);
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
        if args.iter().any(|a| a == "--audio") && args.iter().all(|a| a != "--screenshot") {
            self.audio = audio::Audio::new();
            if let Some(a) = &self.audio {
                self.scene.insert_resource(a.handle());
            }
        }
        if let Some(i) = args.iter().position(|a| a == "--sound") {
            let path = args.get(i + 1).expect("--sound <file.vsnd_c>");
            self.audio = audio::Audio::new();
            match (&mut self.audio, std::fs::read(path)) {
                (Some(a), Ok(b)) => log::info!("sound {path}: {}", if a.play(&b, args.iter().any(|x| x == "--loop"), 0.8) { "playing" } else { "not playing" }),
                (None, _) => log::warn!("no audio output device"),
                (_, Err(e)) => log::warn!("read {path}: {e}"),
            }
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
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if self.dragging {
                    if let Some(ui) = &mut self.ui {
                        ui.drag_to(self.cursor.0);
                    }
                    self.refresh_ui();
                } else if let (Some(ui), Some(r)) = (&mut self.ui, &mut self.renderer) {
                    ui.set_mouse(self.cursor.0, self.cursor.1);
                    r.overlay = ui.rects();
                }
            }
            WindowEvent::Resized(s) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(s.width, s.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.ui.as_ref().is_some_and(ui::Ui::has_focus) => {
                if event.state == ElementState::Pressed {
                    let ui = self.ui.as_mut().unwrap();
                    match &event.logical_key {
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                            ui.backspace();
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Delete) => {
                            ui.delete_forward();
                        }
                        winit::keyboard::Key::Named(k @ (winit::keyboard::NamedKey::ArrowLeft | winit::keyboard::NamedKey::ArrowRight | winit::keyboard::NamedKey::Home | winit::keyboard::NamedKey::End)) => {
                            use winit::keyboard::NamedKey as K;
                            ui.move_caret(
                                match k {
                                    K::ArrowLeft => ui::CaretMove::Left,
                                    K::ArrowRight => ui::CaretMove::Right,
                                    K::Home => ui::CaretMove::Home,
                                    _ => ui::CaretMove::End,
                                },
                                self.mods.shift_key(),
                            );
                        }
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter | winit::keyboard::NamedKey::Escape) => {
                            if let Some((id, text)) = ui.submit() {
                                log::info!("ui: {id} = {text:?}");
                            }
                        }
                        winit::keyboard::Key::Character(c) if self.mods.control_key() && c.eq_ignore_ascii_case("a") => {
                            ui.select_all();
                        }
                        winit::keyboard::Key::Character(c) if self.mods.control_key() && (c.eq_ignore_ascii_case("c") || c.eq_ignore_ascii_case("x")) => {
                            if let (Some(text), Some(cb)) = (ui.selected_text(), &mut self.clipboard) {
                                let _ = cb.set_text(text);
                                if c.eq_ignore_ascii_case("x") {
                                    ui.backspace();
                                }
                            }
                        }
                        winit::keyboard::Key::Character(c) if self.mods.control_key() && c.eq_ignore_ascii_case("v") => {
                            if let Some(text) = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
                                ui.paste(&text);
                            }
                        }
                        _ if self.mods.control_key() => {}
                        _ => {
                            for c in event.text.as_deref().unwrap_or("").chars() {
                                ui.type_char(c);
                            }
                        }
                    }
                    self.refresh_ui();
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
                        KeyCode::ControlLeft => Some("Duck"),
                        KeyCode::KeyE => Some("Use"),
                        KeyCode::KeyR => Some("Reload"),
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
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if let (Some(ui), Some(w)) = (&mut self.ui, &self.window) {
                    ui.viewport_height = w.inner_size().height as f32;
                    if ui.scroll_at(self.cursor.0, self.cursor.1, -notches * 40.0) {
                        self.refresh_ui();
                        return;
                    }
                }
                if let Some(input) = self.scene.resource_mut::<Input>() {
                    input.mouse_wheel += notches;
                }
            }
            WindowEvent::MouseInput { state, button, .. } if button != MouseButton::Left || state == ElementState::Released => {
                if button == MouseButton::Left {
                    self.dragging = false;
                }
                let action = match button {
                    MouseButton::Left => Some("Attack1"),
                    MouseButton::Right => Some("Attack2"),
                    MouseButton::Middle => Some("Attack3"),
                    _ => None,
                };
                if let (Some(a), Some(input)) = (action, self.scene.resource_mut::<Input>()) {
                    input.set(a, state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(ui) = &mut self.ui {
                    self.dragging = ui.focus_at(self.cursor.0, self.cursor.1);
                }
                self.refresh_ui();
                if let Some(id) = self.ui.as_ref().and_then(|ui| ui.click(self.cursor.0, self.cursor.1)) {
                    log::info!("ui: clicked {id}");
                    let effect = self.page.as_mut().and_then(|p| p.click(&id));
                    if effect == Some(page::Effect::Quit) || (effect.is_none() && id.eq_ignore_ascii_case("quit")) {
                        el.exit();
                    }
                    if effect.is_some() {
                        // The handler changed the page's values: lay it out again.
                        if let (Some(p), Some(ui), Some(font), Some(r)) = (&mut self.page, &mut self.ui, &self.ui_font, &mut self.renderer) {
                            p.populate(ui, r, font);
                            r.overlay = ui.rects();
                        }
                    }
                    return;
                }
                if self.click_world_panel(None) {
                    return;
                }
                if let (Some(w), Some(input)) = (&self.window, self.scene.resource_mut::<Input>()) {
                    input.set("Attack1", true);
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
                self.pump_net(dt);
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
    let mut app = App { window: None, renderer: None, scene: Scene::new(), camera: None, last: Instant::now(), audio: None, ui: None, ui_font: None, cursor: (0.0, 0.0), net: None, mods: Default::default(), player_mesh: None, clipboard: arboard::Clipboard::new().ok(), dragging: false, page: None, world_panel: None, rpcs: net_rpcs() };
    el.run_app(&mut app)?;
    Ok(())
}
