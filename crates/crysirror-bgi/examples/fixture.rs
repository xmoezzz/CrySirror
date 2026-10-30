//! Build an authored native BGI scene in a local directory. Optional font is
//! copied from an explicitly supplied owner/system font at runtime only.
use anyhow::{Context, Result};
use std::path::PathBuf;
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let out = PathBuf::from(
        args.next()
            .context("usage: fixture OUTPUT_DIR [SYSTEM_FONT]")?,
    );
    let font = args.next().map(PathBuf::from);
    std::fs::create_dir_all(out.join("assets"))?;
    std::fs::write(
        out.join("assets/main._bp"),
        crysirror_bgi::fixture::scene(font.is_some(), true, true),
    )?;
    let png = crysirror_image::encode_png_uncompressed(&crysirror_image::ImageSurface {
        width: 2,
        height: 2,
        rgba: [240, 50, 20, 255].repeat(4),
    })?;
    std::fs::write(out.join("assets/pixel.png"), png)?;
    let pcm = (0..8000)
        .map(|i| (((i as f64 / 8000. * 440. * std::f64::consts::TAU).sin()) * 10000.) as i16)
        .collect::<Vec<_>>();
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + pcm.len() as u32 * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((pcm.len() as u32 * 2).to_le_bytes());
    for sample in pcm {
        wav.extend(sample.to_le_bytes());
    }
    std::fs::write(out.join("assets/tone.wav"), wav)?;
    let mut fonts = serde_json::Map::new();
    if let Some(path) = font {
        std::fs::copy(path, out.join("assets/font.bin"))?;
        fonts.insert("main".into(), "font.bin".into());
    }
    let config = if fonts.is_empty() {
        serde_json::json!({"script":"main._bp"})
    } else {
        serde_json::json!({"script":"main._bp","font":"main"})
    };
    let project = serde_json::json!({"schema":2,"name":"Authored native BGI profile","content_revision":"1","canvas":[320,180],"assets":"assets","fonts":fonts,"clients":[{"name":"bgi","adapter":"bgi","game":"authored-native-bp","config":config}]});
    std::fs::write(
        out.join("project.json"),
        serde_json::to_vec_pretty(&project)?,
    )?;
    println!("{}", out.join("project.json").display());
    Ok(())
}
