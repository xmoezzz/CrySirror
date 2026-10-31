// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! A sampled host local civil clock, separate from the virtual engine clock.
use crate::{state::ClientId, Result};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilTime {
    pub client: ClientId,
    pub unix_ms: i64,
    pub utc_offset_seconds: i32,
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CivilTimeState {
    Pending,
    Ready(CivilTime),
    Failed(alloc::string::String),
}
/// An authorized worker dependency. Dropping it cancels unfinished sampling.
pub trait CivilTimeQuery: Send {
    fn client(&self) -> ClientId;
    fn poll(&mut self) -> Result<CivilTimeState>;
}
