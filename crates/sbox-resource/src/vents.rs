//! Map entity lumps (`*.vents_c`): each entity is a binary key-values blob.
//!
//! Blob layout: `version: u32 (1)`, `hashed key count: u32`, `string key count: u32 (0 in shipped maps)`, then per entry
//! `key hash: u32`, `type: u32`, value. Keys are stored as MurmurHash2 (seed `0x31415926`) of the lowercase key name.
//! Value types seen in the shipped maps: 1 float, 3 vector, 6 bool (1 byte), 9 colour (RGBA bytes), 26 u64,
//! 30 NUL-terminated string, 39 angles (3 floats). Vector-like values authored as text (`origin`, `angles`, `scales`)
//! are strings of space-separated numbers.

use crate::{kv3, Resource, Value};
use anyhow::{anyhow, bail, ensure, Result};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum EntValue {
    Float(f32),
    Vec3([f32; 3]),
    Bool(bool),
    Color([u8; 4]),
    U64(u64),
    Str(String),
}

/// MurmurHash2 with the seed Source 2 uses for entity keys.
pub fn key_hash(name: &str) -> u32 {
    const M: u32 = 0x5bd1e995;
    let data = name.to_lowercase().into_bytes();
    let mut h = 0x31415926u32 ^ data.len() as u32;
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        let mut k = u32::from_le_bytes(c.try_into().unwrap()).wrapping_mul(M);
        k ^= k >> 24;
        h = h.wrapping_mul(M) ^ k.wrapping_mul(M);
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        for (i, &b) in rest.iter().enumerate().rev() {
            h ^= (b as u32) << (8 * i);
        }
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^ (h >> 15)
}

#[derive(Clone, Debug, Default)]
pub struct Entity {
    values: HashMap<u32, EntValue>,
}

fn nums<const N: usize>(s: &str) -> Option<[f32; N]> {
    let mut it = s.split_whitespace().map(str::parse::<f32>);
    let mut out = [0.0; N];
    for o in &mut out {
        *o = it.next()?.ok()?;
    }
    Some(out)
}

impl Entity {
    pub fn parse(b: &[u8]) -> Result<Self> {
        ensure!(b.len() >= 12, "entity blob too small");
        let u32_at = |o: usize| -> Result<u32> { Ok(u32::from_le_bytes(b.get(o..o + 4).ok_or_else(|| anyhow!("truncated entity"))?.try_into().unwrap())) };
        ensure!(u32_at(0)? == 1, "unsupported entity blob version {}", u32_at(0)?);
        let count = u32_at(4)? as usize;
        ensure!(u32_at(8)? == 0, "named (non-hashed) entity keys are not supported");
        let mut pos = 12;
        let mut values = HashMap::with_capacity(count);
        for _ in 0..count {
            let (hash, ty) = (u32_at(pos)?, u32_at(pos + 4)?);
            pos += 8;
            let take = |pos: &mut usize, n: usize| -> Result<&[u8]> {
                let s = b.get(*pos..*pos + n).ok_or_else(|| anyhow!("truncated entity value"))?;
                *pos += n;
                Ok(s)
            };
            let f32s = |s: &[u8]| -> [f32; 3] { std::array::from_fn(|i| f32::from_le_bytes(s[i * 4..i * 4 + 4].try_into().unwrap())) };
            let v = match ty {
                1 => EntValue::Float(f32::from_le_bytes(take(&mut pos, 4)?.try_into().unwrap())),
                3 | 39 => EntValue::Vec3(f32s(take(&mut pos, 12)?)),
                6 => EntValue::Bool(take(&mut pos, 1)?[0] != 0),
                9 => EntValue::Color(take(&mut pos, 4)?.try_into().unwrap()),
                26 => EntValue::U64(u64::from_le_bytes(take(&mut pos, 8)?.try_into().unwrap())),
                30 => {
                    let end = b[pos..].iter().position(|&c| c == 0).ok_or_else(|| anyhow!("unterminated entity string"))?;
                    let s = String::from_utf8_lossy(&b[pos..pos + end]).into_owned();
                    pos += end + 1;
                    EntValue::Str(s)
                }
                other => bail!("unknown entity value type {other}"),
            };
            values.insert(hash, v);
        }
        Ok(Self { values })
    }

    pub fn get(&self, key: &str) -> Option<&EntValue> {
        self.values.get(&key_hash(key))
    }
    pub fn string(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            EntValue::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn float(&self, key: &str) -> Option<f32> {
        match self.get(key)? {
            EntValue::Float(f) => Some(*f),
            EntValue::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            EntValue::Bool(b) => Some(*b),
            EntValue::Float(f) => Some(*f != 0.0),
            EntValue::U64(v) => Some(*v != 0),
            EntValue::Str(s) => Some(!matches!(s.trim(), "" | "0" | "false")),
            _ => None,
        }
    }
    /// A vector stored either as three floats or as text (`"x y z"`).
    pub fn vec3(&self, key: &str) -> Option<[f32; 3]> {
        match self.get(key)? {
            EntValue::Vec3(v) => Some(*v),
            EntValue::Str(s) => nums(s),
            _ => None,
        }
    }
    pub fn color(&self, key: &str) -> Option<[u8; 4]> {
        match self.get(key)? {
            EntValue::Color(c) => Some(*c),
            _ => None,
        }
    }
    pub fn classname(&self) -> &str {
        self.string("classname").unwrap_or("")
    }
    pub fn origin(&self) -> [f32; 3] {
        self.vec3("origin").unwrap_or([0.0; 3])
    }
    /// Pitch, yaw, roll in degrees.
    pub fn angles(&self) -> [f32; 3] {
        self.vec3("angles").unwrap_or([0.0; 3])
    }
    pub fn scales(&self) -> [f32; 3] {
        self.vec3("scales").unwrap_or([1.0; 3])
    }
}

/// Every entity of a compiled lump (`*.vents_c`).
pub fn load(file: &[u8]) -> Result<Vec<Entity>> {
    let res = Resource::parse(file)?;
    let kv = kv3::parse(res.block_data("DATA").ok_or_else(|| anyhow!("no DATA block"))?)?;
    kv.get("m_entityKeyValues")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| match e.get("m_keyValuesData") {
            Some(Value::Blob(b)) => Some(Entity::parse(b)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn key_hashes_match_known_values() {
        assert_eq!(key_hash("classname"), 0xc61b1c62);
        assert_eq!(key_hash("origin"), 0xe4200216);
        assert_eq!(key_hash("angles"), 0xba98dacf);
        assert_eq!(key_hash("Origin"), key_hash("origin"));
    }

    #[test]
    fn parses_info_player_start() {
        let blob = hex("010000000d00000000000000a9f350ed1e0000000060f1bba303000000000000000000000000000000c3297654270000000000000000000000000000003fed7e3f03000000000000000000000000000000b4cc1cdb06000000000328611e1e00000000693482451e000000006baf37411e00000000621c1bc61e000000696e666f5f706c617965725f737461727400160220e41e000000302e30303030303020302e30303030303020302e30303030303000cfda98ba1e000000302e30303030303020302e30303030303020302e303030303030002ce4c1191e000000312e30303030303020312e30303030303020312e3030303030300038a063a91e000000313700");
        let e = Entity::parse(&blob).unwrap();
        assert_eq!(e.classname(), "info_player_start");
        assert_eq!(e.origin(), [0.0, 0.0, 0.0]);
        assert_eq!(e.scales(), [1.0, 1.0, 1.0]);
        assert_eq!(e.string("targetname"), Some(""));
    }

    #[test]
    fn loads_every_entity_of_the_shipped_preview_map() {
        let Ok(v) = crate::vpk::Vpk::open("E:/sbox-ref/game/core/maps/dev/preview_flat.vpk") else { return };
        let ents = load(&v.read("maps/dev/preview_flat/entities/default_ents.vents_c").unwrap()).unwrap();
        let classes: Vec<&str> = ents.iter().map(Entity::classname).collect();
        assert!(classes.contains(&"worldspawn"), "{classes:?}");
        assert!(classes.contains(&"info_player_start"), "{classes:?}");
    }
}
