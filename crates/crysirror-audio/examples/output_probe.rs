// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Explicit paused/silent device lifecycle probe. No VM, files or microphone.
#[cfg(feature = "cpal-output")]
fn main() -> anyhow::Result<()> {
    use crysirror_audio::pcm_output::{OutputPhase, PcmOutput};
    use std::time::{Duration, Instant};
    let mut output = PcmOutput::default();
    output.pause();
    // Nonzero admission exercises lazy opening; pause keeps it out of hardware.
    output.submit(&[[0.125, -0.125]; 128])?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = output.poll()?;
        if status.phase == OutputPhase::Ready && status.rendered_frames > 0 {
            break;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "device opening/callback probe timeout"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let ready = output.status();
    anyhow::ensure!(
        ready.paused && ready.queued_frames == 128 && ready.underrun_frames == 0,
        "paused source was consumed"
    );
    output.reset();
    // A normal-range f32 value far below audibility exercises actual dequeue.
    output.submit(&[[f32::MIN_POSITIVE, -f32::MIN_POSITIVE]; 128])?;
    output.resume();
    loop {
        let status = output.poll()?;
        if status.queued_frames == 0 && status.rendered_frames > ready.rendered_frames {
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "device PCM consumption timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
    let consumed = output.status();
    output.pause();
    let closed = output.begin_close();
    drop(output);
    while !closed.is_closed() {
        anyhow::ensure!(
            Instant::now() < deadline,
            "device destruction probe timeout"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let closed_status = closed.status();
    std::thread::sleep(Duration::from_millis(20));
    anyhow::ensure!(
        closed.status().rendered_frames == closed_status.rendered_frames,
        "callback continued after close completion"
    );
    let report = serde_json::json!({"probe":"paused_silent_output","ready":ready,"consumed":consumed,"closed":closed_status});
    let text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, text.as_bytes())?;
    }
    println!("{text}");
    Ok(())
}
#[cfg(not(feature = "cpal-output"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("output_probe requires --features cpal-output")
}
