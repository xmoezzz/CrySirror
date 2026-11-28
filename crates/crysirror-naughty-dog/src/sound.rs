// SPDX-License-Identifier: MPL-2.0
//! Sound effects: `audio.bin` sound records and the Scream banks of
//! `sound1/banks.psarc`.
//!
//! `audio.bin` entries of type 0xEE0BFF2F are `{id, Scream name, flags,
//! class, …, half-float parameters}`: the script id selects the record
//! and its name the bank sound (`sub_138650`).
//!
//! A bank is `{version 3, section count, (offset, size)…}`; section 0 is
//! the `SBlk` sound block (magic stored as `klBS`), section 1 the
//! PS-ADPCM sample data. `SBlk` +0x14 sound count, +0x18 grain count
//! (high half), +0x1C sounds, +0x20 grains, +0x34 grain data, +0x38 the
//! name table, all relative to the block. A sound (12 bytes) is `{volume,
//! …, grain count (+4), …, first grain byte offset (+8)}`; a grain (8
//! bytes) is `{command << 24 | data offset, delay}`. Command 1 plays a
//! tone: 24 bytes of grain data ending in the sample's offset and size in
//! section 1. The name table is `{bank name[8], buckets, strings offset}`
//! and one 16-byte record per sound `{string offset, hash, bucket head:u16, next:u16, sound index:u16, pad:u16}`.
use crate::dc::{Dc, sid};
use anyhow::{Context, Result, ensure};
use crysirror_core::service::AssetReader;
use std::collections::HashMap;
use std::sync::Arc;

const TYPE_SOUND: u32 = 0xee0b_ff2f;
const MAX_SECTION: usize = 4 << 20;
const MAX_SAMPLE: usize = 16 << 20;
/// Grain command playing a tone.
const GRAIN_TONE: u32 = 1;
const GRAIN_SOUND: u32 = 5;
/// Sound references followed before giving up.
const MAX_REF_DEPTH: usize = 4;
/// Codec word of MP3 tones.
const TONE_MP3: u32 = 3;
/// The rate bank samples play at. The tone's centre-note bytes are not
/// decoded (SATD-nd-retail-runtime), so samples play at the PS3 mixer
/// rate.
pub const SAMPLE_RATE: u32 = 48_000;

/// `audio.bin`: script sound id → Scream sound name.
#[derive(Clone, Debug, Default)]
pub struct SoundTable {
    pub names: HashMap<u32, String>,
}

impl SoundTable {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut dc = Dc::default();
        dc.load(sid("audio"), bytes)?;
        let mut names = HashMap::new();
        for &(id, at) in &dc.modules[0].entries {
            if dc.mem.type_of(at).ok() != Some(TYPE_SOUND) {
                continue;
            }
            let name = dc.mem.u32(at + 4)?;
            if name != 0 {
                names.insert(id, dc.mem.cstr(name)?);
            }
        }
        Ok(Self { names })
    }
}

/// A tone of a bank sound: the sample's bytes in section 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tone {
    pub offset: u32,
    pub size: u32,
}

/// A sound a grain plays (command 5): `{volume, 0, flags, bank, …}`;
/// bank −1 names a sound of any bank (`+0x14`), otherwise byte 8 is a
/// sound index of the same bank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundRef {
    Index(u16),
    Name(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BankSound {
    /// Volume 0–127 (sound byte 0).
    pub volume: u8,
    pub tones: Vec<Tone>,
    pub refs: Vec<SoundRef>,
}

/// One bank's sounds by name and where its sample data lies.
#[derive(Clone)]
pub struct Bank {
    pub name: String,
    pub sounds: HashMap<String, BankSound>,
    by_index: Vec<String>,
    samples: (u64, u64),
    reader: Arc<dyn AssetReader>,
}

fn be(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(b.get(o..o + 4).context("Scream bank truncated")?.try_into()?))
}

impl Bank {
    /// Read a bank's header and sound block (the samples stay on disc).
    pub fn read(name: &str, reader: Arc<dyn AssetReader>) -> Result<Self> {
        let mut head = [0u8; 32];
        reader.read_at(0, &mut head)?;
        ensure!(be(&head, 0)? == 3 && be(&head, 4)? >= 2, "Scream bank header");
        let (o0, s0, o1, s1) = (be(&head, 8)? as usize, be(&head, 12)? as usize, be(&head, 16)?, be(&head, 20)?);
        ensure!(s0 <= MAX_SECTION && s0 >= 0x40, "SBlk size");
        let mut b = vec![0u8; s0];
        reader.read_at(o0 as u64, &mut b)?;
        ensure!(&b[..4] == b"klBS", "not an SBlk block");
        let count = be(&b, 0x14)? as usize;
        let grains = (be(&b, 0x18)? >> 16) as usize;
        let (sounds, grain_base, data, table) = (be(&b, 0x1c)? as usize, be(&b, 0x20)? as usize, be(&b, 0x34)? as usize, be(&b, 0x38)? as usize);
        ensure!(count <= 65536 && grains <= 1 << 20, "SBlk counts");
        let strings = table + be(&b, table + 12)? as usize;
        let mut by_index = vec![String::new(); count];
        let mut out = HashMap::with_capacity(count);
        for i in 0..count {
            let r = table + 0x10 + 16 * i;
            let at = strings + be(&b, r)? as usize;
            let text = b.get(at..).context("SBlk name")?;
            let end = text.iter().position(|c| *c == 0).unwrap_or(text.len());
            let sound_name = String::from_utf8_lossy(&text[..end]).into_owned();
            let index = u16::from_be_bytes(b.get(r + 12..r + 14).context("SBlk name record")?.try_into()?) as usize;
            ensure!(index < count && by_index[index].is_empty(), "invalid or repeated SBlk sound index");
            let s = sounds + 12 * index;
            let volume = *b.get(s).context("SBlk sound")?;
            let n = *b.get(s + 4).context("SBlk sound")? as usize;
            let first = grain_base + be(&b, s + 8)? as usize;
            let (mut tones, mut refs) = (Vec::new(), Vec::new());
            for g in 0..n {
                let w = be(&b, first + 8 * g)?;
                let t = data + (w & 0x00ff_ffff) as usize;
                match w >> 24 {
                    GRAIN_TONE => {
                        let (offset, size) = (be(&b, t + 16)?, be(&b, t + 20)?);
                        ensure!(u64::from(offset) + u64::from(size) <= u64::from(s1), "tone outside the sample data");
                        tones.push(Tone { offset, size });
                    }
                    GRAIN_SOUND if be(&b, t + 12)? == u32::MAX => {
                        let text = b.get(t + 20..t + 52).context("sound reference")?;
                        let end = text.iter().position(|c| *c == 0).unwrap_or(text.len());
                        refs.push(SoundRef::Name(String::from_utf8_lossy(&text[..end]).into_owned()));
                    }
                    GRAIN_SOUND if be(&b, t + 12)? == 0 => refs.push(SoundRef::Index(u16::from(*b.get(t + 8).context("sound reference")?))),
                    _ => {}
                }
            }
            by_index[index] = sound_name.clone();
            out.insert(sound_name, BankSound { volume, tones, refs });
        }
        Ok(Self { name: name.to_owned(), sounds: out, by_index, samples: (u64::from(o1), u64::from(s1)), reader })
    }

    /// A tone as an encoded stream the host decodes, and its loop start
    /// frame. Tones are raw PS-ADPCM (VAG loop flags: 0x04 start, 0x02
    /// repeat), converted to WAVE, or MP3 behind a 0x80-byte header
    /// `{1, codec 3, …, +0x0C rate, …, +0x34 data size}`, passed through.
    pub fn wave(&self, tone: Tone) -> Result<(Vec<u8>, Option<u64>)> {
        let size = tone.size as usize;
        ensure!(size <= MAX_SAMPLE, "tone size");
        ensure!(u64::from(tone.offset) + tone.size as u64 <= self.samples.1, "tone range");
        let mut adpcm = vec![0u8; size];
        self.reader.read_at(self.samples.0 + u64::from(tone.offset), &mut adpcm)?;
        if size >= 0x80 && be(&adpcm, 0)? == 1 && be(&adpcm, 4)? == TONE_MP3 {
            let len = be(&adpcm, 0x34)? as usize;
            let data = adpcm.get(0x80..0x80 + len).context("MP3 tone size")?;
            ensure!(data.len() >= 2 && data[0] == 0xff && data[1] & 0xe0 == 0xe0, "MP3 tone sync");
            return Ok((data.to_vec(), None));
        }
        ensure!(size % 16 == 0, "PS-ADPCM tone size");
        let mut pcm = Vec::with_capacity(size / 16 * 28);
        let mut hist = (0, 0);
        let (mut start, mut repeats) = (None, false);
        for (k, frame) in adpcm.chunks_exact(16).enumerate() {
            let flags = frame[1];
            if flags & 0x04 != 0 && start.is_none() {
                start = Some(k as u64 * 28);
            }
            crysirror_audio::xvag::ps_adpcm_frame(frame.try_into()?, &mut hist, &mut pcm)?;
            if flags & 0x01 != 0 {
                repeats = flags & 0x02 != 0;
                break;
            }
        }
        ensure!(!pcm.is_empty(), "empty tone");
        Ok((wave(&pcm, SAMPLE_RATE), if repeats { Some(start.unwrap_or(0)) } else { None }))
    }
}

/// Mono PCM16 as a WAVE stream.
fn wave(pcm: &[i16], rate: u32) -> Vec<u8> {
    let bytes = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + bytes).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&bytes.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

/// Every bank's sounds by name.
#[derive(Clone, Default)]
pub struct Banks {
    pub banks: Vec<Bank>,
    index: HashMap<String, usize>,
}

impl Banks {
    pub fn add(&mut self, bank: Bank) {
        let k = self.banks.len();
        for name in bank.sounds.keys() {
            self.index.entry(name.clone()).or_insert(k);
        }
        self.banks.push(bank);
    }
    pub fn sound(&self, name: &str) -> Option<(&Bank, &BankSound)> {
        let bank = &self.banks[*self.index.get(name)?];
        Some((bank, bank.sounds.get(name)?))
    }

    /// The sound with tones that `name` plays: itself, or the first sound
    /// its references lead to.
    pub fn playable(&self, name: &str) -> Option<(&Bank, &BankSound)> {
        let mut at = self.sound(name)?;
        for _ in 0..MAX_REF_DEPTH {
            if !at.1.tones.is_empty() {
                return Some(at);
            }
            at = at.1.refs.iter().find_map(|r| match r {
                SoundRef::Name(n) => self.sound(n),
                SoundRef::Index(i) => at.0.by_index.get(usize::from(*i)).and_then(|n| at.0.sounds.get(n)).map(|s| (at.0, s)),
            })?;
        }
        (!at.1.tones.is_empty()).then_some(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::MemoryAssetReader;

    /// A bank with one sound `boom` of one tone (two ADPCM frames, the
    /// second ending the sample).
    fn bank() -> Vec<u8> {
        let mut s = vec![0u8; 0x100];
        let w = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_be_bytes());
        s[..4].copy_from_slice(b"klBS");
        w(&mut s, 0x14, 1);
        w(&mut s, 0x18, 1 << 16);
        w(&mut s, 0x1c, 0x40);
        w(&mut s, 0x20, 0x4c);
        w(&mut s, 0x34, 0x54);
        w(&mut s, 0x38, 0x80);
        s[0x40] = 100;
        s[0x44] = 1;
        w(&mut s, 0x48, 0);
        w(&mut s, 0x4c, (1 << 24) | 0);
        w(&mut s, 0x54 + 16, 0);
        w(&mut s, 0x54 + 20, 32);
        w(&mut s, 0x80 + 12, 0x20);
        w(&mut s, 0x90, 0);
        // Empty hash bucket at +8; this is not the sound number at +12.
        w(&mut s, 0x98, u32::MAX);
        s[0xa0..0xa4].copy_from_slice(b"boom");
        let mut b = vec![0u8; 32];
        w(&mut b, 0, 3);
        w(&mut b, 4, 2);
        w(&mut b, 8, 32);
        w(&mut b, 12, s.len() as u32);
        w(&mut b, 16, 32 + s.len() as u32);
        w(&mut b, 20, 32);
        b.extend(s);
        let mut frames = [0u8; 32];
        frames[17] = 0x01;
        b.extend(frames);
        b
    }

    #[test]
    fn bank_sound_tones_and_wave() {
        let bank = Bank::read("test", Arc::new(MemoryAssetReader(bank().into()))).unwrap();
        let s = &bank.sounds["boom"];
        assert_eq!(s.volume, 100);
        assert_eq!(s.tones, vec![Tone { offset: 0, size: 32 }]);
        let (w, looped) = bank.wave(s.tones[0]).unwrap();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 2 * 56);
        assert_eq!(looped, None);
    }
}
