// SPDX-License-Identifier: MPL-2.0
//! Caller-supplied font paths; all derived images must remain outside the repo.
use anyhow::{ensure, Result};
use crysirror_text::{
    layout::{LayoutOptions, WritingMode},
    raster::{IconBitmap, Outline, RasterOptions, Shadow},
    rich::{RichText, TextStyle},
    sdf::SdfOptions,
    TextSystem,
};
use std::path::{Path, PathBuf};
fn main() -> Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 4,
        "rich_preview <latin font> <CJK font> <Arabic font> <external output directory>"
    );
    let out = PathBuf::from(&args[3]);
    ensure!(out.is_absolute(), "output must be absolute");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    ensure!(
        !out.starts_with(root),
        "derived font images stay outside source export"
    );
    std::fs::create_dir_all(&out)?;
    let out = out.canonicalize()?;
    ensure!(
        !out.starts_with(root.canonicalize()?),
        "output symlink escapes external directory"
    );
    let mut system = TextSystem::default();
    for (alias, file) in ["latin", "cjk", "arabic"].into_iter().zip(&args[..3]) {
        system.add_font(alias, std::fs::read(file)?)?;
    }
    let style = TextStyle {
        fonts: vec!["latin".into(), "cjk".into(), "arabic".into()],
        size: 26.,
        color: [235, 240, 249, 255],
        bold: false,
    };
    let document=RichText::parse("[size=34][b]CrySirror: shaped CPU text[/b][/size]\n\nLatin kerning: AV office / e\u{301}\nHebrew: [color=#6fd8ff]אבג שלום[/color]\nArabic: [font=arabic][color=#ffcf73]سلام عليكم[/color][/font]\n\nCJK fallback: 日本語 中文\nRuby: [ruby=にほんご]日本語[/ruby] + [ruby=かんじ]漢字[/ruby]\n\n[icon=icons/check.png,24,24] [color=#88eeaa]Inline icon, rich colors, outline, shadow[/color]\nUAX #14 wrapping + CJK prohibition tailoring",style)?;
    let layout = system.layout_rich(
        &document,
        &LayoutOptions {
            canvas: [820, 440],
            ..Default::default()
        },
    )?;
    let pixels = system.rasterize_rich(
        &layout,
        &RasterOptions {
            outline: Some(Outline {
                width: 1.,
                color: [0, 0, 0, 255],
            }),
            shadow: Some(Shadow {
                offset: [2., 3.],
                blur_radius: 2,
                color: [0, 0, 0, 170],
            }),
            ..Default::default()
        },
        |_| {
            Ok(IconBitmap {
                width: 1,
                height: 1,
                rgba: vec![70, 230, 130, 255],
            })
        },
    )?;
    let vertical = RichText::parse(
        "[ruby=にほんご]日本語[/ruby]の縦書き。文字、注音。",
        TextStyle {
            color: [245, 220, 170, 255],
            ..TextStyle::new("cjk", 27.)
        },
    )?;
    let v = system.layout_rich(
        &vertical,
        &LayoutOptions {
            canvas: [240, 440],
            writing: WritingMode::VerticalRl,
            ..Default::default()
        },
    )?;
    let vp = system.rasterize_rich(&v, &RasterOptions::default(), |_| unreachable!())?;
    let mut image = image::RgbaImage::from_pixel(1080, 480, image::Rgba([20, 25, 34, 255]));
    for (rgba, size, offset) in [
        (&pixels, [820u32, 440u32], [20u32, 20u32]),
        (&vp, [240u32, 440u32], [840u32, 20u32]),
    ] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let at = (y as usize * size[0] as usize + x as usize) * 4;
                let a = rgba[at + 3] as u16;
                let p = image.get_pixel_mut(x + offset[0], y + offset[1]);
                for c in 0..3 {
                    p[c] = ((rgba[at + c] as u16 * a + p[c] as u16 * (255 - a) + 127) / 255) as u8;
                }
            }
        }
    }
    image.save(out.join("rich-text.png"))?;
    let atlas = system.build_sdf_atlas(
        &layout,
        &SdfOptions {
            page_size: 512,
            spread: 6,
            max_pages: 4,
        },
    )?;
    for (i, page) in atlas.pages.iter().enumerate() {
        image::GrayImage::from_raw(page.width, page.height, page.pixels.clone())
            .unwrap()
            .save(out.join(format!("sdf-atlas-{i}.png")))?;
    }
    println!(
        "rich-text.png: {} glyphs, {} lines; SDF: {} unique glyphs / {} pages; clipped={}",
        layout.glyphs.len(),
        layout.lines.len(),
        atlas.glyphs.len(),
        atlas.pages.len(),
        layout.clipped
    );
    Ok(())
}
