// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! KaichoTrans option/resource semantics only; filtering/composition lives in
//! CrySirror's shared subtree GPU APIs. Complete v0.2.5.1 Main.cpp, blur.cpp,
//! dim.cpp and license reviewed from the author's source release, SHA256
//! 11c47bd69bf7235d4238c2227fbd122dfadb42e1bf8ce76e84c4400edd66ba13.
use crate::natives::{Rt, unsupported};
use crysirror_scene::layer::{MAX_TRANSITION_BLUR_RADIUS, RuleWipe, SubtreeTransitionEffect};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
fn err(text: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("KaichoTrans: {text}"))
}
fn value(rt: &mut Rt, options: ObjectHandle, name: &str) -> Result<Variant> {
    rt.resolve_object_member(options, name)
}
fn integer(rt: &mut Rt, options: ObjectHandle, name: &str, default: i64) -> Result<i64> {
    match value(rt, options, name)? {
        Variant::Void => Ok(default),
        v => v.to_integer(),
    }
}
fn radius(rt: &mut Rt, options: ObjectHandle, name: &str, default: u32) -> Result<u32> {
    let radius = integer(rt, options, name, i64::from(default))?;
    if !(0..=i64::from(MAX_TRANSITION_BLUR_RADIUS)).contains(&radius) {
        return Err(err(format!(
            "{name}: supported radius is 0..={MAX_TRANSITION_BLUR_RADIUS}"
        )));
    }
    Ok(radius as u32)
}
pub fn effect(rt: &mut Rt, name: &str, options: ObjectHandle) -> Result<SubtreeTransitionEffect> {
    if !rt.host().loaded_plugins.contains("kaichotrans.dll") {
        return Err(unsupported(
            "KaichoTrans transition",
            "linked KaichoTrans provider",
        ));
    }
    for (key, _) in rt.object_members(options) {
        let common = [
            "tagname",
            "taglist",
            "method",
            "layer",
            "page",
            "children",
            "withchildren",
            "exchange",
            "cond",
            "time",
            "callback",
            "selfupdate",
        ];
        let own = if name == "blur" {
            &[
                "blur1", "blur1x", "blur1y", "blur2", "blur2x", "blur2y", "accel", "dynamic",
            ][..]
        } else {
            &["rule", "blur", "xblur", "yblur", "neg", "vague", "accel"][..]
        };
        if !common.contains(&key.as_str()) && !own.contains(&key.as_str()) {
            return Err(err(format!("unmapped {name} option {key}")));
        }
    }
    let acceleration = match value(rt, options, "accel")? {
        Variant::Void => 1.,
        v => v.to_real()?,
    };
    if !acceleration.is_finite() {
        return Err(err("acceleration must be finite"));
    }
    let effect = if name == "blur" {
        let mut before = [
            radius(rt, options, "blur1x", 32)?,
            radius(rt, options, "blur1y", 32)?,
        ];
        let mut after = [
            radius(rt, options, "blur2x", 32)?,
            radius(rt, options, "blur2y", 32)?,
        ];
        // The actual source reads the combined radius last, overriding axes.
        if !matches!(value(rt, options, "blur1")?, Variant::Void) {
            before.fill(radius(rt, options, "blur1", 32)?);
        }
        if !matches!(value(rt, options, "blur2")?, Variant::Void) {
            after.fill(radius(rt, options, "blur2", 32)?);
        }
        // accel is parsed but the original Blur StartProcess never invokes
        // its curve function. Preserve that observable linear behaviour.
        SubtreeTransitionEffect::Blur {
            before,
            after,
            dynamic: integer(rt, options, "dynamic", 0)? != 0,
        }
    } else {
        let rule = value(rt, options, "rule")?;
        if matches!(rule, Variant::Void) {
            return Err(err("dim requires a rule image"));
        }
        let rule = rule.to_tjs_string()?;
        let asset = rt
            .host()
            .storage()?
            .resolve_with(&rule, &[".tlg", ".png", ".jpg", ".jpeg", ".bmp", ".webp"])
            .ok_or_else(|| err(format!("rule image {rule:?} not found")))?;
        let base = radius(rt, options, "blur", 16)?;
        let blur = [
            radius(rt, options, "xblur", base)?,
            radius(rt, options, "yblur", base)?,
        ];
        let vague = integer(rt, options, "vague", 64)?;
        if !(0..=4096).contains(&vague) {
            return Err(err("dim vague must be 0..=4096"));
        }
        // Source checks presence/non-void, not truthiness: neg=false still
        // invokes NegateGrayImage. This is intentional native policy.
        let negative = !matches!(value(rt, options, "neg")?, Variant::Void);
        SubtreeTransitionEffect::Dim {
            rule: RuleWipe {
                asset,
                vague: vague as u32,
            },
            blur,
            negative,
            acceleration,
        }
    };
    effect.validate().map_err(err)?;
    Ok(effect)
}
