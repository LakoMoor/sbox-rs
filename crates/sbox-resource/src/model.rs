//! `.vmdl_c` models: skeleton + embedded meshes (one MDAT/MBUF pair per mesh).

use crate::anim::BonePose;
use crate::mesh::{self, RenderMesh};
use crate::{kv3, vbib, Resource, Value};
use anyhow::{anyhow, Result};
use glam::{Mat4, Quat, Vec3};

#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    pub names: Vec<String>,
    pub parents: Vec<i32>,
    /// Bind pose in parent space.
    pub bind_pos: Vec<[f32; 3]>,
    pub bind_rot: Vec<[f32; 4]>,
}

impl Skeleton {
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn world_from_locals(&self, locals: &[Mat4]) -> Vec<Mat4> {
        let mut world: Vec<Mat4> = Vec::with_capacity(locals.len());
        for (i, l) in locals.iter().enumerate() {
            let p = self.parents[i];
            world.push(if p >= 0 { world[p as usize] * *l } else { *l });
        }
        world
    }

    pub fn bind_world(&self) -> Vec<Mat4> {
        let locals: Vec<Mat4> = (0..self.len())
            .map(|i| {
                let r = self.bind_rot[i];
                Mat4::from_rotation_translation(Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize(), Vec3::from(self.bind_pos[i]))
            })
            .collect();
        self.world_from_locals(&locals)
    }

    /// Skinning matrices (model space, bind-relative) for an animation pose. Animation tracks
    /// follow `anim_skel`'s bone order and hierarchy; bones are matched to this skeleton by name.
    pub fn skin_matrices(&self, anim_skel: &AnimSkeleton, poses: &[BonePose]) -> Vec<[[f32; 4]; 4]> {
        let n = anim_skel.names.len().min(poses.len());
        let mut world: Vec<Mat4> = Vec::with_capacity(n);
        for i in 0..n {
            let p = &poses[i];
            let local = Mat4::from_scale_rotation_translation(
                Vec3::splat(p.scale),
                Quat::from_xyzw(p.rotation[0], p.rotation[1], p.rotation[2], p.rotation[3]).normalize(),
                Vec3::from(p.position),
            );
            let parent = anim_skel.parents[i];
            world.push(if parent >= 0 && (parent as usize) < i { world[parent as usize] * local } else { local });
        }
        let bind = self.bind_world();
        let by_name: std::collections::HashMap<&str, usize> =
            anim_skel.names.iter().take(n).enumerate().map(|(i, nm)| (nm.as_str(), i)).collect();
        // Bones the animation skeleton lacks (ears, eyes, helpers) follow their nearest animated ancestor.
        let mut skin: Vec<Mat4> = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            let m = match by_name.get(self.names[i].as_str()) {
                Some(&j) => world[j] * bind[i].inverse(),
                None => match self.parents.get(i) {
                    Some(&p) if p >= 0 && (p as usize) < i => skin[p as usize],
                    _ => Mat4::IDENTITY,
                },
            };
            skin.push(m);
        }
        skin.iter().map(Mat4::to_cols_array_2d).collect()
    }
}

/// Bone names and hierarchy of an animation skeleton (`.skel_c`).
#[derive(Clone, Debug, Default)]
pub struct AnimSkeleton {
    pub names: Vec<String>,
    pub parents: Vec<i32>,
}

pub fn load_skel(file: &[u8]) -> Result<AnimSkeleton> {
    let res = Resource::parse(file)?;
    let kv = kv3::parse(res.block_data("DATA").ok_or_else(|| anyhow!("no DATA"))?)?;
    Ok(AnimSkeleton {
        names: kv.get("m_boneIDs").and_then(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_str).map(String::from).collect(),
        parents: kv.get("m_parentIndices").and_then(Value::as_array).unwrap_or(&[]).iter().map(|v| v.as_i64().unwrap_or(-1) as i32).collect(),
    })
}

#[derive(Clone, Debug)]
pub struct ModelMesh {
    pub mesh: RenderMesh,
    pub group_mask: i64,
    pub lod_mask: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub skeleton: Skeleton,
    pub meshes: Vec<ModelMesh>,
    pub default_group_mask: i64,
}

impl Model {
    /// Meshes visible in the default mesh group at the highest LOD.
    pub fn default_meshes(&self) -> impl Iterator<Item = &ModelMesh> {
        let mask = self.default_group_mask;
        self.meshes.iter().filter(move |m| m.group_mask & mask != 0 && m.lod_mask & 1 != 0)
    }
}

fn f32s<const N: usize>(v: &Value) -> [f32; N] {
    let a = v.as_array().unwrap_or(&[]);
    std::array::from_fn(|i| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32)
}

pub fn load(file: &[u8]) -> Result<Model> {
    let res = Resource::parse(file)?;
    let kv = kv3::parse(res.block_data("DATA").ok_or_else(|| anyhow!("no DATA"))?)?;

    let mut model = Model { default_group_mask: kv.get("m_nDefaultMeshGroupMask").and_then(Value::as_i64).unwrap_or(-1), ..Default::default() };
    if let Some(sk) = kv.get("m_modelSkeleton") {
        let arr = |k: &str| sk.get(k).and_then(Value::as_array).unwrap_or(&[]);
        model.skeleton = Skeleton {
            names: arr("m_boneName").iter().filter_map(Value::as_str).map(String::from).collect(),
            parents: arr("m_nParent").iter().map(|v| v.as_i64().unwrap_or(-1) as i32).collect(),
            bind_pos: arr("m_bonePosParent").iter().map(f32s::<3>).collect(),
            bind_rot: arr("m_boneRotParent").iter().map(f32s::<4>).collect(),
        };
    }
    let ints = |k: &str| -> Vec<i64> { kv.get(k).and_then(Value::as_array).unwrap_or(&[]).iter().map(|v| v.as_i64().unwrap_or(0)).collect() };
    let remap: Vec<i32> = ints("m_remappingTable").iter().map(|&v| v as i32).collect();
    let starts = ints("m_remappingTableStarts");
    let group_masks = ints("m_refMeshGroupMasks");
    let lod_masks = ints("m_refLODGroupMasks");

    // Embedded meshes: MDAT / MBUF pairs in file order.
    let mdats: Vec<&crate::Block> = res.blocks.iter().filter(|b| &b.kind == b"MDAT").collect();
    let mbufs: Vec<&crate::Block> = res.blocks.iter().filter(|b| &b.kind == b"MBUF").collect();
    for (i, (md, mb)) in mdats.iter().zip(&mbufs).enumerate() {
        let mesh_kv = kv3::parse(&file[md.start..md.start + md.len])?;
        let vb = vbib::parse(&file[mb.start..mb.start + mb.len])?;
        let remap_slice = starts.get(i).map(|&s| {
            let end = starts.get(i + 1).map_or(remap.len(), |&e| e as usize);
            &remap[(s as usize).min(remap.len())..end.min(remap.len())]
        });
        let mesh = mesh::build_with_remap(&mesh_kv, &vb, remap_slice.filter(|r| !r.is_empty()))?;
        model.meshes.push(ModelMesh {
            mesh,
            group_mask: group_masks.get(i).copied().unwrap_or(-1),
            lod_mask: lod_masks.get(i).copied().unwrap_or(1),
        });
    }
    Ok(model)
}
