//! Zero-copy image view over a media decoder's immutable published frame.
use crysirror_core::{image_processing::ImageResource, service::MediaFrame, state::ClientId};
use std::sync::Arc;
#[derive(Clone)]
struct FrameImage {
    owner: ClientId,
    frame: MediaFrame,
}
impl std::fmt::Debug for FrameImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaFrameImage")
            .field("owner", &self.owner)
            .field("size", &self.frame.size())
            .finish()
    }
}
impl ImageResource for FrameImage {
    fn client(&self) -> ClientId {
        self.owner
    }
    fn size(&self) -> [u32; 2] {
        self.frame.size()
    }
    fn rgba(&self) -> Arc<[u8]> {
        self.frame.shared_rgba()
    }
    fn opaque(&self) -> bool {
        self.frame.rgba().chunks_exact(4).all(|p| p[3] == 255)
    }
    fn pixel_at(&self, point: [u32; 2]) -> crysirror_core::Result<Option<[u8; 4]>> {
        crysirror_core::image_processing::rgba_pixel_at(self.size(), self.frame.rgba(), point)
    }
}
pub(crate) fn view(owner: ClientId, frame: MediaFrame) -> Arc<dyn ImageResource> {
    Arc::new(FrameImage { owner, frame })
}
