// SPDX-License-Identifier: MPL-2.0
//! ATL source form (`renpy.atl.Raw*` classes) and its parser (`parse_atl`).
//! Execution lives with the transform implementation.

use crate::lexer::{Lexer, PResult, PyExpr};
use std::rc::Rc;

pub type Loc = (Rc<str>, u32);

/// Transform properties accepted by ATL (`renpy.atl.PROPERTIES`), as
/// registered by `renpy/display/transform.py` and its aliases.
pub const PROPERTIES: &[&str] = &[
    "additive", "alpha", "blend", "blur", "corner1", "corner2", "crop", "crop_relative", "debug", "delay", "events", "fit", "fps",
    "matrixanchor", "matrixcolor", "matrixtransform", "maxsize", "mesh", "mesh_pad", "nearest", "perspective", "rotate", "rotate_pad",
    "point_to", "orientation", "xrotate", "yrotate", "zrotate", "shader", "show_cancels_hide", "subpixel", "transform_anchor", "zoom",
    "xanchoraround", "xanchor", "xaround", "xoffset", "xpan", "xpos", "xsize", "xtile", "xzoom", "yanchoraround", "yanchor", "yaround",
    "yoffset", "ypan", "ypos", "ysize", "ytile", "yzoom", "zpos", "zzoom", "gl_anisotropic", "gl_blend_func", "gl_color_mask",
    "gl_cull_face", "gl_depth", "gl_drawable_resolution", "gl_mipmap", "gl_pixel_perfect", "gl_texture_scaling", "gl_texture_wrap",
    // Aliases.
    "alignaround", "align", "anchor", "anchorangle", "anchoraround", "anchorradius", "angle", "around", "offset", "pos", "radius", "size",
    "xalign", "xcenter", "xycenter", "xysize", "yalign", "ycenter", "_reset",
    // Older releases (6.x/7.x) accepted these as ATL properties too.
    "xalignaround", "yalignaround", "xcenter", "ycenter", "xzoom", "yzoom", "set_center", "radius", "clipping",
];

pub fn is_property(name: &str) -> bool {
    PROPERTIES.contains(&name) || name.starts_with("u_") || name.starts_with("gl_texture_wrap_") || name.starts_with("gl_texture_scaling_")
}

#[derive(Clone, Debug, Default)]
pub struct RawMultipurpose {
    pub loc: Option<Loc>,
    pub warper: Option<String>,
    pub duration: Option<PyExpr>,
    pub warp_function: Option<PyExpr>,
    pub properties: Vec<(String, PyExpr)>,
    pub expressions: Vec<(PyExpr, Option<PyExpr>)>,
    pub splines: Vec<(String, Vec<PyExpr>)>,
    pub revolution: Option<String>,
    pub circles: Option<PyExpr>,
}

#[derive(Clone, Debug)]
pub enum RawStatement {
    Block(RawBlock),
    Multipurpose(RawMultipurpose),
    Repeat { loc: Loc, repeats: Option<PyExpr> },
    Parallel { loc: Loc, blocks: Vec<RawBlock> },
    Choice { loc: Loc, choices: Vec<(PyExpr, RawBlock)> },
    On { loc: Loc, handlers: Vec<(String, RawBlock)> },
    Time { loc: Loc, time: PyExpr },
    Event { loc: Loc, name: String },
    Function { loc: Loc, expr: PyExpr },
    ContainsExpr { loc: Loc, expr: PyExpr },
    Child { loc: Loc, children: Vec<RawBlock> },
}

#[derive(Clone, Debug)]
pub struct RawBlock {
    pub loc: Loc,
    pub statements: Vec<RawStatement>,
    pub animation: bool,
}

/// Script-load-time registries consulted by parsers.
pub trait ParseContext {
    fn is_warper(&self, name: &str) -> bool;
}

/// Warpers defined by `renpy/atl.py` and `renpy/common/000atl.rpy` /
/// `00warpers` in every release; games may register more.
pub const BUILTIN_WARPERS: &[&str] = &[
    "pause", "instant", "linear", "easeout", "easein", "ease", "easeout_quad", "easein_quad", "ease_quad", "easeout_cubic", "easein_cubic",
    "ease_cubic", "easeout_quart", "easein_quart", "ease_quart", "easeout_quint", "easein_quint", "ease_quint", "easeout_expo", "easein_expo",
    "ease_expo", "easeout_circ", "easein_circ", "ease_circ", "easeout_back", "easein_back", "ease_back", "easeout_elastic", "easein_elastic",
    "ease_elastic", "easeout_bounce", "easein_bounce", "ease_bounce",
];

pub fn parse_atl(l: &mut Lexer, cx: &dyn ParseContext) -> PResult<RawBlock> {
    l.advance();
    let block_loc = l.location();
    let mut statements: Vec<Option<RawStatement>> = Vec::new();
    let mut animation = false;
    while !l.eob {
        let loc = l.location();
        if l.keyword("repeat") {
            let r = l.simple_expression()?;
            statements.push(Some(RawStatement::Repeat { loc, repeats: r }));
        } else if l.keyword("block") {
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("block")?;
            let mut sub = l.subblock_lexer(false);
            statements.push(Some(RawStatement::Block(parse_atl(&mut sub, cx)?)));
        } else if l.keyword("contains") {
            match l.simple_expression()? {
                Some(expr) => {
                    l.expect_noblock("contains expression")?;
                    statements.push(Some(RawStatement::ContainsExpr { loc, expr }));
                }
                None => {
                    l.require_re(":", ":")?;
                    l.expect_eol()?;
                    l.expect_block("contains")?;
                    let mut sub = l.subblock_lexer(false);
                    let b = parse_atl(&mut sub, cx)?;
                    statements.push(Some(RawStatement::Child { loc, children: vec![b] }));
                }
            }
        } else if l.keyword("parallel") {
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("parallel")?;
            let mut sub = l.subblock_lexer(false);
            let b = parse_atl(&mut sub, cx)?;
            statements.push(Some(RawStatement::Parallel { loc, blocks: vec![b] }));
        } else if l.keyword("choice") {
            let chance = l.simple_expression()?.unwrap_or_else(|| l.expr("1.0".into()));
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("choice")?;
            let mut sub = l.subblock_lexer(false);
            let b = parse_atl(&mut sub, cx)?;
            statements.push(Some(RawStatement::Choice { loc, choices: vec![(chance, b)] }));
        } else if l.keyword("on") {
            let mut names = vec![match l.word() {
                Some(w) => w,
                None => return l.error("expected 'word' not found."),
            }];
            while l.match_re(",").is_some() {
                match l.word() {
                    Some(n) => names.push(n),
                    None => break,
                }
            }
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("on")?;
            let mut sub = l.subblock_lexer(false);
            let b = parse_atl(&mut sub, cx)?;
            statements.push(Some(RawStatement::On { loc, handlers: names.into_iter().map(|n| (n, b.clone())).collect() }));
        } else if l.keyword("time") {
            let t = l.require_simple()?;
            l.expect_noblock("time")?;
            statements.push(Some(RawStatement::Time { loc, time: t }));
        } else if l.keyword("function") {
            let e = l.require_simple()?;
            l.expect_noblock("function")?;
            statements.push(Some(RawStatement::Function { loc, expr: e }));
        } else if l.keyword("event") {
            let n = match l.word() {
                Some(w) => w,
                None => return l.error("expected 'word' not found."),
            };
            l.expect_noblock("event")?;
            statements.push(Some(RawStatement::Event { loc, name: n }));
        } else if l.keyword("pass") {
            l.expect_noblock("pass")?;
            statements.push(None);
        } else if l.keyword("animation") {
            l.expect_noblock("animation")?;
            animation = true;
        } else {
            let rm = parse_multipurpose(l, cx, loc)?;
            statements.push(Some(RawStatement::Multipurpose(rm)));
        }
        if l.eol() {
            l.advance();
            continue;
        }
        l.require_re(",", "comma or end of line")?;
    }
    // Merge adjacent parallel/choice/contains/on statements; `pass` splits.
    let mut merged: Vec<RawStatement> = Vec::new();
    let mut prev_was_pass = true;
    for s in statements {
        let Some(new) = s else {
            prev_was_pass = true;
            continue;
        };
        if !prev_was_pass {
            match (merged.last_mut(), &new) {
                (Some(RawStatement::Parallel { blocks, .. }), RawStatement::Parallel { blocks: nb, .. }) => {
                    blocks.extend(nb.iter().cloned());
                    continue;
                }
                (Some(RawStatement::Choice { choices, .. }), RawStatement::Choice { choices: nc, .. }) => {
                    choices.extend(nc.iter().cloned());
                    continue;
                }
                (Some(RawStatement::Child { children, .. }), RawStatement::Child { children: nc, .. }) => {
                    children.extend(nc.iter().cloned());
                    continue;
                }
                (Some(RawStatement::On { handlers, .. }), RawStatement::On { handlers: nh, .. }) => {
                    for (n, b) in nh {
                        match handlers.iter_mut().find(|(k, _)| k == n) {
                            Some(slot) => slot.1 = b.clone(),
                            None => handlers.push((n.clone(), b.clone())),
                        }
                    }
                    continue;
                }
                _ => {}
            }
        }
        prev_was_pass = false;
        merged.push(new);
    }
    Ok(RawBlock { loc: block_loc, statements: merged, animation })
}

fn parse_multipurpose(l: &mut Lexer, cx: &dyn ParseContext, loc: Loc) -> PResult<RawMultipurpose> {
    let mut rm = RawMultipurpose { loc: Some(loc), ..Default::default() };
    let cp = l.checkpoint();
    let w = l.name();
    match w.as_deref() {
        Some(name) if cx.is_warper(name) => {
            rm.warper = Some(name.to_string());
            rm.duration = Some(l.require_simple()?);
        }
        Some("warp") => {
            rm.warp_function = Some(l.require_simple()?);
            rm.duration = Some(l.require_simple()?);
        }
        _ => {
            l.revert(&cp);
            rm.duration = Some(l.expr("0".into()));
        }
    }
    let warped = rm.warper.is_some() || rm.warp_function.is_some();
    let mut sub: Option<Lexer> = None;
    let mut last_expression = false;
    let mut this_expression = false;
    loop {
        let has_sub = sub.is_some();
        if warped && !has_sub && l.match_re(":").is_some() {
            l.expect_eol()?;
            l.expect_block("ATL")?;
            let mut s = l.subblock_lexer(false);
            s.advance();
            s.expect_noblock("ATL")?;
            sub = Some(s);
            continue;
        }
        let ll: &mut Lexer = match sub.as_mut() {
            Some(s) => s,
            None => &mut *l,
        };
        if has_sub && ll.eol() {
            if !ll.advance() {
                break;
            }
            ll.expect_noblock("ATL")?;
        }
        last_expression = this_expression;
        this_expression = false;
        if ll.keyword("pass") {
            continue;
        }
        if ll.keyword("clockwise") {
            rm.revolution = Some("clockwise".into());
            continue;
        }
        if ll.keyword("counterclockwise") {
            rm.revolution = Some("counterclockwise".into());
            continue;
        }
        if ll.keyword("circles") {
            rm.circles = Some(ll.require_simple()?);
            continue;
        }
        let cp = ll.checkpoint();
        let prop = ll.name();
        if let Some(p) = prop.filter(|p| is_property(p)) {
            let expr = ll.require_simple()?;
            let mut knots = Vec::new();
            while ll.keyword("knot") {
                knots.push(ll.require_simple()?);
            }
            if knots.is_empty() {
                rm.properties.push((p, expr));
            } else {
                if p == "orientation" {
                    return ll.error("Orientation doesn't support spline.");
                }
                knots.push(expr);
                rm.splines.push((p, knots));
            }
            continue;
        }
        ll.revert(&cp);
        let Some(expr) = ll.simple_expression()? else { break };
        if last_expression {
            return ll.error("ATL statement contains two expressions in a row; is one of them a misspelled property? If not, separate them with pass.");
        }
        this_expression = true;
        let with = if ll.keyword("with") { Some(ll.require_simple()?) } else { None };
        rm.expressions.push((expr, with));
    }
    if sub.is_none() {
        l.expect_noblock("ATL")?;
    }
    Ok(rm)
}
