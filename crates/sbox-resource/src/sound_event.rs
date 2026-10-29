//! `.sound_c` sound events: a JSON document that picks one of several `.vsnd` files and sets volume / falloff.

use crate::Resource;
use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct SoundEvent {
    pub sounds: Vec<String>,
    pub volume: f32,
    pub pitch: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub ui: bool,
}

pub fn parse(file: &[u8]) -> Result<SoundEvent> {
    let res = Resource::parse(file)?;
    let data = res.block_data("DATA").ok_or_else(|| anyhow!("no DATA block"))?;
    let v: Value = serde_json::Deserializer::from_slice(data).into_iter::<Value>().next().ok_or_else(|| anyhow!("empty sound event"))??;
    let f = |k: &str, d: f64| v.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
    Ok(SoundEvent {
        sounds: v.get("Sounds").and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default(),
        volume: f("Volume", 1.0),
        pitch: f("Pitch", 1.0),
        min_distance: f("MinimumDistance", 0.0),
        max_distance: f("MaximumDistance", 1000.0),
        ui: v.get("UI").and_then(Value::as_bool).unwrap_or(false),
    })
}

impl SoundEvent {
    /// Gain at `distance` world units: full inside `min_distance`, linear to silence at `max_distance`.
    pub fn attenuation(&self, distance: f32) -> f32 {
        if self.ui || distance <= self.min_distance {
            1.0
        } else if distance >= self.max_distance {
            0.0
        } else {
            1.0 - (distance - self.min_distance) / (self.max_distance - self.min_distance).max(1e-3)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuation_is_monotonic_and_bounded() {
        let e = SoundEvent { sounds: vec![], volume: 1.0, pitch: 1.0, min_distance: 100.0, max_distance: 500.0, ui: false };
        assert_eq!(e.attenuation(50.0), 1.0);
        assert!((e.attenuation(300.0) - 0.5).abs() < 1e-5);
        assert_eq!(e.attenuation(900.0), 0.0);
    }

    #[test]
    fn real_event_parses() {
        let Ok(b) = std::fs::read("E:/sbox-ref/game/core/sounds/ambience/stings/sting-coyote-sting.sound_c") else { return };
        let e = parse(&b).unwrap();
        assert_eq!(e.sounds.len(), 3);
        assert!(e.max_distance > 1800.0);
    }
}
