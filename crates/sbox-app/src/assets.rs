//! Asset cache and the scene-file component registry.

use sbox_math::Vec3;
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
            unlit: false,
            translucent: false,
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
        let loaded = match model::load(bytes) {
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
            "PointLight" | "SpotLight" => {
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
