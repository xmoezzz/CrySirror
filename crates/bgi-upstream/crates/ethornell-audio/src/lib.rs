//! Native audio container metadata only. CrySirror owns decoding, mixing and output.
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub struct AudioInfo {
    pub kind: AudioKind,
    pub header_len: Option<u32>,
    pub file_size: Option<u32>,
    pub sample_len: Option<u32>,
    pub frequency: Option<u32>,
    pub channels: Option<u32>,
    pub payload_offset: Option<usize>,
    pub payload_kind: Option<AudioKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AudioKind {
    Ogg,
    RiffWave,
    BurikoWaveBoxOgg,
    BurikoWaveBoxUnknown,
    Unknown,
}

pub fn probe_audio(data: &[u8]) -> AudioInfo {
    if data.starts_with(b"OggS") {
        return simple_info(AudioKind::Ogg);
    }
    if data.starts_with(b"RIFF") {
        return simple_info(AudioKind::RiffWave);
    }
    if data.len() >= 0x40 && &data[4..8] == b"bw  " {
        let header_len = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let payload_offset = header_len as usize;
        let payload_kind = data
            .get(payload_offset..payload_offset + 4)
            .map(|magic| {
                if magic == b"OggS" {
                    AudioKind::Ogg
                } else if magic == b"RIFF" {
                    AudioKind::RiffWave
                } else {
                    AudioKind::Unknown
                }
            })
            .unwrap_or(AudioKind::Unknown);
        return AudioInfo {
            kind: if payload_kind == AudioKind::Ogg {
                AudioKind::BurikoWaveBoxOgg
            } else {
                AudioKind::BurikoWaveBoxUnknown
            },
            header_len: Some(header_len),
            file_size: read_u32(data, 8),
            sample_len: read_u32(data, 12),
            frequency: read_u32(data, 16),
            channels: read_u32(data, 20),
            payload_offset: Some(payload_offset),
            payload_kind: Some(payload_kind),
        };
    }
    AudioInfo {
        kind: AudioKind::Unknown,
        header_len: None,
        file_size: None,
        sample_len: None,
        frequency: None,
        channels: None,
        payload_offset: None,
        payload_kind: None,
    }
}

pub fn unwrap_buriko_wave_ogg(data: &[u8]) -> Option<&[u8]> {
    let info = probe_audio(data);
    if info.kind != AudioKind::BurikoWaveBoxOgg {
        return None;
    }
    let offset = info.payload_offset?;
    data.get(offset..)
}

fn simple_info(kind: AudioKind) -> AudioInfo {
    AudioInfo {
        kind,
        header_len: None,
        file_size: None,
        sample_len: None,
        frequency: None,
        channels: None,
        payload_offset: Some(0),
        payload_kind: Some(kind),
    }
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}



pub fn declared_frame_count(bytes: &[u8]) -> Option<usize> {
    let info = probe_audio(bytes);
    let frames = match info.kind {
        AudioKind::BurikoWaveBoxOgg => u64::from(info.sample_len?),
        AudioKind::Ogg => ogg_end_position(bytes)?,
        _ => return None,
    };
    usize::try_from(frames).ok()
}


fn ogg_end_position(data: &[u8]) -> Option<u64> {
    const MAX_PAGE_LEN: usize = 27 + 255 + 255 * 255;
    let serial = data.get(14..18)?;
    let first = data.len().saturating_sub(MAX_PAGE_LEN);
    let page = (first..data.len()).rev().find_map(|start| {
        let page = &data[start..];
        let segments = usize::from(*page.get(26)?);
        let body: usize = page
            .get(27..27 + segments)?
            .iter()
            .map(|&len| usize::from(len))
            .sum();
        (page.starts_with(b"OggS") && page[4] == 0 && 27 + segments + body == page.len())
            .then_some(page)
    })?;
    if &page[14..18] != serial {
        return None;
    }
    let granule = u64::from_le_bytes(page[6..14].try_into().ok()?);
    (granule != u64::MAX).then_some(granule)
}


pub fn buriko_wave_loop_start(bytes: &[u8]) -> u64 {
    if probe_audio(bytes).kind == AudioKind::BurikoWaveBoxOgg {
        read_u32(bytes, 0x1C).map_or(0, u64::from)
    } else {
        0
    }
}
