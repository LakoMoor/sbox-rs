//! Asset cache and the scene-file component registry.

use sbox_math::{Transform, Vec3};
use sbox_physics::{Collider, Rigidbody, Shape};
use sbox_scene::parse_vec3;
use sbox_render::{MaterialDesc, MeshData, Renderer};
use sbox_resource::{model, vmat, vtex};
use sbox_scene::{
    parse_color, CameraComponent, ComponentLoader, DirectionalLight, GoId, MaterialId, MeshId, ModelRenderer, PointLight, Scene,
};
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;

pub struct LoadedModel {
    pub model: model::Model,
    pub parts: Vec<(MeshId, MaterialId)>,
    pub mins: Vec3,
    pub maxs: Vec3,
}

impl LoadedModel {
    /// Collision shape: the model's hulls / spheres / capsules / triangle meshes, or its bounding box when it has no usable physics data.
    pub fn collision_shape(&self) -> Shape {
        use sbox_resource::phys::PhysShape;
        let v = |a: &[f32; 3]| Vec3::from(*a);
        match &self.model.physics {
            Some(list) => Shape::Compound(
                list.iter()
                    .map(|s| match s {
                        PhysShape::Hull(p) => Shape::Hull(p.iter().map(v).collect()),
                        PhysShape::Sphere { center, radius } => Shape::Sphere { center: v(center), radius: *radius },
                        PhysShape::Capsule { a, b, radius } => Shape::Capsule { a: v(a), b: v(b), radius: *radius },
                        PhysShape::Mesh { vertices, indices } => Shape::TriMesh { vertices: vertices.iter().map(v).collect(), indices: indices.clone() },
                    })
                    .collect(),
            ),
            None => Shape::Box { center: (self.mins + self.maxs) * 0.5, size: self.maxs - self.mins },
        }
    }
}

/// Turn physics parts into dynamic bodies at `root` + their rest pose, tied together by twist / swing joints. Returns one
/// object per part. Only the collision shapes exist: the bodies are not drawn and no skeleton follows them yet.
pub fn spawn_ragdoll(scene: &mut Scene, root: &Transform, parts: &[sbox_resource::phys::PhysPart], joints: &[sbox_resource::phys::PhysJoint]) -> Vec<sbox_scene::GoId> {
    use sbox_math::Quat;
    use sbox_resource::phys::PhysShape;
    let v = |a: &[f32; 3]| Vec3::from(*a);
    let bodies: Vec<_> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let go = scene.create_object(&format!("ragdoll_part_{i}"));
            let local = Transform::new(v(&p.position), Quat::from_xyzw(p.rotation[0], p.rotation[1], p.rotation[2], p.rotation[3]), Vec3::ONE);
            scene.set_local_transform(go, Transform::new(root.position + root.rotation * (local.position * root.scale), root.rotation * local.rotation, root.scale));
            scene.add_component(go, Rigidbody { mass_override: (p.mass > 0.0).then_some(p.mass), ..Default::default() });
            let shapes = p
                .shapes
                .iter()
                .map(|s| match s {
                    PhysShape::Hull(h) => Shape::Hull(h.iter().map(v).collect()),
                    PhysShape::Sphere { center, radius } => Shape::Sphere { center: v(center), radius: *radius },
                    PhysShape::Capsule { a, b, radius } => Shape::Capsule { a: v(a), b: v(b), radius: *radius },
                    PhysShape::Mesh { vertices, indices } => Shape::TriMesh { vertices: vertices.iter().map(v).collect(), indices: indices.clone() },
                })
                .collect();
            scene.add_component(go, Collider { shape: Shape::Compound(shapes), is_trigger: false });
            go
        })
        .collect();
    // A body can carry only one `Joint` component, so extra joints of the same body hang off child helper objects.
    for j in joints {
        let q = |f: &([f32; 3], [f32; 4])| Transform::new(v(&f.0), Quat::from_xyzw(f.1[0], f.1[1], f.1[2], f.1[3]), Vec3::ONE);
        let owner = if scene.component::<sbox_physics::Joint>(bodies[j.body1]).is_none() {
            bodies[j.body1]
        } else {
            scene.create_object_in(Some(bodies[j.body1]), "ragdoll_joint")
        };
        let joint = sbox_physics::Joint {
            other: bodies[j.body2],
            frame1: q(&j.frame1),
            frame2: q(&j.frame2),
            swing_limit: j.swing,
            twist_limit: j.twist,
            collide: j.collide,
        };
        scene.add_component(owner, joint);
    }
    bodies
}

pub struct Assets {
    pub fs: Option<vmat::GameFs>,
    models: HashMap<String, Option<Rc<LoadedModel>>>,
    materials: HashMap<String, MaterialId>,
}

impl Assets {
    pub fn new(fs: Option<vmat::GameFs>) -> Self {
        Self { fs, models: HashMap::new(), materials: HashMap::new() }
    }

    pub fn material(&mut self, r: &mut Renderer, path: &str) -> MaterialId {
        if path.is_empty() {
            return MaterialId(0);
        }
        if let Some(m) = self.materials.get(path) {
            return *m;
        }
        let id = self.build_material(r, path);
        self.materials.insert(path.to_string(), id);
        id
    }

    fn build_material(&mut self, r: &mut Renderer, path: &str) -> MaterialId {
        let Some(fs) = &self.fs else { return MaterialId(0) };
        let Some(data) = fs.read(path) else {
            log::warn!("material not found: {path}");
            return MaterialId(0);
        };
        let m = match vmat::parse(&data) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("material {path}: {e:#}");
                return MaterialId(0);
            }
        };
        let tex = m
            .color_texture()
            .and_then(|t| fs.read(t))
            .and_then(|d| vtex::parse(&d).map_err(|e| log::warn!("texture: {e:#}")).ok())
            .and_then(|t| r.upload_texture(&t));
        let mut load_linear = |path: Option<&str>| {
            path.and_then(|t| fs.read(t))
                .and_then(|d| vtex::parse(&d).map_err(|e| log::warn!("texture: {e:#}")).ok())
                .and_then(|t| r.upload_texture_ex(&t, false))
        };
        let normal = load_linear(m.normal_texture());
        let rma = load_linear(m.rma_texture());
        let mut desc = MaterialDesc {
            color: tex,
            normal,
            rma,
            tint: m.color_tint(),
            uv_scale: m.uv_scale(),
            roughness: if rma.is_some() { m.float("g_flRoughnessScaleFactor", 1.0) } else { 0.7 },
            metalness: m.float("g_flMetalness", 0.0),
            bump: m.float("g_flBumpStrength", 1.0),
            unlit: m.flag("F_UNLIT"),
            translucent: m.flag("F_TRANSLUCENT"),
            additive: false,
            alpha_cutoff: if m.flag("F_ALPHA_TEST") { m.float("g_flAlphaTestReference", 0.5) } else { 0.0 },
        };
        crate::families::Family::of(&m.shader).apply(&mut desc);
        r.create_material_ex(&desc)
    }

    /// Resolve a sky material to its HDR cubemap and install it on the renderer.
    pub fn load_sky(&mut self, r: &mut Renderer, material: &str, tint: [f32; 4]) {
        let Some(fs) = &self.fs else { return };
        let Some(m) = fs.read(material).and_then(|d| vmat::parse(&d).ok()) else {
            log::warn!("sky material not found: {material}");
            return;
        };
        let Some(tex_path) = m.sky_texture() else {
            log::warn!("sky material {material} has no texture");
            return;
        };
        match fs.read(tex_path).map(|d| vtex::parse(&d)) {
            Some(Ok(t)) => {
                if !r.set_skybox(&t, 1.0, tint) {
                    log::warn!("sky texture {tex_path}: unsupported ({:?}, cube={})", t.format, t.is_cube());
                }
            }
            Some(Err(e)) => log::warn!("sky texture {tex_path}: {e:#}"),
            None => log::warn!("sky texture not found: {tex_path}"),
        }
    }

    pub fn load_model_bytes(&mut self, r: &mut Renderer, key: &str, bytes: &[u8]) -> Option<Rc<LoadedModel>> {
        if let Some(m) = self.models.get(key) {
            return m.clone();
        }
        let loaded = match model::load_with(bytes, &|p| self.fs.as_ref().and_then(|f| f.read(p))) {
            Ok(md) => {
                let (mut mins, mut maxs) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                let mut parts = Vec::new();
                for mm in md.default_meshes() {
                    mins = mins.min(Vec3::from(mm.mesh.mins));
                    maxs = maxs.max(Vec3::from(mm.mesh.maxs));
                    for dc in &mm.mesh.draw_calls {
                        let mesh = r.upload_mesh(&MeshData::from_draw_call(dc));
                        parts.push((mesh, self.material(r, &dc.material)));
                    }
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(Rc::new(LoadedModel { model: md, parts, mins, maxs }))
                }
            }
            Err(e) => {
                log::warn!("model {key}: {e:#}");
                None
            }
        };
        self.models.insert(key.to_string(), loaded.clone());
        loaded
    }

    pub fn load_model(&mut self, r: &mut Renderer, path: &str) -> Option<Rc<LoadedModel>> {
        if let Some(m) = self.models.get(path) {
            return m.clone();
        }
        let bytes = self.fs.as_ref().and_then(|fs| fs.read(path));
        match bytes {
            Some(b) => self.load_model_bytes(r, path, &b),
            None => {
                log::warn!("model not found: {path}");
                self.models.insert(path.to_string(), None);
                None
            }
        }
    }

    /// Spawn one child object per draw call under `parent`.
    pub fn spawn(&self, scene: &mut Scene, parent: GoId, m: &LoadedModel, tint: [f32; 4], material_override: Option<MaterialId>) -> Vec<GoId> {
        m.parts
            .iter()
            .enumerate()
            .map(|(i, (mesh, mat))| {
                let go = scene.create_object_in(Some(parent), &format!("part{i}"));
                let mut r = ModelRenderer::new(*mesh);
                r.material = material_override.unwrap_or(*mat);
                r.tint = tint;
                scene.add_component(go, r);
                go
            })
            .collect()
    }
}

/// Maps s&box component type names from scene files onto the components this port has.
pub struct SceneLoader<'a> {
    pub r: &'a mut Renderer,
    pub assets: &'a mut Assets,
    pub camera: Option<GoId>,
}

fn strip_ns(t: &str) -> &str {
    t.rsplit('.').next().unwrap_or(t)
}

impl ComponentLoader for SceneLoader<'_> {
    fn create(&mut self, scene: &mut Scene, go: GoId, type_name: &str, p: &Value) -> bool {
        match strip_ns(type_name) {
            "ModelRenderer" | "SkinnedModelRenderer" | "Prop" => {
                let Some(path) = p.get("Model").and_then(Value::as_str) else { return true };
                let Some(m) = self.assets.load_model(self.r, path) else { return true };
                let tint = parse_color(p.get("Tint"), [1.0; 4]);
                let ov = p.get("MaterialOverride").and_then(Value::as_str).map(|s| self.assets.material(self.r, s));
                self.assets.spawn(scene, go, &m, tint, ov);
                true
            }
            "TextRenderer" => {
                let Some(ts) = p.get("TextScope") else { return true };
                let f = |k: &str, d: f64| ts.get(k).and_then(Value::as_f64).unwrap_or(d);
                let family = ts.get("FontName").and_then(Value::as_str).unwrap_or("Poppins");
                let weight = f("FontWeight", 400.0) as i64;
                let italic = ts.get("FontItalic").and_then(Value::as_bool).unwrap_or(false);
                let font = self.assets.fs.as_ref().and_then(|fs| {
                    crate::text::font_files(family, weight, italic)
                        .iter()
                        .find_map(|path| fs.read_raw(path))
                        .and_then(|d| ab_glyph::FontVec::try_from_vec(d).ok())
                });
                if let Some(font) = font {
                    let st = crate::text::TextStyle {
                        text: ts.get("Text").and_then(Value::as_str).unwrap_or("").to_string(),
                        color: parse_color(ts.get("TextColor"), [1.0; 4]),
                        font: family.to_string(),
                        size: f("FontSize", 32.0) as f32,
                        weight,
                        italic,
                        line_height: f("LineHeight", 1.0) as f32,
                        align: match p.get("TextAlignment").and_then(Value::as_str) {
                            Some("Left") => -1,
                            Some("Right") => 1,
                            _ => 0,
                        },
                        scale: p.get("Scale").and_then(Value::as_f64).unwrap_or(1.0) as f32,
                    };
                    crate::text::spawn_text(scene, self.r, go, &font, &st);
                } else {
                    log::warn!("font not found: {family}");
                }
                true
            }
            "SoundPointComponent" | "SoundBoxComponent" => {
                let Some(ev_path) = p.get("SoundEvent").and_then(Value::as_str) else { return true };
                let Some(fs) = self.assets.fs.as_ref() else { return true };
                let Some(ev) = fs.read(ev_path).and_then(|d| sbox_resource::sound_event::parse(&d).ok()) else {
                    log::warn!("sound event not found: {ev_path}");
                    return true;
                };
                let Some(file) = ev.sounds.first().and_then(|s| fs.read(s)) else { return true };
                let looping = p.get("MinRepeatTime").and_then(Value::as_f64).map_or(false, |t| t <= 0.0);
                let vol = p.get("Volume").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                if p.get("PlayOnStart").and_then(Value::as_bool).unwrap_or(true) {
                    scene.add_component(go, crate::game::SoundPoint::new(ev, file, looping, vol));
                }
                true
            }
            "ParticleEffect" => {
                scene.add_component(go, crate::particles::ParticleEffect::from_json(p));
                true
            }
            "ParticleSphereEmitter" | "ParticleConeEmitter" => {
                scene.add_component(go, crate::particles::ParticleEmitter::from_json(p, strip_ns(type_name) == "ParticleConeEmitter"));
                true
            }
            "ParticleSpriteRenderer" => {
                let tex_path = p
                    .pointer("/Sprite/data/Animations/0/Frames/0/Texture")
                    .and_then(Value::as_str)
                    .map(|t| t.trim_end_matches("_c").to_string());
                let parsed = tex_path
                    .as_deref()
                    .and_then(|t| self.assets.fs.as_ref().and_then(|fs| fs.read(t)))
                    .and_then(|d| sbox_resource::vtex::parse(&d).map_err(|e| log::warn!("sprite texture: {e:#}")).ok());
                let sheet = parsed.as_ref().map(|t| t.sheet.clone()).unwrap_or_default();
                let tex = parsed.and_then(|t| self.r.upload_texture(&t));
                let additive = p.get("Additive").and_then(Value::as_bool).unwrap_or(false);
                let scale = p.get("Scale").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                scene.add_component(go, crate::particles::ParticleSpriteRenderer::new(self.r, tex, &sheet, scale, additive));
                true
            }
            "TemporaryEffect" => {
                let secs = p.get("DestroyAfterSeconds").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                scene.add_component(go, crate::game::TemporaryEffect::new(secs));
                true
            }
            "CubeController" => {
                let speed = p.get("MoveSpeed").and_then(Value::as_f64).unwrap_or(100.0) as f32;
                scene.add_component(go, crate::game::CubeController { move_speed: speed });
                true
            }
            // Post-processing / environment components with no renderer support yet; accepted so they are not reported as unknown.
            "SkyBox2D" => {
                let tint = parse_color(p.get("Tint"), [1.0; 4]);
                if let Some(mat) = p.get("SkyMaterial").and_then(Value::as_str) {
                    self.assets.load_sky(self.r, mat, tint);
                }
                true
            }
            "Bloom" => {
                let c = parse_color(p.get("Tint"), [1.0; 4]);
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                scene.add_component(go, sbox_scene::Bloom { strength: f("Strength", 1.0), threshold: f("Threshold", 1.0), tint: [c[0], c[1], c[2]] });
                true
            }
            "Tonemapping" => {
                let mode = match p.get("Mode").and_then(Value::as_str) {
                    Some("AgX") => sbox_scene::TonemapMode::AgX,
                    Some("Reinhard") => sbox_scene::TonemapMode::Reinhard,
                    _ => sbox_scene::TonemapMode::Aces,
                };
                let ev = p.get("ExposureCompensation").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                scene.add_component(
                    go,
                    sbox_scene::Tonemapping {
                        mode,
                        exposure_compensation: ev,
                        // Opt-in: the exposure range semantics are unverified against s&box, and enabling it visibly darkens scenes.
                        auto_exposure: std::env::args().any(|a| a == "--auto-exposure") && p.get("AutoExposureEnabled").and_then(Value::as_bool).unwrap_or(false),
                        min_exposure: f("MinimumExposure", 0.25),
                        max_exposure: f("MaximumExposure", 1.0),
                    },
                );
                true
            }
            "Sharpen" => {
                scene.add_component(go, sbox_scene::Sharpen { scale: p.get("Scale").and_then(Value::as_f64).unwrap_or(0.25) as f32 });
                true
            }
            "DepthOfField" => {
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                let b = |k: &str, d: bool| p.get(k).and_then(Value::as_bool).unwrap_or(d);
                scene.add_component(
                    go,
                    sbox_scene::DepthOfField { focal_distance: f("FocalDistance", 100.0), focus_range: f("FocusRange", 100.0), blur_size: f("BlurSize", 8.0), front_blur: b("FrontBlur", false), back_blur: b("BackBlur", true) },
                );
                true
            }
            "AmbientOcclusion" => {
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                scene.add_component(go, sbox_scene::AmbientOcclusion { radius: f("Radius", 128.0), intensity: f("Intensity", 1.0) });
                true
            }
            "ChromaticAberration" => {
                let o = parse_vec3(p.get("Offset"), Vec3::new(4.0, 6.0, 0.0));
                scene.add_component(go, sbox_scene::ChromaticAberration { offset: [o.x, o.y] });
                true
            }
            "GradientFog" => {
                let c = parse_color(p.get("Color"), [0.5, 0.6, 0.7, 1.0]);
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                scene.add_component(
                    go,
                    sbox_scene::GradientFog {
                        color: [c[0], c[1], c[2]],
                        start_distance: f("StartDistance", 0.0),
                        end_distance: f("EndDistance", 4096.0),
                        falloff_exponent: f("FalloffExponent", 1.0),
                        height: f("Height", 1000.0),
                        vertical_falloff_exponent: f("VerticalFalloffExponent", 1.0),
                    },
                );
                true
            }
            "CubemapFog" => {
                let t = parse_color(p.get("Tint"), [1.0; 4]);
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                scene.add_component(
                    go,
                    sbox_scene::CubemapFog {
                        tint: [t[0], t[1], t[2]],
                        blur: f("Blur", 0.5),
                        start_distance: f("StartDistance", 0.0),
                        end_distance: f("EndDistance", 4096.0),
                        falloff_exponent: f("FalloffExponent", 1.0),
                    },
                );
                true
            }
            "Vignette" => {
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                scene.add_component(go, sbox_scene::Vignette { intensity: f("Intensity", 1.0), smoothness: f("Smoothness", 1.0), roundness: f("Roundness", 1.0) });
                true
            }
            "EnvmapProbe" => true,
            "ScreenPanel" => {
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                scene.add_component(go, sbox_scene::ScreenPanel { opacity: f("Opacity", 1.0), scale: f("Scale", 1.0).max(0.001), z_index: f("ZIndex", 100.0) as i32 });
                true
            }
            "WorldPanel" => {
                let size = p.get("PanelSize").map(|v| parse_vec3(Some(v), Vec3::new(512.0, 512.0, 0.0))).unwrap_or(Vec3::new(512.0, 512.0, 0.0));
                scene.add_component(
                    go,
                    sbox_scene::WorldPanel {
                        panel_size: [size.x, size.y],
                        render_scale: p.get("RenderScale").and_then(Value::as_f64).unwrap_or(1.0) as f32,
                        look_at_camera: p.get("LookAtCamera").and_then(Value::as_bool).unwrap_or(false),
                    },
                );
                true
            }
            "DirectionalLight" => {
                let c = parse_color(p.get("LightColor"), [1.0; 4]);
                let a = parse_color(p.get("SkyColor"), [0.15, 0.17, 0.2, 1.0]);
                scene.add_component(go, DirectionalLight { color: [c[0], c[1], c[2]], ambient: [a[0], a[1], a[2]] });
                true
            }
            "SpotLight" => {
                let c = parse_color(p.get("LightColor"), [1.0; 4]);
                let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
                let outer = f("ConeOuter", 45.0);
                scene.add_component(
                    go,
                    sbox_scene::SpotLight { color: [c[0], c[1], c[2]], radius: f("Radius", 400.0), intensity: 1.0, cone_inner: f("ConeInner", 15.0).min(outer), cone_outer: outer },
                );
                true
            }
            "PointLight" => {
                let c = parse_color(p.get("LightColor"), [1.0; 4]);
                let radius = p.get("Radius").and_then(Value::as_f64).unwrap_or(400.0) as f32;
                scene.add_component(go, PointLight { color: [c[0], c[1], c[2]], radius, intensity: 1.0 });
                true
            }
            "CameraComponent" => {
                let mut c = CameraComponent::default();
                if let Some(v) = p.get("FieldOfView").and_then(Value::as_f64) {
                    c.field_of_view = v as f32;
                }
                if let Some(v) = p.get("ZNear").and_then(Value::as_f64) {
                    c.z_near = v as f32;
                }
                if let Some(v) = p.get("ZFar").and_then(Value::as_f64) {
                    c.z_far = v as f32;
                }
                let bg = parse_color(p.get("BackgroundColor"), c.background);
                c.background = bg;
                scene.add_component(go, c);
                self.camera.get_or_insert(go);
                true
            }
            "Rigidbody" => {
                let flag = |k: &str, d: bool| p.get(k).and_then(Value::as_bool).unwrap_or(d);
                scene.add_component(
                    go,
                    Rigidbody {
                        gravity: flag("Gravity", true),
                        motion_enabled: flag("MotionEnabled", true),
                        mass_override: p.get("MassOverride").and_then(Value::as_f64).filter(|m| *m > 0.0).map(|m| m as f32),
                        kinematic: false,
                    },
                );
                true
            }
            "BoxCollider" => {
                let center = parse_vec3(p.get("Center"), Vec3::ZERO);
                let size = parse_vec3(p.get("Scale"), Vec3::splat(50.0));
                let trig = p.get("IsTrigger").and_then(Value::as_bool).unwrap_or(false);
                scene.add_component(go, Collider { shape: Shape::Box { center, size }, is_trigger: trig });
                true
            }
            "SphereCollider" => {
                let center = parse_vec3(p.get("Center"), Vec3::ZERO);
                let radius = p.get("Radius").and_then(Value::as_f64).unwrap_or(25.0) as f32;
                let trig = p.get("IsTrigger").and_then(Value::as_bool).unwrap_or(false);
                scene.add_component(go, Collider { shape: Shape::Sphere { center, radius }, is_trigger: trig });
                true
            }
            "Decal" => {
                // Approximation: a camera-independent alpha-blended quad facing +X, without projection onto surfaces.
                let Some(name) = p.get("Decals").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str) else { return true };
                let Some(fs) = &self.assets.fs else { return true };
                let Some(data) = fs.read(name) else { return true };
                // The compiled decal references its colour texture in the resource reference list.
                let Some(tex_path) = find_vtex_path(&data) else { return true };
                let Some(tex) = fs.read(&tex_path).and_then(|d| vtex::parse(&d).ok()) else { return true };
                let Some(id) = self.r.upload_texture(&tex) else { return true };
                let tint = match p.get("ColorTint") {
                    Some(Value::Object(o)) => parse_color(o.get("ConstantA"), [1.0; 4]),
                    other => parse_color(other, [1.0; 4]),
                };
                let mat = self.r.create_material_ex(&MaterialDesc { color: Some(id), tint, unlit: true, translucent: true, ..Default::default() });
                let scale = p.get("Scale").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                let (hw, hh) = (tex.width as f32 * scale * 0.5, tex.height as f32 * scale * 0.5);
                let v = |y: f32, z: f32, u: f32, vv: f32| sbox_render::Vertex { position: [0.0, y, z], normal: [1.0, 0.0, 0.0], uv: [u, vv], joints: [0; 4], weights: [0.0; 4] };
                let mesh = self.r.upload_mesh(&MeshData {
                    vertices: vec![v(hw, hh, 0.0, 0.0), v(-hw, hh, 1.0, 0.0), v(-hw, -hh, 1.0, 1.0), v(hw, -hh, 0.0, 1.0)],
                    indices: vec![0, 1, 2, 0, 2, 3],
                });
                let mut m = ModelRenderer::new(mesh);
                m.material = mat;
                m.cast_shadows = false;
                scene.add_component(go, m);
                true
            }
            "MeshComponent" => {
                // Editor polygon mesh: triangulated per material slot; its collision shape is not created.
                if p.get("HideInGame").and_then(Value::as_bool) == Some(true) {
                    return true;
                }
                let groups = match crate::meshcomp::build(p) {
                    Ok(g) => g,
                    Err(e) => {
                        log::warn!("MeshComponent: {e:#}");
                        return true;
                    }
                };
                let materials: Vec<String> = p
                    .get("Mesh")
                    .and_then(|m| m.get("Materials"))
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let tint = parse_color(p.get("Color"), [1.0; 4]);
                for (slot, md) in groups {
                    let mesh = self.r.upload_mesh(&md);
                    let child = scene.create_object_in(Some(go), "mesh");
                    let mut m = ModelRenderer::new(mesh);
                    m.tint = tint;
                    if let Some(path) = materials.get(slot) {
                        m.material = self.assets.material(self.r, path);
                    }
                    scene.add_component(child, m);
                }
                true
            }
            "ModelCollider" => {
                let Some(path) = p.get("Model").and_then(Value::as_str) else { return true };
                let Some(m) = self.assets.load_model(self.r, path) else { return true };
                let trig = p.get("IsTrigger").and_then(Value::as_bool).unwrap_or(false);
                scene.add_component(go, Collider { shape: m.collision_shape(), is_trigger: trig });
                true
            }
            "PlaneCollider" => {
                scene.add_component(go, Collider { shape: Shape::Plane, is_trigger: false });
                true
            }
            _ => false,
        }
    }
}

impl Assets {
    /// Instantiate the world nodes of a compiled map (`maps/<name>/world.vwrld_c`) as model objects.
    /// Returns the number of objects spawned, the world bounds and what the map entities provided.
    pub fn load_map(&mut self, scene: &mut Scene, r: &mut Renderer, world_path: &str) -> (usize, Vec3, Vec3, MapEntities) {
        use sbox_resource::{kv3, Resource, Value};
        let (mut mins, mut maxs) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let Some(fs) = &self.fs else { return (0, mins, maxs, MapEntities::default()) };
        let kv_of = |p: &str| -> Option<Value> {
            let d = fs.read(p)?;
            let res = Resource::parse(&d).ok()?;
            kv3::parse(res.block_data("DATA")?).ok()
        };
        let Some(world) = kv_of(world_path) else {
            log::warn!("map not found: {world_path}");
            return (0, mins, maxs, MapEntities::default());
        };
        let lumps: Vec<String> = world.get("m_entityLumps").and_then(Value::as_array).unwrap_or(&[]).iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let prefixes: Vec<String> = world
            .get("m_worldNodes")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|n| n.get("m_worldNodePrefix").and_then(Value::as_str).map(|p| format!("{p}.vwnod")))
            .collect();
        let mut placements = Vec::new();
        for node_path in prefixes {
            let Some(node) = kv_of(&node_path) else { continue };
            let objs = node.get("m_sceneObjects").and_then(Value::as_array).unwrap_or(&[]);
            // Lightmapped duplicates of a mesh carry a cubemap handshake; prefer the plain variant.
            let has_plain = objs.iter().any(|o| o.get("m_nCubeMapPrecomputedHandshake").and_then(Value::as_i64) == Some(0));
            for o in objs {
                if has_plain && o.get("m_nCubeMapPrecomputedHandshake").and_then(Value::as_i64) != Some(0) {
                    continue;
                }
                let Some(model) = o.get("m_renderableModel").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue };
                let rows = o.get("m_vTransform").and_then(Value::as_array).unwrap_or(&[]);
                let row = |i: usize| -> [f32; 4] {
                    let a = rows.get(i).and_then(Value::as_array).unwrap_or(&[]);
                    std::array::from_fn(|j| a.get(j).and_then(Value::as_f64).unwrap_or(if i == j { 1.0 } else { 0.0 }) as f32)
                };
                let (r0, r1, r2) = (row(0), row(1), row(2));
                // Rows of a 3x4 matrix: [axis-ish | translation].
                let m = sbox_math::Mat4::from_cols(
                    sbox_math::Vec4::new(r0[0], r1[0], r2[0], 0.0),
                    sbox_math::Vec4::new(r0[1], r1[1], r2[1], 0.0),
                    sbox_math::Vec4::new(r0[2], r1[2], r2[2], 0.0),
                    sbox_math::Vec4::new(r0[3], r1[3], r2[3], 1.0),
                );
                let (scale, rotation, position) = m.to_scale_rotation_translation();
                placements.push((model.to_string(), Transform::new(position, rotation, scale)));
            }
        }
        let mut count = 0;
        for (model, t) in placements {
            let Some(m) = self.load_model(r, &model) else { continue };
            let go = scene.create_object("map");
            scene.set_local_transform(go, t);
            self.spawn(scene, go, &m, [1.0; 4], None);
            for c in [m.mins, m.maxs] {
                let p = t.point_to_world(c);
                mins = mins.min(p);
                maxs = maxs.max(p);
            }
            count += 1;
        }
        let mut info = MapEntities::default();
        for lump in lumps {
            let Some(bytes) = self.fs.as_ref().and_then(|fs| fs.read(&format!("{lump}"))) else { continue };
            match sbox_resource::vents::load(&bytes) {
                Ok(ents) => {
                    for e in &ents {
                        self.spawn_entity(scene, r, e, &mut info);
                    }
                }
                Err(e) => log::warn!("entity lump {lump}: {e:#}"),
            }
        }
        (count, mins, maxs, info)
    }

    /// Instantiate one map entity. Unknown classes are ignored.
    fn spawn_entity(&mut self, scene: &mut Scene, r: &mut Renderer, e: &sbox_resource::vents::Entity, info: &mut MapEntities) {
        use sbox_math::{Angles, Quat};
        let [ox, oy, oz] = e.origin();
        let [pitch, yaw, roll] = e.angles();
        let rotation = Angles::new(pitch, yaw, roll).to_rotation();
        let [sx, sy, sz] = e.scales();
        let transform = Transform::new(Vec3::new(ox, oy, oz), rotation, Vec3::new(sx, sy, sz));
        let rgb = |e: &sbox_resource::vents::Entity| {
            let c = e.color("color").unwrap_or([255; 4]);
            let b = e.float("brightness").unwrap_or(1.0);
            [c[0] as f32 / 255.0 * b, c[1] as f32 / 255.0 * b, c[2] as f32 / 255.0 * b]
        };
        if e.bool("enabled") == Some(false) {
            return;
        }
        match e.classname() {
            "light_environment" => {
                let go = scene.create_object("light_environment");
                scene.set_local_transform(go, Transform::new(Vec3::ZERO, rotation, Vec3::ONE));
                scene.add_component(go, DirectionalLight { color: rgb(e), ..Default::default() });
                info.has_sun = true;
            }
            "light_omni" | "light_omni2" => {
                let go = scene.create_object("light_omni");
                scene.set_local_transform(go, Transform::from_position(transform.position));
                scene.add_component(go, PointLight { color: rgb(e), radius: e.float("range").unwrap_or(512.0), intensity: 1.0 });
            }
            "light_spot" => {
                let go = scene.create_object("light_spot");
                scene.set_local_transform(go, Transform::new(transform.position, rotation, Vec3::ONE));
                let outer = e.float("outerconeangle").unwrap_or(45.0);
                scene.add_component(
                    go,
                    sbox_scene::SpotLight { color: rgb(e), radius: e.float("range").unwrap_or(512.0), intensity: 1.0, cone_inner: e.float("innerconeangle").unwrap_or(30.0).min(outer), cone_outer: outer },
                );
            }
            "light_ortho" => {
                // Approximation: an orthographic (parallel-beam) light is drawn as a wide spot light.
                let go = scene.create_object("light_ortho");
                scene.set_local_transform(go, Transform::new(transform.position, rotation, Vec3::ONE));
                scene.add_component(go, sbox_scene::SpotLight { color: rgb(e), radius: e.float("range").unwrap_or(512.0), intensity: 1.0, cone_inner: 50.0, cone_outer: 70.0 });
            }
            "env_sky" => {
                if let Some(mat) = e.string("skyname").filter(|s| !s.is_empty()) {
                    let t = e.color("tint_color").unwrap_or([255; 4]);
                    self.load_sky(r, mat, [t[0] as f32 / 255.0, t[1] as f32 / 255.0, t[2] as f32 / 255.0, 1.0]);
                }
            }
            "env_volumetric_fog_volume" => {
                let go = scene.create_object("fog_volume");
                scene.set_local_transform(go, Transform::from_position(transform.position));
                scene.add_component(
                    go,
                    sbox_scene::FogVolume {
                        mins: e.vec3("box_mins").unwrap_or([-64.0; 3]),
                        maxs: e.vec3("box_maxs").unwrap_or([64.0; 3]),
                        strength: e.float("fogstrength").unwrap_or(1.0),
                    },
                );
            }
            "info_player_start" => {
                if info.player_start.is_none() {
                    info.player_start = Some((transform.position, yaw));
                }
            }
            "prop_physics" | "prop_dynamic" | "prop_static" | "func_shatterglass" => {
                let Some(model) = e.string("model").filter(|s| !s.is_empty()) else { return };
                let Some(m) = self.load_model(r, &model.replace('\\', "/")) else { return };
                let go = scene.create_object("prop");
                scene.set_local_transform(go, transform);
                self.spawn(scene, go, &m, [1.0; 4], None);
                // Only `prop_physics` moves.
                scene.add_component(go, Collider { shape: m.collision_shape(), is_trigger: false });
                if e.classname() == "prop_physics" && e.bool("motiondisabled") != Some(true) {
                    scene.add_component(go, Rigidbody::default());
                }
                info.props += 1;
            }
            _ => {}
        }
        let _ = Quat::IDENTITY;
    }
}

/// What the entity lump of a map provided.
#[derive(Default)]
pub struct MapEntities {
    pub has_sun: bool,
    pub player_start: Option<(Vec3, f32)>,
    pub props: usize,
}

/// First `*.vtex` path mentioned in a compiled resource's reference list.
fn find_vtex_path(data: &[u8]) -> Option<String> {
    let end = data.windows(5).position(|w| w == b".vtex")? + 5;
    let start = data[..end].iter().rposition(|&b| b == 0 || b < 32).map_or(0, |i| i + 1);
    std::str::from_utf8(&data[start..end]).ok().map(|p| p.replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sbox_resource::phys::{PhysJoint, PhysPart, PhysShape};

    #[test]
    fn a_ragdoll_falls_as_a_connected_chain() {
        let mut scene = Scene::new();
        sbox_physics::Physics::install(&mut scene);
        let ground = scene.create_object("ground");
        scene.add_component(ground, Collider { shape: Shape::Plane, is_trigger: false });
        // Three capsule parts stacked 40 units apart, each joined to the next at the shared point; both joints come from the
        // middle part, so one of them hangs off a helper object.
        let part = |z: f32| PhysPart { shapes: vec![PhysShape::Capsule { a: [0.0, 0.0, -10.0], b: [0.0, 0.0, 10.0], radius: 5.0 }], position: [0.0, 0.0, z], rotation: [0.0, 0.0, 0.0, 1.0], mass: 5.0 };
        let frame = |z: f32| ([0.0, 0.0, z], [0.0, 0.0, 0.0, 1.0]);
        let joint = |a: usize, b: usize| PhysJoint { body1: a, body2: b, frame1: frame(20.0), frame2: frame(-20.0), swing: Some(1.0), twist: Some([-0.5, 0.5]), collide: false };
        let bodies = spawn_ragdoll(&mut scene, &Transform::from_position(Vec3::new(0.0, 0.0, 300.0)), &[part(0.0), part(40.0), part(80.0)], &[joint(1, 2), joint(1, 0)]);
        assert_eq!(bodies.len(), 3);
        assert_eq!(scene.iter_components::<sbox_physics::Joint>().count(), 2);
        for _ in 0..250 {
            scene.tick(1.0 / 50.0);
        }
        let p: Vec<Vec3> = bodies.iter().map(|&b| scene.world_transform(b).position).collect();
        assert!(p.iter().all(|p| p.z < 300.0), "gravity moved every part: {p:?}");
        // The shared joint points stay together: part 1's +20 meets part 2's -20 (and part 1's +20 meets part 0's -20).
        for (a, b) in [(1usize, 2usize), (1, 0)] {
            let at = |go: sbox_scene::GoId, z: f32| {
                let t = scene.world_transform(go);
                t.position + t.rotation * Vec3::new(0.0, 0.0, z)
            };
            let (pa, pb) = (at(bodies[a], 20.0), at(bodies[b], -20.0));
            assert!((pa - pb).length() < 4.0, "joint {a}-{b} came apart by {}", (pa - pb).length());
        }
    }
}
