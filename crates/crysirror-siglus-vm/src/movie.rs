// SPDX-License-Identifier: MPL-2.0
//! Native global MOV declarations; decoding, playback clocks and PCM are engine services.
use anyhow::{Result, bail, ensure};
#[derive(Debug, Clone)]
pub(crate) enum MovieCommand {
    Play(String),
    Stop,
}
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct MovieManager {
    // Backend receipts/resources are not native save bytes. Adapter snapshots
    // explicitly reject live Movie resources until reconstruction is supported.
    #[serde(skip)]
    pub(crate) pending: Vec<MovieCommand>,
    #[serde(skip)]
    pub(crate) surface: Option<String>,
    #[serde(skip)]
    pub(crate) ready: bool,
    #[serde(skip)]
    pub(crate) live: bool,
}
impl MovieManager {
    pub fn stop(&mut self) {
        if self.live {
            self.pending.push(MovieCommand::Stop);
        }
        self.live = false;
        self.ready = false;
        self.surface = None;
    }
    pub(crate) fn play(&mut self, name: &str) -> Result<()> {
        ensure!(
            !name.is_empty() && name.len() <= 4096 && !name.chars().any(char::is_control),
            "native MOV file name"
        );
        ensure!(self.pending.len() < 64, "native MOV declaration limit");
        self.pending.push(MovieCommand::Play(name.into()));
        self.live = true;
        self.ready = false;
        self.surface = None;
        Ok(())
    }
    pub fn prepare(&mut self, _: &str) -> Result<()> {
        bail!(
            "Siglus original-save Movie preparation requires resource reconstruction; playback is not a prepare operation"
        )
    }
}
