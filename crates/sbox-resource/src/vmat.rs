//! `.vmat_c` material loader and a minimal virtual file system over the game directories.

use crate::{kv3, Resource, Value};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Material {
    pub name: String,
    pub shader: String,
    pub floats: HashMap<String, f32>,
    pub vectors: HashMap<String, [f32; 4]>,
    /// Parameter name (e.g. `g_tColor`) -> virtual path of the `.vtex`.
    pub textures: HashMap<String, String>,
}

impl Material {
    pub fn color_tint(&self) -> [f32; 4] {
        let t = self.vectors.get("g_vColorTint").copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
        [t[0], t[1], t[2], 1.0]
    }
    pub fn uv_scale(&self) -> [f32; 2] {
        self.vectors.get("g_vTexCoordScale").map_or([1.0, 1.0], |v| [v[0], v[1]])
    }
    /// Texture of a sky material (`SkyTexture`), falling back to the first texture parameter.
    pub fn sky_texture(&self) -> Option<&str> {
        self.textures
            .get("SkyTexture")
            .or_else(|| self.textures.get("g_tSkyTexture"))
            .or_else(|| self.textures.values().next())
            .map(String::as_str)
    }
    pub fn color_texture(&self) -> Option<&str> {
        self.textures.get("g_tColor").map(String::as_str)
    }
}

fn vec4(v: &Value) -> [f32; 4] {
    let a = v.as_array().unwrap_or(&[]);
    let g = |i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [g(0), g(1), g(2), g(3)]
}

pub fn parse(file: &[u8]) -> Result<Material> {
    let res = Resource::parse(file)?;
    let kv = kv3::parse(res.block_data("DATA").ok_or_else(|| anyhow!("no DATA"))?)?;
    let mut m = Material {
        name: kv.get("m_materialName").and_then(Value::as_str).unwrap_or("").to_string(),
        shader: kv.get("m_shaderName").and_then(Value::as_str).unwrap_or("").to_string(),
        ..Default::default()
    };
    let list = |key: &str| kv.get(key).and_then(Value::as_array).map(<[Value]>::to_vec).unwrap_or_default();
    for p in list("m_floatParams") {
        if let (Some(n), Some(v)) = (p.get("m_name").and_then(Value::as_str), p.get("m_flValue").and_then(Value::as_f64)) {
            m.floats.insert(n.to_string(), v as f32);
        }
    }
    for p in list("m_vectorParams") {
        if let (Some(n), Some(v)) = (p.get("m_name").and_then(Value::as_str), p.get("m_value")) {
            m.vectors.insert(n.to_string(), vec4(v));
        }
    }
    for p in list("m_textureParams") {
        if let (Some(n), Some(v)) = (p.get("m_name").and_then(Value::as_str), p.get("m_pValue").and_then(Value::as_str)) {
            m.textures.insert(n.to_string(), v.to_string());
        }
    }
    Ok(m)
}

/// Resolves virtual paths (`materials/dev/red.vmat`) against a list of root directories,
/// appending the `_c` suffix of compiled files.
pub struct GameFs {
    roots: Vec<PathBuf>,
}

impl GameFs {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// Roots for a standard `game/` directory: `core` plus every `addons/*/Assets` and `addons/*`.
    pub fn from_game_dir(game: impl Into<PathBuf>) -> Self {
        let game = game.into();
        let mut roots = vec![game.join("core")];
        if let Ok(rd) = std::fs::read_dir(game.join("addons")) {
            for e in rd.flatten() {
                let p = e.path();
                if p.join("Assets").is_dir() {
                    roots.push(p.join("Assets"));
                }
                roots.push(p);
            }
        }
        Self { roots }
    }

    pub fn read(&self, virtual_path: &str) -> Option<Vec<u8>> {
        let rel = virtual_path.trim_start_matches('/');
        let compiled = if rel.ends_with("_c") { rel.to_string() } else { format!("{rel}_c") };
        self.roots.iter().find_map(|r| std::fs::read(r.join(&compiled)).ok())
    }
}
