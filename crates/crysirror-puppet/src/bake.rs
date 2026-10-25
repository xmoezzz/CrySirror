// SPDX-License-Identifier: MPL-2.0
//! Bakes any [`PuppetSource`] into a [`PuppetRig`] by sampling it through
//! the interface. This is how puppets from other runtimes enter packages:
//! the package holds only the engine format and its textures.
//!
//! The model is additive per parameter: each parameter is sampled with all
//! others at their defaults, and its vertex offsets and opacity factors
//! become keyforms. Interactions between parameters (e.g. head angle ×
//! mouth shape) are therefore approximated, and the report says so.
use crate::{
    Curve, Interpolation, Motion, MotionBlend, Parameter, Part, PartKey, PartKeyform, PuppetRig,
    PuppetFrame, PuppetSource, FORMAT, VERSION,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct BakeOptions {
    /// Evenly spaced samples per parameter, added to the source's own keys.
    pub samples_per_parameter: usize,
    /// Motion sampling rate (Hz).
    pub motion_fps: f32,
    /// Length sampled for motions of unknown duration.
    pub max_motion_ms: f32,
    /// Offsets below this (puppet pixels) count as no deformation.
    pub tolerance: f32,
    /// Free-form note stored in the rig.
    pub provenance: String,
}
impl Default for BakeOptions {
    fn default() -> Self {
        Self { samples_per_parameter: 5, motion_fps: 30., max_motion_ms: 10_000., tolerance: 0.01, provenance: "crysirror-puppet bake".into() }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BakeReport {
    pub parts: usize,
    pub parameters: usize,
    pub keyforms: usize,
    pub motions: usize,
    /// (part, parameter) pairs whose vertex count changed between samples.
    pub topology_changes: Vec<(String, String)>,
    /// Parameters that change the relative draw order (not baked).
    pub reordering_parameters: Vec<String>,
    /// Parts absent at the defaults (baked with opacity keys only).
    pub hidden_at_rest: Vec<String>,
    pub limitations: Vec<String>,
}

/// One baked texture: file name in the rig and pixels when the source has them.
pub struct BakedTexture {
    pub file: String,
    pub source_name: String,
    pub rgba: Option<(u32, u32, Vec<u8>)>,
}

fn settle(source: &mut dyn PuppetSource, params: &[crate::ParameterInfo]) -> Result<()> {
    source.stop("")?;
    for p in params {
        source.release_parameter(&p.id)?;
    }
    source.advance(10_000.)?;
    Ok(())
}
fn pin_defaults(source: &mut dyn PuppetSource, params: &[crate::ParameterInfo]) -> Result<()> {
    for p in params {
        source.set_parameter(&p.id, p.default, 0.)?;
    }
    source.advance(0.)
}
fn mean_alpha(colors: &[[f32; 4]]) -> f32 {
    if colors.is_empty() {
        1.
    } else {
        colors.iter().map(|c| c[3]).sum::<f32>() / colors.len() as f32
    }
}
fn order_pairs(f: &PuppetFrame) -> BTreeMap<String, usize> {
    f.parts.iter().enumerate().filter(|(_, p)| p.visible).map(|(i, p)| (p.id.clone(), i)).collect()
}

pub fn bake(source: &mut dyn PuppetSource, options: &BakeOptions) -> Result<(PuppetRig, Vec<BakedTexture>, BakeReport)> {
    ensure!(options.samples_per_parameter >= 2 && options.motion_fps > 0. && options.max_motion_ms > 0., "invalid bake options");
    let params = source.parameters();
    let mut report = BakeReport { parameters: params.len(), ..Default::default() };
    report.limitations.push("parameters are baked independently (additive keyforms); cross-parameter interactions are approximated".into());
    report.limitations.push("source physics are baked into motion curves where they move parameters; no pendulums are generated".into());
    settle(source, &params)?;
    pin_defaults(source, &params)?;
    let base = source.frame()?;
    let base_order = order_pairs(&base);

    // Sample every parameter.
    let mut samples: Vec<(usize, Vec<(f32, PuppetFrame)>)> = Vec::new();
    for (pi, p) in params.iter().enumerate() {
        let mut values: Vec<f32> = (0..options.samples_per_parameter)
            .map(|i| p.min + (p.max - p.min) * i as f32 / (options.samples_per_parameter - 1) as f32)
            .chain(p.keys.iter().copied().filter(|v| (p.min..=p.max).contains(v)))
            .chain(std::iter::once(p.default))
            .collect();
        values.sort_by(f32::total_cmp);
        values.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let mut frames = Vec::new();
        for v in values {
            source.set_parameter(&p.id, v, 0.)?;
            source.advance(0.)?;
            frames.push((v, source.frame()?));
        }
        source.set_parameter(&p.id, p.default, 0.)?;
        source.advance(0.)?;
        samples.push((pi, frames));
    }

    // Parts: every part seen at rest or in any sample, rest data from its
    // first appearance, draw order from the rest frame (or first sample).
    let mut parts: Vec<Part> = Vec::new();
    let mut rank: BTreeMap<String, f32> = BTreeMap::new();
    let mut rest_alpha: BTreeMap<String, f32> = BTreeMap::new();
    let mut all_frames: Vec<&PuppetFrame> = vec![&base];
    all_frames.extend(samples.iter().flat_map(|(_, f)| f.iter().map(|(_, fr)| fr)));
    for frame in &all_frames {
        for (i, fp) in frame.parts.iter().enumerate() {
            if parts.iter().any(|p| p.id == fp.id) {
                continue;
            }
            // Parts first seen in a sample slot in after their predecessor.
            let r = if std::ptr::eq(*frame, &base) {
                i as f32
            } else {
                i.checked_sub(1).and_then(|j| rank.get(&frame.parts[j].id)).map_or(-1., |r| r + 0.5)
            };
            rank.insert(fp.id.clone(), r);
            if !std::ptr::eq(*frame, &base) {
                report.hidden_at_rest.push(fp.id.clone());
            }
            rest_alpha.insert(fp.id.clone(), mean_alpha(&fp.colors).max(1e-6));
            parts.push(Part {
                id: fp.id.clone(),
                texture: fp.texture,
                positions: fp.positions.clone(),
                uvs: fp.uvs.clone(),
                indices: fp.indices.clone(),
                colors: fp.colors.iter().map(|c| [c[0], c[1], c[2], 1.]).collect(),
                opacity: 1.,
                order: 0,
                blend: fp.blend,
                deformer: None,
                masks: fp.masks.iter().map(|m| frame.parts[*m].id.clone()).collect(),
                invert_mask: fp.invert_mask,
                visible: fp.visible,
                keyforms: Vec::new(),
            });
        }
    }
    let mut ordered: Vec<(f32, String)> = rank.iter().map(|(k, v)| (*v, k.clone())).collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (i, (_, id)) in ordered.iter().enumerate() {
        parts.iter_mut().find(|p| &p.id == id).unwrap().order = i as i32 * 10;
    }
    let present_at_rest: BTreeSet<&str> = base.parts.iter().map(|p| p.id.as_str()).collect();

    // Keyforms.
    for (pi, frames) in &samples {
        let p = &params[*pi];
        let mut reorders = false;
        for (_, f) in frames {
            let o = order_pairs(f);
            let mut common: Vec<(&String, usize, usize)> =
                o.iter().filter_map(|(k, v)| base_order.get(k).map(|b| (k, *b, *v))).collect();
            common.sort_by_key(|c| c.1);
            if common.windows(2).any(|w| w[0].2 > w[1].2) {
                reorders = true;
            }
        }
        if reorders {
            report.reordering_parameters.push(p.id.clone());
        }
        for part in parts.iter_mut() {
            let presence: Vec<Option<&crate::FramePart>> =
                frames.iter().map(|(_, f)| f.parts.iter().find(|x| x.id == part.id)).collect();
            let varies = presence.iter().any(Option::is_some) && presence.iter().any(Option::is_none);
            if presence.iter().all(Option::is_none) {
                continue;
            }
            let mut keys = Vec::new();
            let mut meaningful = varies;
            let mut topology = false;
            for ((v, _), fp) in frames.iter().zip(&presence) {
                let Some(fp) = fp else {
                    keys.push(PartKey { value: *v, offsets: vec![], opacity: 0., order: 0 });
                    continue;
                };
                if fp.positions.len() != part.positions.len() {
                    topology = true;
                    break;
                }
                let offsets: Vec<[f32; 2]> =
                    fp.positions.iter().zip(&part.positions).map(|(a, b)| [a[0] - b[0], a[1] - b[1]]).collect();
                let moved = offsets.iter().any(|o| o[0].abs() > options.tolerance || o[1].abs() > options.tolerance);
                let opacity = (mean_alpha(&fp.colors) / rest_alpha[&part.id]).clamp(0., 1.);
                meaningful |= moved || (opacity - 1.).abs() > 1e-3;
                keys.push(PartKey { value: *v, offsets: if moved { offsets } else { vec![] }, opacity, order: 0 });
            }
            if topology {
                report.topology_changes.push((part.id.clone(), p.id.clone()));
                continue;
            }
            if meaningful {
                part.keyforms.push(PartKeyform { parameter: p.id.clone(), keys });
                report.keyforms += 1;
            }
        }
    }
    // A part that only exists away from the defaults has a keyform whose
    // rest key is transparent; one missing everywhere at rest is not drawn
    // until some parameter brings it in.
    for part in &mut parts {
        if !present_at_rest.contains(part.id.as_str()) && part.keyforms.iter().all(|k| k.keys.iter().all(|k| k.opacity > 0.)) {
            part.opacity = 0.;
        }
    }

    // Motions: sample parameter values over time.
    let mut motions = Vec::new();
    for m in source.motions() {
        settle(source, &params)?;
        source.play(&m.id, crate::PlayOptions { looping: Some(false), weight: 1. })
            .with_context(|| format!("bake motion '{}'", m.id))?;
        let duration = m.duration_ms.unwrap_or(options.max_motion_ms).min(options.max_motion_ms).max(1.);
        let step = 1000. / options.motion_fps;
        let mut series: BTreeMap<String, Vec<[f32; 2]>> = BTreeMap::new();
        let mut t = 0f32;
        source.advance(0.)?;
        loop {
            for p in &params {
                let v = source.parameter_value(&p.id).unwrap_or(p.default);
                series.entry(p.id.clone()).or_default().push([t, v]);
            }
            if t >= duration {
                break;
            }
            let h = step.min(duration - t);
            source.advance(h)?;
            t += h;
        }
        let curves: Vec<Curve> = series
            .into_iter()
            .filter(|(_, keys)| {
                let first = keys[0][1];
                keys.iter().any(|k| (k[1] - first).abs() > 1e-4)
            })
            .map(|(parameter, keys)| Curve { parameter, keys: simplify(&keys, 1e-4), interpolation: Interpolation::Linear })
            .collect();
        if curves.is_empty() {
            continue;
        }
        motions.push(Motion { id: m.id.clone(), duration_ms: duration, looping: m.looping, blend: MotionBlend::Override, fade_in_ms: 0., fade_out_ms: 0., curves });
    }
    settle(source, &params)?;
    report.motions = motions.len();
    report.parts = parts.len();

    let names = source.texture_names();
    let textures: Vec<BakedTexture> = names
        .iter()
        .enumerate()
        .map(|(i, n)| BakedTexture { file: format!("texture_{i:03}.png"), source_name: n.clone(), rgba: source.texture_rgba(i as u32) })
        .collect();
    let rig = PuppetRig {
        format: FORMAT.into(),
        version: VERSION,
        size: source.size(),
        textures: textures.iter().map(|t| t.file.clone()).collect(),
        parameters: params.iter().map(|p| Parameter { id: p.id.clone(), min: p.min, max: p.max, default: p.default }).collect(),
        deformers: Vec::new(),
        parts,
        motions,
        physics: Vec::new(),
        provenance: Some(options.provenance.clone()),
    };
    rig.validate().context("baked rig failed validation")?;
    Ok((rig, textures, report))
}

/// Drops keys that linear interpolation of their neighbours reproduces.
fn simplify(keys: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if keys.len() <= 2 {
        return keys.to_vec();
    }
    let mut out = vec![keys[0]];
    for i in 1..keys.len() - 1 {
        let (a, b, c) = (out[out.len() - 1], keys[i], keys[i + 1]);
        let f = (b[0] - a[0]) / (c[0] - a[0]);
        if (a[1] + (c[1] - a[1]) * f - b[1]).abs() > eps {
            out.push(b);
        }
    }
    out.push(keys[keys.len() - 1]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{eval::tests::test_rig, Puppet};
    use std::sync::Arc;

    fn max_error(a: &PuppetFrame, b: &PuppetFrame) -> f32 {
        let mut e = 0f32;
        for pa in a.parts.iter().filter(|p| p.visible) {
            let pb = b.parts.iter().find(|p| p.id == pa.id).expect("part present in both");
            for (x, y) in pa.positions.iter().zip(&pb.positions) {
                e = e.max((x[0] - y[0]).abs()).max((x[1] - y[1]).abs());
            }
        }
        e
    }

    #[test]
    fn baking_through_the_interface_reproduces_single_parameter_poses_and_motions() {
        let rig = Arc::new(test_rig());
        let mut source = Puppet::new(rig.clone()).unwrap();
        let (baked, textures, report) = bake(&mut source, &BakeOptions { samples_per_parameter: 9, ..Default::default() }).unwrap();
        assert_eq!(textures.len(), 1);
        assert_eq!(report.parts, 4);
        assert!(baked.deformers.is_empty(), "deformers are flattened into keyforms");
        let eye = baked.parts.iter().find(|p| p.id == "eye").unwrap();
        assert_eq!(eye.masks, vec!["eye_white".to_string()]);
        // The two puppets agree at sampled and in-between single-parameter poses.
        let mut a = Puppet::new(rig).unwrap();
        let mut b = Puppet::new(Arc::new(baked.clone())).unwrap();
        for (id, v) in [("angle", 30.), ("angle", -11.), ("eye_open", 0.25), ("breath", 1.)] {
            for p in [&mut a, &mut b] {
                p.set_parameter(id, v, 0.).unwrap();
                p.advance(0.).unwrap();
            }
            let e = max_error(&a.frame(), &b.frame());
            // Rotation is curved; nine linear keys over ±15° stay within 0.5 px.
            assert!(e < 0.5, "{id}={v}: {e} px");
            for p in [&mut a, &mut b] {
                p.release_parameter(id).unwrap();
                p.advance(0.).unwrap();
            }
        }
        // Combined parameters are approximated, not exact.
        for p in [&mut a, &mut b] {
            p.set_parameter("angle", 30., 0.).unwrap();
            p.set_parameter("breath", 1., 0.).unwrap();
            p.advance(0.).unwrap();
        }
        assert!(max_error(&a.frame(), &b.frame()) < 2.0);
        // Motions come back as parameter curves.
        let blink = baked.motions.iter().find(|m| m.id == "blink").expect("blink baked");
        assert_eq!(blink.curves.len(), 1);
        assert_eq!(blink.curves[0].parameter, "eye_open");
        let min = blink.curves[0].keys.iter().map(|k| k[1]).fold(f32::MAX, f32::min);
        // 30 Hz samples straddle the 150 ms minimum (133 and 167 ms).
        assert!(min < 0.15, "blink closes the eye: {min}");
        assert!(!report.limitations.is_empty());
    }

    #[test]
    fn simplify_drops_collinear_keys() {
        let k = [[0., 0.], [1., 1.], [2., 2.], [3., 0.]];
        assert_eq!(simplify(&k, 1e-4), vec![[0., 0.], [2., 2.], [3., 0.]]);
    }
}
