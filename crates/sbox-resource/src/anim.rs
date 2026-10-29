//! `.anim_c` animation clips.
//!
//! Each track (one per bone) has compression settings. Static channels use a constant
//! (`m_constantRotation`, translation range start, scale range start); animated channels are
//! stored per frame in `m_compressedPoseData` as u16 values:
//! translation = 3 x u16 (range-normalised), rotation = 3 x u16 (48-bit smallest-three
//! quaternion), scale = 1 x u16.

use crate::{kv3, Resource, Value};
use anyhow::{anyhow, ensure, Result};

#[derive(Clone, Debug)]
pub struct Track {
    pub translation_start: [f32; 3],
    pub translation_len: [f32; 3],
    pub scale_start: f32,
    pub scale_len: f32,
    pub constant_rotation: [f32; 4],
    pub rotation_static: bool,
    pub translation_static: bool,
    pub scale_static: bool,
}

#[derive(Clone, Debug)]
pub struct Animation {
    pub skeleton: String,
    pub frames: usize,
    pub duration: f32,
    pub additive: bool,
    pub tracks: Vec<Track>,
    data: Vec<u16>,
    offsets: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
pub struct BonePose {
    pub position: [f32; 3],
    /// x, y, z, w
    pub rotation: [f32; 4],
    pub scale: f32,
}

fn range(v: Option<&Value>) -> (f32, f32) {
    let g = |k: &str| v.and_then(|v| v.get(k)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    (g("m_flRangeStart"), g("m_flRangeLength"))
}

fn flag(v: &Value, k: &str) -> bool {
    matches!(v.get(k), Some(Value::Bool(true)))
}

pub fn parse(file: &[u8]) -> Result<Animation> {
    let res = Resource::parse(file)?;
    let kv = kv3::parse(res.block_data("DATA").ok_or_else(|| anyhow!("no DATA"))?)?;
    let frames = kv.get("m_nNumFrames").and_then(Value::as_i64).unwrap_or(0) as usize;
    let mut tracks = Vec::new();
    for t in kv.get("m_trackCompressionSettings").and_then(Value::as_array).unwrap_or(&[]) {
        let (x0, xl) = range(t.get("m_translationRangeX"));
        let (y0, yl) = range(t.get("m_translationRangeY"));
        let (z0, zl) = range(t.get("m_translationRangeZ"));
        let (s0, sl) = range(t.get("m_scaleRange"));
        let q = t.get("m_constantRotation").and_then(Value::as_array).unwrap_or(&[]);
        let g = |i: usize, d: f32| q.get(i).and_then(Value::as_f64).map_or(d, |v| v as f32);
        tracks.push(Track {
            translation_start: [x0, y0, z0],
            translation_len: [xl, yl, zl],
            scale_start: s0,
            scale_len: sl,
            constant_rotation: [g(0, 0.0), g(1, 0.0), g(2, 0.0), g(3, 1.0)],
            rotation_static: flag(t, "m_bIsRotationStatic"),
            translation_static: flag(t, "m_bIsTranslationStatic"),
            scale_static: flag(t, "m_bIsScaleStatic"),
        });
    }
    let data: Vec<u16> =
        kv.get("m_compressedPoseData").and_then(Value::as_array).unwrap_or(&[]).iter().map(|v| v.as_i64().unwrap_or(0) as u16).collect();
    let offsets: Vec<usize> =
        kv.get("m_compressedPoseOffsets").and_then(Value::as_array).unwrap_or(&[]).iter().map(|v| v.as_i64().unwrap_or(0) as usize).collect();
    let per_frame: usize = tracks
        .iter()
        .map(|t| (!t.translation_static) as usize * 3 + (!t.rotation_static) as usize * 3 + (!t.scale_static) as usize)
        .sum();
    if per_frame > 0 {
        ensure!(offsets.len() >= frames, "pose offsets {} < frames {frames}", offsets.len());
        ensure!(data.len() >= frames * per_frame, "pose data too short: {} < {}", data.len(), frames * per_frame);
    }
    Ok(Animation {
        skeleton: kv.get("m_skeleton").and_then(Value::as_str).unwrap_or("").to_string(),
        frames,
        duration: kv.get("m_flDuration").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        additive: matches!(kv.get("m_bIsAdditive"), Some(Value::Bool(true))),
        tracks,
        data,
        offsets,
    })
}

/// 48-bit smallest-three quaternion: three 15-bit components; the top bits of the words
/// hold the index of the omitted (largest) component and its sign.
pub fn decode_quat48(a: u16, b: u16, c: u16) -> [f32; 4] {
    const S: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let dec = |v: u16| ((v & 0x7fff) as f32 - 16384.0) / 16384.0 * S;
    let (x, y, z) = (dec(a), dec(b), dec(c));
    let idx = ((((a >> 15) & 1) << 1) | ((b >> 15) & 1)) as usize;
    let negative = (c >> 15) & 1 == 1;
    let mut w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
    if negative {
        w = -w;
    }
    match idx {
        0 => [w, x, y, z],
        1 => [x, w, y, z],
        2 => [x, y, w, z],
        _ => [x, y, z, w],
    }
}

impl Animation {
    /// Raw u16 words of one frame block (debugging aid).
    pub fn raw_frame(&self, frame: usize) -> &[u16] {
        let o = self.offsets.get(frame).copied().unwrap_or(0);
        &self.data[o.min(self.data.len())..]
    }

    pub fn fps(&self) -> f32 {
        if self.duration > 0.0 { self.frames as f32 / self.duration } else { 30.0 }
    }

    /// Sample every track at a whole frame index.
    pub fn sample_frame(&self, frame: usize) -> Vec<BonePose> {
        let frame = frame.min(self.frames.saturating_sub(1));
        let mut pos = self.offsets.get(frame).copied().unwrap_or(0);
        let mut out = Vec::with_capacity(self.tracks.len());
        let mut next = |data: &[u16]| {
            let v = data.get(pos).copied().unwrap_or(0);
            pos += 1;
            v
        };
        for t in &self.tracks {
            let rotation = if t.rotation_static {
                t.constant_rotation
            } else {
                let (a, b, c) = (next(&self.data), next(&self.data), next(&self.data));
                decode_quat48(a, b, c)
            };
            let position = if t.translation_static {
                t.translation_start
            } else {
                let mut p = [0.0; 3];
                for i in 0..3 {
                    p[i] = t.translation_start[i] + next(&self.data) as f32 / 65535.0 * t.translation_len[i];
                }
                p
            };
            let scale = if t.scale_static { t.scale_start } else { t.scale_start + next(&self.data) as f32 / 65535.0 * t.scale_len };
            out.push(BonePose { position, rotation, scale });
        }
        out
    }

    /// Linear interpolation between frames at time `t` seconds (looping).
    pub fn sample(&self, t: f32) -> Vec<BonePose> {
        if self.frames <= 1 || self.duration <= 0.0 {
            return self.sample_frame(0);
        }
        let f = (t / self.duration).rem_euclid(1.0) * (self.frames - 1) as f32;
        let (i, frac) = (f.floor() as usize, f.fract());
        let (a, b) = (self.sample_frame(i), self.sample_frame(i + 1));
        a.iter()
            .zip(&b)
            .map(|(a, b)| {
                let dot: f32 = (0..4).map(|k| a.rotation[k] * b.rotation[k]).sum();
                let sg = if dot < 0.0 { -1.0 } else { 1.0 };
                let mut r = [0.0; 4];
                for k in 0..4 {
                    r[k] = a.rotation[k] + (b.rotation[k] * sg - a.rotation[k]) * frac;
                }
                let l = r.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
                BonePose {
                    position: std::array::from_fn(|k| a.position[k] + (b.position[k] - a.position[k]) * frac),
                    rotation: r.map(|v| v / l),
                    scale: a.scale + (b.scale - a.scale) * frac,
                }
            })
            .collect()
    }
}
