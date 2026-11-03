//! Encoded multi-frame image operations over immutable engine-owned resources.
use crate::image_processing::ImageResource;
use alloc::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodedFrameFormat {
    BfMovie,
}
#[derive(Clone, Debug)]
pub struct EncodedFrameSpec {
    pub format: EncodedFrameFormat,
    pub bytes: Arc<[u8]>,
    pub frame: u32,
    /// Delta frames are decoded over this exact immutable previous image.
    pub destination: Arc<dyn ImageResource>,
}
