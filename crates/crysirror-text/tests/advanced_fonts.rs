// SPDX-License-Identifier: MPL-2.0
//! Caller-supplied system fonts only. No font fixture is stored in this crate.
use crysirror_text::{
    layout::{LayoutOptions, WritingMode},
    raster::{IconBitmap, Outline, RasterOptions, Shadow},
    rich::{RichText, TextStyle},
    sdf::SdfOptions,
    TextSystem,
};
fn fonts() -> TextSystem {
    let mut system = TextSystem::default();
    for (alias, var) in [
        ("latin", "CRYSIRROR_TEST_FONT"),
        ("cjk", "CRYSIRROR_TEST_CJK_FONT"),
        ("arabic", "CRYSIRROR_TEST_ARABIC_FONT"),
    ] {
        let file = std::env::var_os(var)
            .unwrap_or_else(|| panic!("provide a licensed local font via {var}"));
        system
            .add_font(alias, std::fs::read(file).unwrap())
            .unwrap();
    }
    system
}
#[test]
#[ignore = "requires licensed system font paths: CRYSIRROR_TEST_FONT / _CJK_FONT / _ARABIC_FONT"]
fn supplied_fonts_shape_fallback_arabic_bidi_ruby_and_vertical() {
    let system = fonts();
    let mut style = TextStyle::new("latin", 28.);
    style.fonts = vec!["latin".into(), "cjk".into(), "arabic".into()];
    let document=RichText::parse("office 漢字 [color=#ff0000]אבג[/color]\n[ruby=にほんご]日本語[/ruby] [icon=icons/check.png,20,20]",style.clone()).unwrap();
    let layout = system
        .layout_rich(
            &document,
            &LayoutOptions {
                canvas: [500, 180],
                ..Default::default()
            },
        )
        .unwrap();
    assert!(layout.glyphs.iter().any(|g| g.font == "cjk"));
    assert!(layout.glyphs.iter().any(|g| g.ruby));
    assert_eq!(layout.icons.len(), 1);
    assert!(!layout.clipped);
    let rtl = layout
        .glyphs
        .iter()
        .filter(|g| g.color == [255, 0, 0, 255])
        .collect::<Vec<_>>();
    assert_eq!(rtl.len(), 3);
    assert!(rtl
        .windows(2)
        .all(|g| g[0].bytes.start > g[1].bytes.start && g[0].position[0] < g[1].position[0]));
    let arabic = RichText::plain("سلام", TextStyle::new("arabic", 32.));
    let a = system
        .layout_rich(&arabic, &LayoutOptions::default())
        .unwrap();
    assert!(
        a.glyphs.len() < 4,
        "Arabic lam-alef must be shaped together"
    );
    assert!(a
        .glyphs
        .iter()
        .all(|g| g.font == "arabic" && g.glyph_id > 0));
    let combining = system
        .layout_rich(
            &RichText::plain("e\u{301}", style),
            &LayoutOptions::default(),
        )
        .unwrap();
    assert_eq!(combining.grapheme_count, 1);
    assert!(combining.glyphs.iter().all(|g| g.graphemes == (0..1)));
    let vertical = RichText::parse(
        "[ruby=にほん]日本[/ruby]語。日本語",
        TextStyle::new("cjk", 28.),
    )
    .unwrap();
    let v = system
        .layout_rich(
            &vertical,
            &LayoutOptions {
                canvas: [180, 100],
                writing: WritingMode::VerticalRl,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(v.lines.len() > 1);
    let base = v.glyphs.iter().filter(|g| !g.ruby).collect::<Vec<_>>();
    assert!(base[1].position[1] > base[0].position[1]);
    assert!(v
        .glyphs
        .iter()
        .filter(|g| g.ruby)
        .all(|g| g.position[0] > base[0].position[0]));
    assert!(system
        .layout_rich(
            &RichText::plain("x", TextStyle::new("not-loaded", 16.)),
            &LayoutOptions::default()
        )
        .is_err());
}
#[test]
#[ignore = "requires licensed system font paths: CRYSIRROR_TEST_FONT / _CJK_FONT / _ARABIC_FONT"]
fn supplied_fonts_raster_color_outline_shadow_reveal_icons_and_sdf_dedup() {
    let system = fonts();
    let doc = RichText::parse(
        "[b][color=#ff0000]AAA[/color][/b] [icon=icons/pixel.png,16,16]",
        TextStyle::new("latin", 36.),
    )
    .unwrap();
    let layout = system
        .layout_rich(
            &doc,
            &LayoutOptions {
                canvas: [256, 96],
                ..Default::default()
            },
        )
        .unwrap();
    let icon = |_: &str| {
        Ok(IconBitmap {
            width: 1,
            height: 1,
            rgba: vec![0, 255, 0, 255],
        })
    };
    let raw = system
        .rasterize_rich(&layout, &RasterOptions::default(), icon)
        .unwrap();
    assert!(raw
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[0] > 200 && p[1] < 10 && p[3] > 100));
    assert!(raw
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[1] > 200 && p[0] < 10 && p[3] > 100));
    let styled = system
        .rasterize_rich(
            &layout,
            &RasterOptions {
                outline: Some(Outline {
                    width: 2.,
                    color: [0, 0, 255, 255],
                }),
                shadow: Some(Shadow {
                    offset: [4., 4.],
                    blur_radius: 2,
                    color: [0, 0, 0, 160],
                }),
                ..Default::default()
            },
            icon,
        )
        .unwrap();
    assert!(styled
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[2] > 200 && p[0] < 10));
    assert!(
        styled
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .count()
            > raw.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
    );
    let empty = system
        .rasterize_rich(
            &layout,
            &RasterOptions {
                visible_graphemes: Some(0),
                ..Default::default()
            },
            icon,
        )
        .unwrap();
    assert!(empty.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    let atlas = system
        .build_sdf_atlas(
            &layout,
            &SdfOptions {
                page_size: 128,
                spread: 4,
                max_pages: 2,
            },
        )
        .unwrap();
    assert_eq!(
        atlas.glyphs.len(),
        1,
        "three identical A glyphs share one entry; space has no outline"
    );
    assert_eq!(atlas.placements.len(), 3);
    assert_eq!(atlas.pages.len(), 1);
    assert!(atlas.pages[0].pixels.iter().any(|p| *p > 128));
    assert_eq!(atlas.icons.len(), 1);
    let g = atlas.glyphs.values().next().unwrap();
    assert!(g.uv[0] < g.uv[2] && g.uv[1] < g.uv[3]);
}
