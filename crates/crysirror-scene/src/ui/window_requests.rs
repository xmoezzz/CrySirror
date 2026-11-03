//! One-shot platform-neutral window requests. The desktop owner consumes them;
//! no OS window handle is retained in the engine or a VM snapshot.
use anyhow::{Result, ensure};
#[derive(Clone, Debug, Default)]
pub struct WindowRequests {
    pub size: Option<(u32, [u32; 2])>,
    pub position: Option<(u32, [i32; 2])>,
    pub visible: Option<(u32, bool)>,
    pub minimized: Option<(u32, bool)>,
}
impl WindowRequests {
    pub fn set_size(&mut self, client: u32, size: [u32; 2]) -> Result<()> {
        ensure!(
            size.iter().all(|n| (1..=32768).contains(n)),
            "window client size range"
        );
        self.size = Some((client, size));
        Ok(())
    }
    pub fn set_position(&mut self, client: u32, position: [i32; 2]) -> Result<()> {
        ensure!(
            position.iter().all(|n| n.unsigned_abs() <= 1_000_000),
            "window position range"
        );
        self.position = Some((client, position));
        Ok(())
    }
}
