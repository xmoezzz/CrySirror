//! Device-free native VM/service diagnostic; a successful probe does not verify GPU pixels.
use anyhow::{Context, Result};
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use std::path::PathBuf;
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let project = PathBuf::from(
        args.next()
            .context("usage: native_probe PROJECT STATE_DIR [FRAMES]")?,
    );
    let state = PathBuf::from(args.next().context("STATE_DIR required")?);
    let frames: u32 = args
        .next()
        .map(|a| a.to_string_lossy().parse())
        .transpose()?
        .unwrap_or(1);
    let mut registry = AdapterRegistry::default();
    crysirror_rfvp::register(&mut registry)?;
    let mut runtime = Runtime::with_registry(Project::load(&project)?, &project, &state, registry)?;
    for frame in 0..frames {
        if let Err(error) = runtime.tick(16, InputFrame::default()) {
            let report = runtime.inspect();
            eprintln!(
                "{}",
                serde_json::json!({"frame":frame,"sessions":report["sessions"]})
            );
            return Err(error.context(format!("native probe frame {frame}")));
        }
    }
    let report = runtime.inspect();
    println!(
        "{}",
        serde_json::json!({"frame":report["frame"],"sessions":report["sessions"],"channel_audio":report["channel_audio"]})
    );
    Ok(())
}
