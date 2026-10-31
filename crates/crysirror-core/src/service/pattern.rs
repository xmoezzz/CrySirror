// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Host-owned persistent raster patterns. Specification/RNG/snapshot bytes are
//! bounded portable data; resources are scoped, immutable and never serialized.
use crate::{image_processing::{ImageInput,ImageResource},state::ClientId,Result};
use alloc::{string::String,sync::Arc};
use core::any::Any;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct PatternResourceInfo {
    pub client: ClientId,
    pub size: [u32;2],
    pub completed_steps: u64,
    pub resident_bytes: usize,
}
pub trait PatternResource: Any+Send+Sync+core::fmt::Debug {
    fn client(&self)->ClientId;
    fn info(&self)->PatternResourceInfo;
    /// Immutable portable state, encoded on the worker before publication.
    /// Retain the resource while borrowing its snapshot, as for ImageResource.
    fn snapshot(&self)->Arc<[u8]>;
    fn as_any(&self)->&dyn Any;
}
#[derive(Clone,Debug)]
pub enum PatternRequest {
    Create {spec:Arc<[u8]>,rng:Arc<[u8]>},
    Advance {source:Arc<dyn PatternResource>,rng:Arc<[u8]>},
    /// Same immutable resource/RNG transaction, limited to a prefix bank.
    AdvancePrefix {source:Arc<dyn PatternResource>,rng:Arc<[u8]>,words:usize},
    Render {source:Arc<dyn PatternResource>,destination:ImageInput,rect:[u32;4],opacity256:u16},
    Restore {snapshot:Arc<[u8]>,rng:Arc<[u8]>},
}
#[derive(Clone,Debug)]
pub enum PatternOutput {
    Resource {resource:Arc<dyn PatternResource>,rng_after:Arc<[u8]>},
    Image(Arc<dyn ImageResource>),
}
#[derive(Clone,Debug)]
pub enum PatternProcessState {
    Pending,
    Ready(PatternOutput),
    Failed(String),
}
/// Poll never joins. Completion is retained; dropping unfinished jobs cancels.
pub trait PatternProcess:Send {
    fn client(&self)->ClientId;
    fn poll(&mut self)->Result<PatternProcessState>;
}
pub type PatternJob=alloc::boxed::Box<dyn PatternProcess>;
