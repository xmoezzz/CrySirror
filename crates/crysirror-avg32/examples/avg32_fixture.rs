// SPDX-License-Identifier: MPL-2.0
//! Generate self-authored native TPC32 resources, never game data/fonts.
use serde_json::json;
use std::path::PathBuf;
fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or_else(|| {
        anyhow::anyhow!("usage: cargo run -p crysirror-avg32 --example avg32_fixture -- OUTPUT_DIR")
    })?);
    std::fs::create_dir_all(root.join("assets"))?;
    anyhow::ensure!(
        !root.join("project.json").exists(),
        "output project already exists"
    );
    let mut bg = image::RgbaImage::new(640, 480);
    for (x, y, p) in bg.enumerate_pixels_mut() {
        *p = image::Rgba([20 + (x / 8) as u8, 30 + (y / 5) as u8, 90, 255]);
    }
    bg.save(root.join("assets/background.png"))?;
    image::RgbaImage::from_pixel(12, 12, image::Rgba([250u8, 200, 80, 255]))
        .save(root.join("assets/icon.png"))?;
    let mut code = vec![0x0b, 0x09];
    code.extend_from_slice(b"background\0");
    code.push(0x10);
    code.push(0xfe);
    code.extend_from_slice(b"AVG32 -> CrySirror APIs\0");
    code.extend_from_slice(&[0x03, 0x60, 0x20]);
    let mut seen = vec![0; 0x63];
    seen[..5].copy_from_slice(b"TPC32");
    seen[0x28..0x2c].copy_from_slice(&1i32.to_le_bytes());
    seen[0x5f..0x63].copy_from_slice(&1i32.to_le_bytes());
    seen.extend(code);
    std::fs::write(root.join("assets/SEEN001.TXT"), seen)?;
    let project = json!({"schema":2,"name":"AVG32 native API proof","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"avg32","game":"self-authored-tpc32","config":{"scenes":{"1":"SEEN001.TXT"},"version":1604,"resources":{"background":"background.png"},"message_rect":[36,360,568,80],"font_size":24,"cell_step":[18,30],"message_interval_ms":20,"message_icon":"icon.png"}}]});
    std::fs::write(
        root.join("project.json"),
        serde_json::to_vec_pretty(&project)?,
    )?;
    println!("{}", root.join("project.json").display());
    Ok(())
}
