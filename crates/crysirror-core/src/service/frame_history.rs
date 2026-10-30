// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Client-scoped history of an engine mesh layer; no device handles cross here.
use super::*;

const PREFIX: &str = "__crysirror/frame-history/";
pub fn frame_history_path(layer: u32) -> String {
    alloc::format!("{PREFIX}{layer}")
}
pub fn frame_history_layer(path: &str) -> Option<u32> {
    let id = path.strip_prefix(PREFIX)?.parse::<u32>().ok()?;
    (frame_history_path(id) == path).then_some(id)
}
/// A presenter-owned capability for one client's mesh layers. Normal feedback
/// binds a logical texture path; no pixels/readback are needed on that path.
/// Readback is an explicit blocking fence, for native saves/screenshots on a
/// VM worker or an offline caller. It must not be used for per-frame feedback.
pub trait MeshFrameHistory: Send + Sync {
    /// Reserve one layer's history at its native logical size. Repeating the
    /// same reservation is idempotent; changing size resets its history.
    fn texture(&self, layer: u32, size: [u32; 2]) -> Result<String>;
    /// Last completed layer picture, resized to a thumbnail if requested.
    /// Fails visibly before the first picture or after presenter disposal.
    fn readback(&self, layer: u32, size: [u32; 2]) -> Result<Vec<u8>>;
}
/// A host installs its provider before boot. Absent providers retain the
/// engine CPU reference path. Capabilities and device work are never saved.
pub trait MeshFrameHistoryProvider: Send + Sync {
    fn client(self: Arc<Self>, client: ClientId) -> Result<Arc<dyn MeshFrameHistory>>;
    fn registered(&self, client: ClientId, layer: u32) -> bool;
}
