// SPDX-License-Identifier: MPL-2.0
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct MovieManager;
impl MovieManager {
    pub fn stop(&mut self) {}
    pub fn prepare(&mut self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("Siglus native movie backend unavailable")
    }
}
