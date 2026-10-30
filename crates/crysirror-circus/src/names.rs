//! Native NAMES.DAT version 7 resource indices (sub_40C7B0). These are native
//! semantic names/IDs; decoding, archive extraction and playback are engine APIs.
use anyhow::{ensure, Context, Result};
#[derive(Debug)]
pub struct Names {
    pub bootstrap: Vec<String>,
    pub music: Vec<String>,
    pub sfx: Vec<String>,
    pub images: Vec<String>,
    pub scenes: Vec<String>,
}
fn string(raw: &[u8]) -> Result<String> {
    let n = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    let (s, errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(&raw[..n]);
    ensure!(!errors, "Circus NAMES Shift-JIS");
    Ok(s.into_owned())
}
fn count(bytes: &[u8], at: &mut usize, max: usize) -> Result<usize> {
    let n = u32::from_le_bytes(
        bytes
            .get(*at..*at + 4)
            .context("Circus NAMES count")?
            .try_into()?,
    ) as usize;
    *at += 4;
    ensure!(n <= max, "Circus NAMES count limit");
    Ok(n)
}
fn table(bytes: &[u8], at: &mut usize, width: usize, max: usize) -> Result<Vec<String>> {
    let n = count(bytes, at, max)?;
    let end = at
        .checked_add(n.checked_mul(width).context("Circus NAMES overflow")?)
        .context("Circus NAMES overflow")?;
    let raw = bytes
        .get(*at..end)
        .context("truncated Circus NAMES table")?;
    *at = end;
    raw.chunks_exact(width).map(|r| string(&r[..32])).collect()
}
impl Names {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= 256 * 1024, "Circus NAMES byte limit");
        let mut at = 0;
        ensure!(count(bytes, &mut at, 7)? == 7, "Circus NAMES version");
        let bootstrap = bytes
            .get(at..at + 224)
            .context("Circus NAMES bootstrap")?
            .chunks_exact(32)
            .map(string)
            .collect::<Result<Vec<_>>>()?;
        at += 224;
        let music = table(bytes, &mut at, 48, 400)?;
        let sfx = table(bytes, &mut at, 36, 600)?;
        let images = table(bytes, &mut at, 32, 1200)?;
        let scenes = table(bytes, &mut at, 32, 650)?;
        ensure!(at == bytes.len(), "Circus NAMES trailing data");
        Ok(Self {
            bootstrap,
            music,
            sfx,
            images,
            scenes,
        })
    }
}
