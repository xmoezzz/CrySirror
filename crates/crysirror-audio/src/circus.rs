//! Circus XPCM containers on the shared packet/seek decoder.
//! Reference: GARbro b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0,
//! ArcFormats/Circus/AudioPCM.cs (morkt, MIT; notice in circus.LICENSE).
//! PCM and Ogg payloads remain streamed; no native mixer or device is involved.
use crate::decoder::{AudioSource, MediaSource, Stream};
use anyhow::{ensure, Context, Result};
use std::io::{Read, Seek, SeekFrom};

fn u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// A checked container view exposing a small generated header and a bounded
/// region of a caller-owned seekable capability. Seeks cannot escape its body.
struct View {
    source: Box<dyn MediaSource>,
    prefix: Vec<u8>,
    body_start: u64,
    body_len: u64,
    at: u64,
}
impl Read for View {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let mut done = 0;
        if self.at < (self.prefix.len() as u64) {
            let n = out.len().min(self.prefix.len() - self.at as usize);
            out[..n].copy_from_slice(&self.prefix[self.at as usize..self.at as usize + n]);
            self.at += n as u64;
            done += n;
        }
        if done < out.len() {
            let relative = self.at - self.prefix.len() as u64;
            let n = (out.len() - done).min(self.body_len.saturating_sub(relative) as usize);
            if n > 0 {
                self.source
                    .seek(SeekFrom::Start(self.body_start + relative))?;
                let n = self.source.read(&mut out[done..done + n])?;
                if n == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "truncated XPCM payload",
                    ));
                }
                self.at += n as u64;
                done += n;
            }
        }
        Ok(done)
    }
}
impl Seek for View {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let length = self.prefix.len() as u64 + self.body_len;
        let next = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.at) + i128::from(n),
            SeekFrom::End(n) => i128::from(length) + i128::from(n),
        };
        if next < 0 || next > i128::from(length) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "XPCM seek outside payload",
            ));
        }
        self.at = next as u64;
        Ok(self.at)
    }
}
impl MediaSource for View {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.prefix.len() as u64 + self.body_len)
    }
}
/// Open XPCM mode 0 PCM or mode 5 Ogg on the engine's existing decoder.
/// Recognizable unsupported transform modes fail rather than becoming noise.
pub fn open_reader(mut source: Box<dyn MediaSource>) -> Result<Box<dyn AudioSource>> {
    ensure!(source.is_seekable(), "XPCM requires a seekable capability");
    source.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; 16];
    source.read_exact(&mut header)?;
    ensure!(&header[..4] == b"XPCM", "XPCM signature");
    let decoded = u64::from(u32(&header, 4));
    ensure!(decoded > 0, "XPCM decoded length");
    let mode = u32(&header, 8) & 255;
    let (start,len,prefix,extension):(u64,u64,Vec<u8>,&str)=match mode{
        5=>{
            let len=u64::from(u32(&header,12));ensure!(len>0,"XPCM Ogg length");
            let mut magic=[0u8;4];source.read_exact(&mut magic)?;ensure!(&magic==b"OggS","XPCM mode 5 payload is not Ogg");
            (16,len,Vec::new(),"ogg")
        }
        0=>{
            let mut format=[0u8;16];format[..4].copy_from_slice(&header[12..16]);source.read_exact(&mut format[4..])?;
            let tag=u16(&format,0);let channels=u16(&format,2);let rate=u32(&format,4);let byte_rate=u32(&format,8);let align=u16(&format,12);let bits=u16(&format,14);
            ensure!(tag==1&&matches!(channels,1|2)&&matches!(bits,8|16|24|32),"unsupported XPCM PCM layout");
            ensure!((8000..=192000).contains(&rate)&&align==channels*(bits/8)&&byte_rate==rate*u32::from(align),"XPCM PCM rates/alignment");
            ensure!(decoded%u64::from(align)==0&&decoded<=u64::from(u32::MAX)-36,"XPCM PCM length/alignment");
            let mut prefix=b"RIFF".to_vec();prefix.extend(((decoded+36) as u32).to_le_bytes());prefix.extend(b"WAVEfmt ");prefix.extend(16u32.to_le_bytes());prefix.extend(format);prefix.extend(b"data");prefix.extend((decoded as u32).to_le_bytes());
            (28,decoded,prefix,"wav")
        }
        other=>anyhow::bail!("unsupported XPCM transform mode {other}; LZSS/zlib spectral decoding is not implemented"),
    };
    let end = start.checked_add(len).context("XPCM payload overflow")?;
    if let Some(length) = source.byte_len() {
        ensure!(end <= length, "truncated XPCM payload length");
    }
    let view = View {
        source,
        prefix,
        body_start: start,
        body_len: len,
        at: 0,
    };
    Ok(Box::new(Stream::open(Box::new(view), Some(extension))?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn pcm() -> Vec<u8> {
        let mut b = b"XPCM".to_vec();
        b.extend(8u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(8000u32.to_le_bytes());
        b.extend(32000u32.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        for n in [i16::MIN, i16::MAX, 0, 16384] {
            b.extend(n.to_le_bytes());
        }
        b
    }
    #[test]
    fn streamed_pcm_has_original_rate_samples_exact_seek_and_eof() {
        let mut s = open_reader(Box::new(Cursor::new(pcm()))).unwrap();
        assert_eq!(s.sample_rate(), 8000);
        assert_eq!(s.channel_count(), Some(2));
        assert_eq!(s.frame_count(), Some(2));
        let mut out = [[0.; 2]; 8];
        assert_eq!(s.read(&mut out).unwrap(), 2);
        assert_eq!(out[0], [-1., 32767. / 32768.]);
        assert_eq!(out[1], [0., 0.5]);
        assert_eq!(s.read(&mut out).unwrap(), 0);
        s.seek(1).unwrap();
        assert_eq!(s.read(&mut out).unwrap(), 1);
        assert_eq!(out[0], [0., 0.5]);
        assert!(s.seek(3).is_err());
    }
    #[test]
    fn invalid_layouts_transform_modes_and_truncation_are_visible_errors() {
        let mut b = pcm();
        b[8] = 3;
        assert!(open_reader(Box::new(Cursor::new(b))).is_err());
        let mut b = pcm();
        b[12] = 2;
        assert!(open_reader(Box::new(Cursor::new(b))).is_err());
        let mut b = pcm();
        b.pop();
        assert!(open_reader(Box::new(Cursor::new(b))).is_err());
    }
    #[test]
    #[ignore = "requires owner-provided original Circus SE/MUSIC files"]
    fn installed_xpcm_resources_stream_through_engine_decoder() {
        let root = std::env::var_os("CRYSIRROR_CIRCUS_ADVDATA").expect("CRYSIRROR_CIRCUS_ADVDATA");
        let mut counts = [0usize; 2];
        for folder in ["SE", "MUSIC"] {
            for item in std::fs::read_dir(std::path::Path::new(&root).join(folder)).unwrap() {
                let p = item.unwrap().path();
                if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pcm")) {
                    continue;
                }
                let mut file = std::fs::File::open(&p).unwrap();
                let mut header = [0u8; 16];
                file.read_exact(&mut header).unwrap();
                if &header[..4] != b"XPCM" {
                    continue;
                }
                let mode = u32(&header, 8) & 255;
                assert!(matches!(mode, 0 | 5), "{}: mode {mode}", p.display());
                file.seek(SeekFrom::Start(0)).unwrap();
                let mut source = crate::decoder::open_reader(Box::new(file), Some("pcm"))
                    .unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
                let mut out = [[0.; 2]; 16];
                assert!(source.read(&mut out).unwrap() > 0);
                source.seek(0).unwrap();
                assert!(source.read(&mut out).unwrap() > 0);
                counts[usize::from(mode == 5)] += 1;
            }
        }
        assert!(counts.iter().all(|n| *n > 0));
        eprintln!(
            "streamed {} PCM and {} Ogg XPCM resources",
            counts[0], counts[1]
        );
    }
}
