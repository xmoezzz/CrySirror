// SPDX-License-Identifier: MPL-2.0
//! Developer batch adapter over engine CPU codecs; capability is manifest parent.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    #[serde(default)]
    indices: Vec<Indices>,
    #[serde(default)]
    textures: Vec<Texture>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Indices {
    file: String,
    offset: usize,
    length: usize,
    index_count: usize,
    vertex_count: usize,
    encoding: crysirror_asset::indices::IndexEncoding,
    #[serde(default)]
    reverse_winding: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Texture {
    file: String,
    output: String,
    max_size: u32,
}
fn input(root: &Path, name: &str) -> Result<PathBuf> {
    let path = root.join(name).canonicalize()?;
    ensure!(
        path.starts_with(root) && path.is_file(),
        "input escapes capability root"
    );
    Ok(path)
}
fn output(root: &Path, name: &str) -> Result<std::fs::File> {
    let out = root.join(name);
    let parent = out.parent().context("output parent")?.canonicalize()?;
    ensure!(
        parent.starts_with(root) && out.extension().is_some_and(|s| s == "png"),
        "texture output escapes root or is not PNG"
    );
    // Atomic creation also rejects dangling symlinks and concurrent overwrites.
    Ok(std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(parent.join(out.file_name().context("output filename")?))?)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    ensure!(args.len() == 3, "transcode <manifest.json> <result.json>");
    let path = Path::new(&args[1]).canonicalize()?;
    let root = path.parent().context("manifest parent")?;
    let bytes = std::fs::read(&path)?;
    ensure!(bytes.len() <= 8 * 1024 * 1024, "manifest too large");
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    ensure!(
        manifest.schema == 1 && manifest.indices.len() <= 65536 && manifest.textures.len() <= 4096,
        "manifest version/limits"
    );
    let mut buffers = std::collections::HashMap::new();
    let mut indices = Vec::new();
    for job in manifest.indices {
        let file = input(root, &job.file)?;
        if !buffers.contains_key(&file) {
            ensure!(
                file.metadata()?.len() <= 64 * 1024 * 1024,
                "input too large"
            );
            buffers.insert(file.clone(), std::fs::read(&file)?);
        }
        let data = &buffers[&file];
        let end = job
            .offset
            .checked_add(job.length)
            .context("index range overflow")?;
        let slice = data.get(job.offset..end).context("index range")?;
        let mut ix = crysirror_asset::indices::decode(
            slice,
            job.encoding,
            job.index_count,
            job.vertex_count,
        )?;
        if job.reverse_winding {
            for tri in ix.chunks_exact_mut(3) {
                tri.swap(1, 2);
            }
        }
        indices.push(ix);
    }
    let mut textures = Vec::new();
    for job in manifest.textures {
        ensure!(
            (1..=2048).contains(&job.max_size),
            "texture preview dimensions"
        );
        let path = input(root, &job.file)?;
        ensure!(
            path.metadata()?.len() <= 64 * 1024 * 1024,
            "texture input too large"
        );
        let surface = crysirror_image::decode_standard(&std::fs::read(&path)?)?;
        let image = image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(surface.width, surface.height, surface.rgba)
                .context("surface")?,
        )
        .thumbnail(job.max_size, job.max_size)
        .to_rgba8();
        image.write_to(
            &mut output(root, &job.output)?,
            image::ImageOutputFormat::Png,
        )?;
        textures.push(
            serde_json::json!({"file":job.output,"width":image.width(),"height":image.height()}),
        );
    }
    std::fs::write(
        &args[2],
        serde_json::to_vec(&serde_json::json!({"schema":1,"indices":indices,"textures":textures}))?,
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_paths_and_existing_or_dangling_outputs_are_rejected() {
        let root = std::env::temp_dir().join(format!("crysirror-transcode-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("source.bin");
        std::fs::write(&path, b"synthetic").unwrap();
        assert_eq!(input(&root, "source.bin").unwrap(), path);
        assert!(input(&root, "../outside.bin").is_err());
        assert!(output(&root, "../outside.png").is_err());
        assert!(output(&root, "wrong.jpg").is_err());
        drop(output(&root, "preview.png").unwrap());
        assert!(output(&root, "preview.png").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("missing.bin"), root.join("dangling.png"))
                .unwrap();
            assert!(output(&root, "dangling.png").is_err());
            assert!(!root.join("missing.bin").exists());
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
