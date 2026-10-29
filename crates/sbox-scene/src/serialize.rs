//! Loading of s&box `.scene` / `.prefab` JSON (see GameObject.Serialize.cs).
//!
//! ```json
//! { "GameObjects": [ { "Name": "Sun", "Position": "x,y,z", "Rotation": "x,y,z,w",
//!     "Scale": "x,y,z", "Enabled": true, "Tags": "a,b",
//!     "Components": [ { "__type": "Sandbox.DirectionalLight", ... } ], "Children": [ ... ] } ] }
//! ```

use crate::{GoId, Scene};
use anyhow::{anyhow, Result};
use sbox_math::{Quat, Transform, Vec3};
use serde_json::Value;

/// Creates components for the JSON `__type` names the embedding application understands.
pub trait ComponentLoader {
    /// Called for every component entry. Unknown types should be ignored (return `false`).
    fn create(&mut self, scene: &mut Scene, go: GoId, type_name: &str, props: &Value) -> bool;
}

fn floats(v: &Value) -> Vec<f32> {
    v.as_str().map(|s| s.split(',').filter_map(|p| p.trim().parse::<f32>().ok()).collect()).unwrap_or_default()
}

pub fn parse_vec3(v: Option<&Value>, default: Vec3) -> Vec3 {
    match v.map(floats) {
        Some(f) if f.len() == 3 => Vec3::new(f[0], f[1], f[2]),
        _ => default,
    }
}

pub fn parse_quat(v: Option<&Value>) -> Quat {
    match v.map(floats) {
        Some(f) if f.len() == 4 => Quat::from_xyzw(f[0], f[1], f[2], f[3]).normalize(),
        _ => Quat::IDENTITY,
    }
}

/// Parses "r,g,b,a" colours.
pub fn parse_color(v: Option<&Value>, default: [f32; 4]) -> [f32; 4] {
    match v.map(floats) {
        Some(f) if f.len() >= 3 => [f[0], f[1], f[2], f.get(3).copied().unwrap_or(1.0)],
        _ => default,
    }
}

#[derive(Debug, Default)]
pub struct LoadStats {
    pub objects: usize,
    pub components: usize,
    pub unknown_components: std::collections::BTreeMap<String, usize>,
}

fn load_object(scene: &mut Scene, parent: Option<GoId>, j: &Value, loader: &mut dyn ComponentLoader, stats: &mut LoadStats) -> GoId {
    let name = j.get("Name").and_then(Value::as_str).unwrap_or("Object");
    let go = scene.create_object_in(parent, name);
    stats.objects += 1;
    scene.set_local_transform(
        go,
        Transform::new(parse_vec3(j.get("Position"), Vec3::ZERO), parse_quat(j.get("Rotation")), parse_vec3(j.get("Scale"), Vec3::ONE)),
    );
    if let Some(tags) = j.get("Tags").and_then(Value::as_str) {
        if let Some(g) = scene.get_mut(go) {
            g.tags.extend(tags.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()));
        }
    }
    for c in j.get("Components").and_then(Value::as_array).into_iter().flatten() {
        let Some(ty) = c.get("__type").and_then(Value::as_str) else { continue };
        if loader.create(scene, go, ty, c) {
            stats.components += 1;
        } else {
            *stats.unknown_components.entry(ty.to_string()).or_default() += 1;
        }
    }
    for child in j.get("Children").and_then(Value::as_array).into_iter().flatten() {
        load_object(scene, Some(go), child, loader, stats);
    }
    if j.get("Enabled").and_then(Value::as_bool) == Some(false) {
        scene.set_enabled(go, false);
    }
    go
}

/// Load a scene JSON document into `scene`, returning root object ids and statistics.
pub fn load_scene_json(scene: &mut Scene, json: &str, loader: &mut dyn ComponentLoader) -> Result<(Vec<GoId>, LoadStats)> {
    // Compiled files may carry trailing bytes after the JSON document: read the first value only.
    let doc: Value = serde_json::Deserializer::from_str(json)
        .into_iter::<Value>()
        .next()
        .ok_or_else(|| anyhow!("scene json: empty"))?
        .map_err(|e| anyhow!("scene json: {e}"))?;
    let mut stats = LoadStats::default();
    let mut roots = Vec::new();
    for o in doc.get("GameObjects").and_then(Value::as_array).into_iter().flatten() {
        roots.push(load_object(scene, None, o, loader, &mut stats));
    }
    // A prefab file has a single "RootObject".
    if let Some(o) = doc.get("RootObject") {
        roots.push(load_object(scene, None, o, loader, &mut stats));
    }
    Ok((roots, stats))
}
