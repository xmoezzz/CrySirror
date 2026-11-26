// SPDX-License-Identifier: MPL-2.0
//! Skeleton and clip check: rest pose from inverse binds against the stored
//! default pose, then world joint positions of a clip's first frame.
//! `clip_pose SKELETON_PAK CLIP_PAK CLIP [JOINT…]`
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{anim, mesh::Loaded, skeleton};
use glam::{Mat4, Vec3};

fn load(path: &str) -> Result<Loaded> {
    Loaded::read(&MemoryAssetReader(std::fs::read(path)?.into()), 1 << 30)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let skel = skeleton::of_package(&load(args.first().context("SKELETON_PAK")?)?)?.context("no skeleton")?;
    let clips = load(args.get(1).context("CLIP_PAK")?)?;
    let name = args.get(2).context("CLIP")?;
    let (_, payload) = anim::items(&clips).find(|(n, _)| n == name).context("clip not found")?;
    let clip = anim::decode(&clips, payload, name, skel.animated)?;
    println!("skeleton {:08x}: {} joints ({} animated); clip {:08x} {} frames @ {} fps; unsupported {:?}", skel.hash, skel.names.len(), skel.animated, clip.hierarchy, clip.frames, clip.fps, clip.unsupported);
    println!("root speed {:.2} m/s; align {} frames {:?} → {:?}; apReference {} frames", clip.root_speed, clip.align.len(), clip.align.first().map(|a| a.0.to_array()), clip.align.last().map(|a| (a.0.to_array(), a.1.to_array())), clip.ap_reference.len());
    if let Some((t, q)) = clip.ap_reference.first() {
        println!("apReference first frame: translation {:?} rotation {:?}", t.to_array(), q.to_array());
    }
    let animated = clip.joints.iter().filter(|j| j.rotation.is_some() || j.translation.is_some()).count();
    println!("joints with channels: {animated}");
    let mut world = vec![Mat4::IDENTITY; skel.names.len()];
    for j in skel.order() {
        let (mut t, mut r, mut s) = skel.rest[j];
        if let Some(track) = clip.joints.get(j) {
            t = track.translation.as_ref().map_or(t, |v| v[0]);
            r = track.rotation.as_ref().map_or(r, |v| v[0]);
            s = track.scale.as_ref().map_or(s, |v| v[0]);
        }
        let local = Mat4::from_scale_rotation_translation(s, r, t);
        world[j] = skel.parents[j].map_or(local, |p| world[p] * local);
    }
    if std::env::var_os("FRAMES").is_some() {
        for f in 0..clip.frames {
            let mut w = vec![Mat4::IDENTITY; skel.names.len()];
            for j in skel.order() {
                let (mut t, mut r, mut sc) = skel.rest[j];
                if let Some(tr) = clip.joints.get(j) {
                    t = tr.translation.as_ref().map_or(t, |v| v[f]);
                    r = tr.rotation.as_ref().map_or(r, |v| v[f]);
                    sc = tr.scale.as_ref().map_or(sc, |v| v[f]);
                }
                let l = Mat4::from_scale_rotation_translation(sc, r, t);
                w[j] = skel.parents[j].map_or(l, |p| w[p] * l);
            }
            let at = |n: &str| skel.names.iter().position(|x| x == n).map(|j| w[j].transform_point3(Vec3::ZERO).to_array().map(|v| (v * 100.).round() / 100.));
            println!("frame {f:2} l_ankle {:?} r_ankle {:?} l_wrist {:?} head {:?}", at("l_ankle"), at("r_ankle"), at("l_wrist"), at("head"));
        }
    }
    if let Ok(list) = std::env::var("LOCAL") {
        for n in list.split(',') {
            if let Some(j) = skel.names.iter().position(|x| x == n) {
                let r = clip.joints.get(j).and_then(|t| t.rotation.as_ref()).map(|v| v[0]);
                println!("local {n} {:?}", r.map(|q| q.to_array().map(|v| (v * 1e4).round() / 1e4)));
            }
        }
    }
    let pick: Vec<&str> = if args.len() > 3 { args[3..].iter().map(String::as_str).collect() } else { vec!["root", "pelvis", "l_ankle", "r_ankle", "spined", "head", "l_wrist", "r_wrist"] };
    for n in pick {
        if let Some(j) = skel.names.iter().position(|x| x == n) {
            let bind = Mat4::from_cols_array(&skel.inverse_bind[j]).inverse().transform_point3(Vec3::ZERO);
            println!("{n:12} posed {:?} bind {:?}", world[j].transform_point3(Vec3::ZERO).to_array().map(|v| (v * 1000.).round() / 1000.), bind.to_array().map(|v| (v * 1000.).round() / 1000.));
        }
    }
    Ok(())
}
