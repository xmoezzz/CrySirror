// SPDX-License-Identifier: MPL-2.0
//! Device-free BGI runtime probe. All project/game/font/capture files stay
//! outside the source tree. Example:
//! cargo run -p crysirror-bgi --example native_trace -- /absolute/project.json \
//!   /absolute/state 600 --click 400 640 350 --capture /absolute/frame.png
//! `--capture-at 399 /absolute/before.png` records pixels and native diagnostics
//! after that zero-based input frame, without advancing or modifying the VM.
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::{buttons, InputFrame};
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use std::{collections::BTreeMap, path::PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let project_path = PathBuf::from(args.next().context("project path required")?);
    let state = PathBuf::from(args.next().context("state directory required")?);
    let frames: u32 = args.next().context("frame count required")?.parse()?;
    ensure!(
        (1..=1_000_000).contains(&frames),
        "frame count must be 1..1000000"
    );
    let mut clicks = BTreeMap::new();
    let mut enter = std::collections::BTreeSet::new();
    let mut capture = None;
    let mut capture_at = BTreeMap::new();
    while let Some(option) = args.next() {
        match option.as_str() {
            "--click" => {
                let frame: u32 = args.next().context("click frame required")?.parse()?;
                let x: i32 = args.next().context("click X required")?.parse()?;
                let y: i32 = args.next().context("click Y required")?.parse()?;
                ensure!(
                    frame < frames.saturating_sub(1) && clicks.insert(frame, (x, y)).is_none(),
                    "invalid/duplicate click frame"
                );
            }
            "--enter" => {
                let frame: u32 = args.next().context("Enter frame required")?.parse()?;
                ensure!(
                    frame < frames && enter.insert(frame),
                    "invalid/duplicate Enter frame"
                );
            }
            "--capture" => {
                capture = Some(PathBuf::from(args.next().context("capture path required")?))
            }
            "--capture-at" => {
                let frame: u32 = args.next().context("capture frame required")?.parse()?;
                let path = PathBuf::from(args.next().context("capture path required")?);
                ensure!(
                    frame < frames && capture_at.insert(frame, path).is_none(),
                    "invalid/duplicate capture frame"
                );
            }
            _ => bail!("unknown option {option}"),
        }
    }
    let mut previous = None;
    for &frame in clicks.keys() {
        ensure!(
            previous.is_none_or(|last| frame - last > 1),
            "clicks need a release frame between them"
        );
        previous = Some(frame);
    }
    for path in capture.iter().chain(capture_at.values()) {
        ensure!(path.is_absolute(), "capture path must be absolute");
        let parent = path
            .parent()
            .context("capture parent required")?
            .canonicalize()?;
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?;
        ensure!(
            !parent.starts_with(repository),
            "game captures must stay outside the repository"
        );
        ensure!(!path.is_symlink(), "capture must not be a symlink");
    }
    let project = Project::load(&project_path)?;
    let client = project
        .clients
        .iter()
        .find(|client| client.adapter == "bgi")
        .context("project requires an original BGI client")?
        .name
        .clone();
    let mut registry = AdapterRegistry::default();
    crysirror_bgi::register(&mut registry)?;
    let mut runtime = Runtime::with_registry(project, &project_path, &state, registry)?;
    let mut pointer = (0, 0);
    let mut checkpoints = Vec::new();
    let mut pcm_frames = 0u64;
    let mut pcm_peak = 0f32;
    let mut pcm_hash = 0xcbf29ce484222325u64;
    for frame in 0..frames {
        let mut input = InputFrame::default();
        if let Some(&(x, y)) = clicks.get(&frame) {
            pointer = (x, y);
            input.buttons_down |= buttons::MOUSE_LEFT;
            input.buttons_pressed |= buttons::MOUSE_LEFT;
        }
        if enter.contains(&frame) {
            input.buttons_pressed |= buttons::ENTER;
        }
        input.pointer_x = pointer.0;
        input.pointer_y = pointer.1;
        runtime
            .tick(if frame == 0 { 0 } else { 16 }, input)
            .with_context(|| format!("BGI runtime frame {frame}"))?;
        if let Some(path) = capture_at.get(&frame) {
            write_capture(&runtime, &client, path)?;
            let inspect = runtime.inspect();
            checkpoints.push(serde_json::json!({"after_input_frame":frame,"capture":path,
                "runtime_frame":inspect["frame"],"time_ms":inspect["time_ms"],"sessions":inspect["sessions"]}));
        }
        let pcm = runtime.host.take_pcm_output();
        pcm_frames += pcm.len() as u64;
        for sample in pcm.iter().flatten() {
            pcm_peak = pcm_peak.max(sample.abs());
            for byte in sample.to_bits().to_le_bytes() {
                pcm_hash = (pcm_hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    if let Some(path) = &capture {
        write_capture(&runtime, &client, path)?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "runtime":runtime.inspect(),"capture":capture,"checkpoints":checkpoints,"gpu_created":false,
            "audio_device_opened":false,"native_whole_frame_renderer":false,
            "pcm_frames":pcm_frames,"pcm_peak":pcm_peak,"pcm_fnv1a":pcm_hash,
        }))?
    );
    Ok(())
}

fn write_capture(runtime: &Runtime, client: &str, path: &std::path::Path) -> Result<()> {
    let id = runtime.client_id(client).context("BGI client missing")?;
    let surface = runtime
        .assets
        .surface(id, "__surface/0")
        .context("BGI has not presented a frame")?;
    let png = crysirror_image::encode_png(&crysirror_image::ImageSurface {
        width: surface.width,
        height: surface.height,
        rgba: surface.rgba.to_vec(),
    })?;
    std::fs::write(path, png)?;
    Ok(())
}
