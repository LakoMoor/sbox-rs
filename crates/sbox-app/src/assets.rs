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
        r.create_material_ex(&MaterialDesc {
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
        })
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
            "Bloom" | "EnvmapProbe" => true,
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
    /// Returns the number of objects spawned and the world bounds.
    pub fn load_map(&mut self, scene: &mut Scene, r: &mut Renderer, world_path: &str) -> (usize, Vec3, Vec3) {
        use sbox_resource::{kv3, Resource, Value};
        let (mut mins, mut maxs) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let Some(fs) = &self.fs else { return (0, mins, maxs) };
        let kv_of = |p: &str| -> Option<Value> {
            let d = fs.read(p)?;
            let res = Resource::parse(&d).ok()?;
            kv3::parse(res.block_data("DATA")?).ok()
        };
        let Some(world) = kv_of(world_path) else {
            log::warn!("map not found: {world_path}");
            return (0, mins, maxs);
        };
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
        (count, mins, maxs)
    }
}
