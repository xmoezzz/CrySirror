//! RealLive extension DLLs (EF00, PT00, DT00) from siglus_rs `crates/reallive`
//! at 5f358ef1360b0439e92aed2b749e8f25d8721954. See ../SOURCE_NOTES.md.
//! Shared VFX owns spatial kernels; shared Resource owns PE data parsing.
#![forbid(unsafe_code)]
pub mod dt00;
mod dll;
pub mod pt00;
pub use dll::*;
