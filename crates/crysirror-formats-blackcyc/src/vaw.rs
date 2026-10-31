// SPDX-License-Identifier: MIT
// Wrapper/delta format derived from GARbro AudioVAW.cs, morkt 2015-2016.
// Outputs immutable standard WAVE/Ogg; playback/seek/mixing stay shared Audio.
use crate::{header,u32at};
use anyhow::{Context,Result,ensure,bail};
use crysirror_core::service::AssetReader;
use crysirror_resource::{DecodeContext,DecodedFile,FileFormat,FileKind,Probe,read_exact_at,read_all,subrange};
use std::sync::Arc;
pub struct Vaw;
struct Bits<'a>{bytes:&'a[u8],bit:usize}
impl Bits<'_>{fn get(&mut self,n:usize)->Result<u32>{
    ensure!(n<=15,"VAW bit width");let end=self.bit.checked_add(n).context("VAW bit address overflow")?;
    ensure!(end<=self.bytes.len()*8,"VAW truncated delta bitstream");let mut value=0;
    for i in 0..n {value|=u32::from((self.bytes[(self.bit+i)/8]>>((self.bit+i)%8))&1)<<i;}
    self.bit=end;Ok(value)
}}
fn unpack(d:&[u8],cx:&DecodeContext)->Result<Arc<dyn AssetReader>> {
    let b=d.get(64..).context("VAW compressed body")?;
    ensure!(b.get(..4)==Some(b"RIFF")&&b.get(8..16)==Some(b"WAVEfmt "),"VAW compressed RIFF/fmt profile");
    let fmt=u32at(b,16)? as usize;ensure!(fmt<=65536,"VAW format block budget");
    let prefix=20usize.checked_add(fmt).context("VAW format size overflow")?;
    let size=(u32at(b,4)? as usize).checked_add(8).context("VAW output size overflow")?;
    ensure!(size>=prefix&&(size-prefix)%2==0,"VAW sample/output span");
    let source=b.get(prefix..).context("VAW compressed samples absent")?;
    cx.decode_with(size as u64,|| {
        let mut output=Vec::with_capacity(size);output.extend_from_slice(&b[..prefix]);
        let mut bits=Bits{bytes:source,bit:0};let mut sample=0i16;
        while output.len()<size {
            if (output.len()-prefix)%8192==0 {cx.check_cancelled()?;}
            let width=bits.get(4)? as usize;
            let code=if width==0{0}else{
                let raw=bits.get(width)? as i32;
                let signed=(raw<<(32-width))>>(32-width);
                let sign=signed>>31;(signed^(0x4000>>(15-width)))-sign
            };
            sample=sample.wrapping_add(code as i16);output.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(output)
    })
}
impl FileFormat for Vaw {
    fn id(&self)->&'static str{"black-cyc/vaw"}
    fn kind(&self)->FileKind{FileKind::Audio}
    fn probe(&self,path:&str,source:&dyn AssetReader)->Result<Probe> {
        let size=source.len().map_err(|e|anyhow::anyhow!("{e}"))?;if size<64{return Ok(Probe::NO);}
        let mut bytes=[0;64];read_exact_at(source,0,&mut bytes)?;
        let Ok(h)=header(&bytes) else{return Ok(Probe::NO)};
        let native_extension=path.rsplit('.').next().is_some_and(|s|s.eq_ignore_ascii_case("vaw")||s.eq_ignore_ascii_case("wgq"));
        if !matches!(h.pack,0|1|2|6){return Ok(if native_extension{Probe{confidence:90}}else{Probe::NO});}
        let at=if h.pack==2{108}else{64};
        if size<at+4{return Ok(Probe::NO);}
        let mut mark=[0;4];read_exact_at(source,at,&mut mark)?;
        Ok(if if h.pack<=1{mark==*b"RIFF"}else{mark==*b"OggS"}{Probe{confidence:90}}else{Probe::NO})
    }
    fn decode(&self,_:&str,source:Arc<dyn AssetReader>,cx:&DecodeContext)->Result<DecodedFile> {
        cx.check_cancelled()?;let mut bytes=[0;64];read_exact_at(source.as_ref(),0,&mut bytes)?;
        let h=header(&bytes)?;
        let (reader,extension)=match h.pack {
            1=>{let d=read_all(source.as_ref(),cx.limits.max_entry_size)?;(unpack(&d,cx)?,"wav")},
            0|2|6=>{
                let at=if h.pack==2{108}else{64};
                if h.pack==6 {ensure!(bytes.get(16..20)==Some(b"OGG "),"unsupported VAW PACKTYPE6 payload");}
                let size=source.len().map_err(|e|anyhow::anyhow!("{e}"))?;
                ensure!(size>=at&&size-at<=cx.limits.max_entry_size,"VAW payload bounds");
                let mut mark=[0;12];read_exact_at(source.as_ref(),at,&mut mark)?;
                let ext=if h.pack==0 {ensure!(mark[..4]==*b"RIFF"&&mark[8..12]==*b"WAVE","VAW WAVE payload mark");"wav"}
                    else {ensure!(mark[..4]==*b"OggS","VAW Ogg payload mark");"ogg"};
                (subrange(source,at,size-at)?,ext)
            }
            _=>bail!("unsupported VAW PACKTYPE {}",h.pack),
        };
        Ok(DecodedFile{reader,extension:extension.into(),metadata:serde_json::json!({"format":"black-cyc/vaw","pack_type":h.pack})})
    }
}
