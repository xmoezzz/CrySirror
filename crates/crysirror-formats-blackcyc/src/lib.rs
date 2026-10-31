// SPDX-License-Identifier: MIT
// Black Cyc format facts/algorithms derived from GARbro, morkt 2015-2017.
// See LICENSE and SOURCE.json. No original engine is linked.
#![forbid(unsafe_code)]
mod archive;
mod dwq;
mod vaw;
mod gpk_index;
pub use gpk_index::{GpkIndex,GpkIndexEntry};
pub use archive::{Gpk,Vpk};
pub use dwq::{Dwq,DwqInfo};
pub use vaw::Vaw;
use anyhow::{Context,Result,ensure};
use std::sync::Arc;
pub fn register_archives(registry:&mut crysirror_resource::FormatRegistry)->Result<()> {
    registry.register(Arc::new(Gpk))?;registry.register(Arc::new(Vpk))
}
pub fn register_images(registry:&mut crysirror_image::DecoderRegistry)->Result<()> {registry.register_format(Arc::new(Dwq))}
pub fn register_audio(registry:&mut crysirror_resource::FileRegistry)->Result<()> {registry.register(Arc::new(Vaw))}
fn u16at(d:&[u8],at:usize)->Result<u16>{Ok(u16::from_le_bytes(d.get(at..at+2).context("BlackCyc short u16")?.try_into()?))}
fn u32at(d:&[u8],at:usize)->Result<u32>{Ok(u32::from_le_bytes(d.get(at..at+4).context("BlackCyc short u32")?.try_into()?))}
#[derive(Debug,Clone,Copy)]
struct Header {pack:u16,alpha:bool}
fn header(d:&[u8])->Result<Header> {
    let h=d.get(48..64).context("BlackCyc resource header")?;
    let rest=h.strip_prefix(b"PACKTYPE=").context("BlackCyc PACKTYPE absent")?;
    let n=rest.iter().take_while(|b|b.is_ascii_digit()).count();
    ensure!(n>0,"BlackCyc PACKTYPE digits absent");
    let pack=std::str::from_utf8(&rest[..n])?.parse::<u16>()?;
    let alpha=rest.get(n)==Some(&b'A');let pad=&rest[n+usize::from(alpha)..];
    ensure!(!pad.is_empty()&&pad.iter().all(|b|*b==b' '),"BlackCyc PACKTYPE padding/profile");
    Ok(Header{pack,alpha})
}
