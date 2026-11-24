// SPDX-License-Identifier: MPL-2.0
//! `JOINT_HIERARCHY` skeletons.
//!
//! The item payload is `{8, joint count, JH block, flip data, names, …}`;
//! names are `{StringId, string pointer}` pairs. The JH block (ICE joint
//! hierarchy, magic `0x4a48010a`) holds its hierarchy hash at +4 (clips
//! name the hash they animate), the joint count (`u16` +0xc), the animated
//! joint count (`u16` +0x10) and block-relative offsets: +0x28 per-joint
//! records `{flags, parent, first child, next sibling}`, +0x2c the default
//! local pose of the animated joints (48 bytes: scale, quaternion x y z w,
//! translation; the animated joints come first) and +0x44 one 3 × 4
//! row-major inverse bind matrix per joint (`x' = R·x + t`, metres). On
//! `npc-normal-base` and the player rig in `actor27.psarc/proto.pak` these
//! give a human skeleton rooted at the feet (ankles 0.12 m, pelvis 1.06 m).
use crate::mesh::Loaded;
use anyhow::{Context, Result, ensure};
use glam::{Affine3A, Mat4, Quat, Vec3};

pub const MAGIC: u32 = 0x4a48_010a;

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub hash: u32,
    pub names: Vec<String>,
    /// Parent joint, `None` for roots (not necessarily a lower index).
    pub parents: Vec<Option<usize>>,
    /// Joints `0..animated` carry clip channels.
    pub animated: usize,
    /// glTF column-major inverse bind matrices.
    pub inverse_bind: Vec<[f32; 16]>,
    /// Bind-pose local transforms (translation, rotation, scale).
    pub rest: Vec<(Vec3, Quat, Vec3)>,
}

/// The skeleton of a `JOINT_HIERARCHY` payload.
pub fn read(p: &Loaded, payload: usize) -> Result<Skeleton> {
    let jh = p.ptr(payload + 8)?;
    ensure!(p.u32(jh)? == MAGIC, "ND joint hierarchy magic");
    let hash = p.u32(jh + 4)?;
    let count = usize::from(p.u16(jh + 0xc)?);
    let animated = usize::from(p.u16(jh + 0x10)?);
    ensure!(count > 0 && count <= 1024 && animated <= count, "ND joint counts");
    let (records, inverse) = (jh + p.u32(jh + 0x28)? as usize, jh + p.u32(jh + 0x44)? as usize);
    let names_at = p.ptr(payload + 0x10)?;
    let mut names = Vec::with_capacity(count);
    let mut parents = Vec::with_capacity(count);
    let mut world = Vec::with_capacity(count);
    let mut inverse_bind = Vec::with_capacity(count);
    for j in 0..count {
        names.push(p.ptr(names_at + 8 * j + 4).ok().map(|a| cstr(p, a)).unwrap_or_else(|| format!("joint{j}")));
        let parent = p.u32(records + 16 * j + 4)? as i32;
        ensure!(parent < count as i32, "ND joint parent");
        parents.push((parent >= 0).then_some(parent as usize));
        let m: [f32; 12] = std::array::from_fn(|k| p.f32(inverse + 48 * j + 4 * k).unwrap_or(f32::NAN));
        ensure!(m.iter().all(|v| v.is_finite()), "ND inverse bind matrix");
        // Row-major 3 × 4 (`x' = R·x + t`) → column-major 4 × 4.
        let ib = [m[0], m[4], m[8], 0., m[1], m[5], m[9], 0., m[2], m[6], m[10], 0., m[3], m[7], m[11], 1.];
        inverse_bind.push(ib);
        let inv = Affine3A::from_mat3_translation(glam::Mat3::from_cols_array(&[m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]]), Vec3::new(m[3], m[7], m[11]));
        world.push(inv.inverse());
    }
    let rest = (0..count)
        .map(|j| {
            let local = match parents[j] {
                Some(pj) => world[pj].inverse() * world[j],
                None => world[j],
            };
            let (s, r, t) = Mat4::from(local).to_scale_rotation_translation();
            (t, r.normalize(), s)
        })
        .collect();
    // Every joint must reach a root.
    for j in 0..count {
        let (mut at, mut steps) = (j, 0);
        while let Some(p) = parents[at] {
            at = p;
            steps += 1;
            ensure!(steps <= count, "ND joint hierarchy cycle");
        }
    }
    Ok(Skeleton { hash, names, parents, animated, inverse_bind, rest })
}

impl Skeleton {
    /// Joints ordered so that every parent precedes its children.
    pub fn order(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.parents.len());
        let mut done = vec![false; self.parents.len()];
        fn visit(j: usize, parents: &[Option<usize>], done: &mut [bool], out: &mut Vec<usize>) {
            if done[j] {
                return;
            }
            if let Some(p) = parents[j] {
                visit(p, parents, done, out);
            }
            done[j] = true;
            out.push(j);
        }
        for j in 0..self.parents.len() {
            visit(j, &self.parents, &mut done, &mut out);
        }
        out
    }
}

fn cstr(p: &Loaded, at: usize) -> String {
    let bytes = p.data.get(at..).unwrap_or_default();
    let end = bytes.iter().take(256).position(|b| *b == 0).unwrap_or(0);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The first `JOINT_HIERARCHY` of a relocated package.
pub fn of_package(p: &Loaded) -> Result<Option<Skeleton>> {
    match p.directory.items.iter().find(|i| i.kind == "JOINT_HIERARCHY") {
        Some(item) => read(p, item.payload_offset as usize).map(Some).context("ND joint hierarchy"),
        None => Ok(None),
    }
}
