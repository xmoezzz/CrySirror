// SPDX-License-Identifier: MPL-2.0
//! Device-free native display-list export. Layout/reveal and native resource
//! resolution remain here; glyph rasterization and composition are host services.
use crate::{RuntimeTraceApi, graph::RuntimeGraphDrawItem, snapshot::TextNodeShadow};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use ethornell_core::{EthornellError, Result};
pub use ethornell_image::DecodedImage as FrameImage;
use ethornell_image::DecodedImage;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct FramePlan {
    pub width: u32,
    pub height: u32,
    pub images: BTreeMap<String, DecodedImage>,
    /// Authoritative draw order, including each glyph's shadow before its ink.
    pub draws: Vec<FrameDraw>,
}
#[derive(Clone, Debug)]
pub enum FrameDraw {
    Image(ImageDraw),
    Glyph(GlyphDraw),
}
#[derive(Clone, Debug)]
pub struct ImageDraw {
    pub key: String,
    pub destination: [f32; 4],
    pub source: [f32; 4],
    pub opacity: f32,
    pub ignore_source_alpha: bool,
    pub blend_mode: i32,
    /// TL, BL, BR, TR; includes native mode-2 rotation and mode-5 geometry.
    pub destination_quad: Option<[[f32; 2]; 4]>,
    pub linear_sampling: bool,
    pub clip: Option<[f32; 4]>,
    pub native_priority: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct GlyphEffect {
    pub mode: i32,
    pub dx: i32,
    pub dy: i32,
    pub rgb: u32,
    pub concentration: i32,
}
#[derive(Clone, Debug)]
pub struct GlyphDraw {
    pub glyph_id: u32,
    pub baseline: [f32; 2],
    /// Integer post-raster translation, preserving native shadow coverage.
    pub offset: [i32; 2],
    pub size: f32,
    pub color: [f32; 4],
    pub bold: bool,
    pub italic: bool,
    pub clip: Option<[f32; 4]>,
    pub effect: Option<GlyphEffect>,
}
fn fail(message: &str) -> EthornellError {
    EthornellError::Other(message.into())
}
fn push_image(
    api: &RuntimeTraceApi,
    plan: &mut FramePlan,
    item: RuntimeGraphDrawItem,
) -> Result<()> {
    // The original rasterizer retains unknown property11 kinds as a no-op.
    // A shared-service adapter must expose that missing capability instead.
    if let Some(owner) = item.owner_object {
        if api.display_tree.kind(owner) == Some(crate::display_tree::NativeDisplayKind::Sprite) {
            if let Some(properties) = api.graph_object_properties.get(&owner) {
                for index in 0..16u32 {
                    if let Some((packed, _)) = properties.properties.get(&(0x1100 | index)) {
                        let kind = ((*packed as u32 >> 8) & 255) as u8;
                        if kind > 5 {
                            return Err(fail(&format!(
                                "unsupported native sprite filter {kind} object {owner}"
                            )));
                        }
                    }
                }
            }
        }
    }
    let Some(image) = api.graph_images.get(&item.key) else {
        return Ok(());
    };
    let quad = crate::native_sprite_filters::item_quad(&item);
    if !plan.images.contains_key(&item.key) {
        let len = u64::from(image.width) * u64::from(image.height) * 4;
        if image.width == 0
            || image.height == 0
            || len != image.rgba.len() as u64
            || len > 128 * 1024 * 1024
        {
            return Err(fail("invalid native frame image"));
        }
        let total = plan.images.values().map(|i| i.rgba.len()).sum::<usize>();
        if total.saturating_add(image.rgba.len()) > 256 * 1024 * 1024 {
            return Err(fail("native frame image budget exceeded"));
        }
        plan.images.insert(item.key.clone(), image.clone());
    }
    plan.draws.push(FrameDraw::Image(ImageDraw {
        key: item.key,
        destination: [item.x, item.y, item.width, item.height],
        source: [item.src_x, item.src_y, item.src_width, item.src_height],
        opacity: item.opacity,
        ignore_source_alpha: item.ignore_source_alpha,
        blend_mode: item.blend_mode,
        destination_quad: quad,
        linear_sampling: item.linear_sampling,
        clip: item.clip.map(|c| [c.x, c.y, c.width, c.height]),
        native_priority: item.z,
    }));
    Ok(())
}
fn push_glyph(
    plan: &mut FramePlan,
    font: &FontArc,
    ch: char,
    size: f32,
    baseline: [f32; 2],
    color: [f32; 4],
    bold: bool,
    italic: bool,
    clip: Option<[f32; 4]>,
    shadow: TextNodeShadow,
) -> Result<()> {
    let glyph_id = font.glyph_id(ch);
    if glyph_id.0 == 0 && !ch.is_control() {
        return Err(fail(&format!(
            "native frame font has no glyph for U+{:04X}",
            ch as u32
        )));
    }
    if !size.is_finite()
        || size <= 0.
        || size > 1024.
        || baseline
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
    {
        return Err(fail("invalid native glyph placement"));
    }
    let mut glyph = GlyphDraw {
        glyph_id: u32::from(glyph_id.0),
        baseline,
        offset: [0, 0],
        size,
        color,
        bold,
        italic,
        clip,
        effect: None,
    };
    match shadow {
        TextNodeShadow::Message(Some(effect)) if effect.mode == 2 => {
            glyph.bold = false;
            glyph.italic = false;
            // Preserve the native placed-record top/baseline round trip in f32.
            let ascent = font.as_scaled(PxScale::from(size)).ascent();
            glyph.baseline[1] = (glyph.baseline[1] - ascent) + ascent;
            glyph.effect = Some(GlyphEffect {
                mode: effect.mode,
                dx: effect.dx,
                dy: effect.dy,
                rgb: effect.rgb,
                concentration: effect.concentration,
            });
        }
        TextNodeShadow::Message(Some(effect)) => {
            if effect.mode != 1 {
                return Err(fail("unsupported native glyph shadow mode"));
            }
            let mut under = glyph.clone();
            under.offset = [effect.dx, effect.dy];
            under.color = [
                ((effect.rgb >> 16) & 255) as f32 / 255.,
                ((effect.rgb >> 8) & 255) as f32 / 255.,
                (effect.rgb & 255) as f32 / 255.,
                color[3] * effect.concentration as f32 / 256.,
            ];
            plan.draws.push(FrameDraw::Glyph(under));
        }
        TextNodeShadow::Default(Some((xp, yp, concentration))) => {
            let height = size.max(0.).round() as i32;
            let mut under = glyph.clone();
            under.offset = [
                height.saturating_mul(xp) / 100,
                height.saturating_mul(yp) / 100,
            ];
            under.color = [
                0.,
                0.,
                0.,
                color[3] * (256 - concentration.clamp(0, 256)) as f32 / 256.,
            ];
            plan.draws.push(FrameDraw::Glyph(under));
        }
        _ => {}
    }
    plan.draws.push(FrameDraw::Glyph(glyph));
    if plan.draws.len() > 65_536 {
        return Err(fail("native frame draw budget exceeded"));
    }
    Ok(())
}
/// Export the same native image/text/movie sequence as the CPU reference, without
/// invoking its whole-frame compositor or any native glyph rasterizer.
pub(crate) fn capture(api: &RuntimeTraceApi, font: &FontArc) -> Result<FramePlan> {
    let (width, height) = crate::snapshot::runtime_frame_size(api);
    let mut plan = FramePlan {
        width,
        height,
        images: BTreeMap::new(),
        draws: Vec::new(),
    };
    for item in api.graph_output_draw_items() {
        push_image(api, &mut plan, item)?;
    }
    for node in api
        .text_nodes
        .values()
        .filter(|node| api.should_draw_text_node(node))
    {
        let mut node = node.clone();
        node.style_spans = api.text_node_style_spans(&node);
        if let Some(surface) = node
            .target_surface
            .filter(|s| api.graph_surfaces.contains_key(s))
        {
            let (x, y) = api.surface_world_position(surface);
            node.x += x;
            node.y += y;
            node.color[3] *= api.surface_display_opacity(surface);
        }
        let clip = node
            .target_surface
            .and_then(|s| api.surface_chain_clip(s))
            .map(|c| [c.x, c.y, c.width, c.height]);
        let layout = api.formatted_text_layout(&node);
        let shadow = api.text_node_shadow(&node);
        // Body ink clamps its scale to 12px, but the original default shadow
        // offset is computed from the unclamped node.size.
        let body_shadow = match shadow {
            TextNodeShadow::Default(Some((xp, yp, concentration))) => {
                let height = node.size.max(0.).round() as i32;
                TextNodeShadow::Message(Some(crate::snapshot::TextShadow {
                    mode: 1,
                    dx: height.saturating_mul(xp) / 100,
                    dy: height.saturating_mul(yp) / 100,
                    rgb: 0,
                    concentration: 256 - concentration.clamp(0, 256),
                }))
            }
            value => value,
        };
        let size = node.size.max(12.);
        let scaled = font.as_scaled(PxScale::from(size));
        let max_x = (width as f32 - 36.).max(node.x + 64.);
        let mut x = node.x;
        let mut y = node.y + layout.body_y_offset + scaled.ascent();
        for (index, ch) in node.text.chars().enumerate() {
            if ch == '\n' {
                x = node.x;
                y += layout.line_height;
                continue;
            }
            let style = node
                .style_spans
                .iter()
                .find(|s| s.start_char <= index && index < s.end_char);
            let color = style
                .and_then(|s| s.style.packed_rgb)
                .map(|rgb| {
                    [
                        ((rgb >> 16) & 255) as f32 / 255.,
                        ((rgb >> 8) & 255) as f32 / 255.,
                        (rgb & 255) as f32 / 255.,
                        node.color[3],
                    ]
                })
                .unwrap_or(node.color);
            let advance = scaled.h_advance(font.glyph_id(ch)).max(node.size * 0.5);
            if x + advance > max_x {
                x = node.x;
                y += layout.line_height;
            }
            push_glyph(
                &mut plan,
                font,
                ch,
                size,
                [x, y],
                color,
                style.is_some_and(|s| s.style.bold),
                style.is_some_and(|s| s.style.italic),
                clip,
                body_shadow,
            )?;
            x += advance;
        }
        let ruby_shadow = match shadow {
            TextNodeShadow::Default(value) => TextNodeShadow::Default(value),
            _ => TextNodeShadow::Default(None),
        };
        for (ruby, mut x, y, size) in crate::text::ruby_draw_runs(&node, layout) {
            let scaled = font.as_scaled(PxScale::from(size));
            let baseline = y + scaled.ascent();
            for ch in ruby.chars() {
                push_glyph(
                    &mut plan,
                    font,
                    ch,
                    size,
                    [x, baseline],
                    node.color,
                    false,
                    false,
                    clip,
                    ruby_shadow,
                )?;
                x += scaled.h_advance(font.glyph_id(ch)).max(size * 0.5);
            }
        }
    }
    if let Some(item) = api.directshow_movie_draw_item() {
        push_image(api, &mut plan, item)?;
    }
    if plan.draws.len() > 65_536 {
        return Err(fail("native frame draw budget exceeded"));
    }
    Ok(plan)
}
