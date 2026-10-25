// SPDX-License-Identifier: MPL-2.0
//! Developer probe: per-frame timestamps and mean color of a video file.
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("probe <video>");
    let mut s = crysirror_video::open(std::fs::read(&path)?)?;
    println!("{:?}", s.info());
    while let Some(f) = s.next_frame()? {
        let n = (f.width * f.height) as u64;
        let m: Vec<u64> = (0..3).map(|c| f.rgba.chunks_exact(4).map(|p| p[c] as u64).sum::<u64>() / n).collect();
        println!("{:6} ms {}x{} mean {:?}", f.pts_ms, f.width, f.height, m);
    }
    Ok(())
}
