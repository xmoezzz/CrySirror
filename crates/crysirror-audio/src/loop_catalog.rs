// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Bounded explicit loop metadata, independent of decoders, devices and VMs.
//! Publication changes future explicit lookups only. Playback never implicitly
//! consults this catalog. The owner publishes a fully validated batch atomically.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioLoopCatalogReceipt, AudioLoopMetadata, AudioLoopTimebase},
    state::ClientId,
    Error, Result,
};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_LOOP_ENTRIES_PER_CLIENT: usize = 4096;
pub const MAX_LOOP_CATALOG_ENTRIES: usize = 16384;
pub const MAX_LOOP_CATALOG_PATH_BYTES: usize = 2 * 1024 * 1024;
const MAX_CLIENTS: usize = 64;
#[derive(Default)]
struct Catalog {
    revision: u64,
    entries: BTreeMap<String, AudioLoopMetadata>,
    bytes: usize,
}
#[derive(Default)]
pub struct AudioLoopCatalog {
    clients: BTreeMap<ClientId, Catalog>,
    entries: usize,
    bytes: usize,
}
impl AudioLoopCatalog {
    pub fn publish(
        &mut self,
        client: ClientId,
        entries: &[AudioLoopMetadata],
    ) -> Result<AudioLoopCatalogReceipt> {
        if entries.len() > MAX_LOOP_ENTRIES_PER_CLIENT {
            return Err(Error::Limit("audio loop batch"));
        }
        let prior = self.clients.get(&client);
        if prior.is_none() && !entries.is_empty() && self.clients.len() >= MAX_CLIENTS {
            return Err(Error::Limit("audio loop clients"));
        }
        let mut unique = BTreeSet::new();
        let mut extra_entries = 0;
        let mut extra_bytes = 0;
        let mut changed = false;
        for entry in entries {
            if entry.asset.len() > 4096 {
                return Err(Error::Limit("audio loop asset path"));
            }
            if VfsPath::new(Mount::Asset, &entry.asset)?.relative() != entry.asset {
                return Err(Error::Invalid("canonical audio loop path"));
            }
            if !unique.insert(entry.asset.as_str()) {
                return Err(Error::Invalid("duplicate audio loop asset"));
            }
            if let AudioLoopTimebase::ReferenceFrames { sample_rate } = entry.range.timebase {
                if !(8000..=384000).contains(&sample_rate) {
                    return Err(Error::Invalid("audio loop reference rate"));
                }
            }
            if entry
                .range
                .end_frame
                .is_some_and(|end| end <= entry.range.start_frame)
            {
                return Err(Error::Invalid("audio loop extent"));
            }
            let old = prior.and_then(|p| p.entries.get(&entry.asset));
            if old != Some(entry) {
                changed = true;
            }
            if old.is_none() {
                extra_entries += 1;
                extra_bytes += entry.asset.len();
            }
        }
        let count = prior.map_or(0, |p| p.entries.len()) + extra_entries;
        if count > MAX_LOOP_ENTRIES_PER_CLIENT
            || self.entries + extra_entries > MAX_LOOP_CATALOG_ENTRIES
            || self.bytes + extra_bytes > MAX_LOOP_CATALOG_PATH_BYTES
        {
            return Err(Error::Limit("audio loop catalog"));
        }
        let revision = prior.map_or(0, |p| p.revision);
        let revision = if changed {
            revision
                .checked_add(1)
                .ok_or(Error::Limit("audio loop revision"))?
        } else {
            revision
        };
        if changed {
            let catalog = self.clients.entry(client).or_default();
            for entry in entries {
                catalog.entries.insert(entry.asset.clone(), entry.clone());
            }
            catalog.revision = revision;
            catalog.bytes += extra_bytes;
            self.entries += extra_entries;
            self.bytes += extra_bytes;
        }
        Ok(AudioLoopCatalogReceipt {
            client,
            revision,
            entries: count as u32,
        })
    }
    pub fn get(&self, client: ClientId, asset: &str) -> Result<Option<AudioLoopMetadata>> {
        if asset.len() > 4096 {
            return Err(Error::Limit("audio loop asset path"));
        }
        if VfsPath::new(Mount::Asset, asset)?.relative() != asset {
            return Err(Error::Invalid("canonical audio loop path"));
        }
        Ok(self
            .clients
            .get(&client)
            .and_then(|c| c.entries.get(asset))
            .cloned())
    }
    pub fn clear(&mut self, client: ClientId) -> Result<AudioLoopCatalogReceipt> {
        let Some(c) = self.clients.get_mut(&client) else {
            return Ok(AudioLoopCatalogReceipt {
                client,
                revision: 0,
                entries: 0,
            });
        };
        if !c.entries.is_empty() {
            let revision = c
                .revision
                .checked_add(1)
                .ok_or(Error::Limit("audio loop revision"))?;
            self.entries -= c.entries.len();
            self.bytes -= c.bytes;
            c.entries.clear();
            c.bytes = 0;
            c.revision = revision;
        }
        Ok(AudioLoopCatalogReceipt {
            client,
            revision: c.revision,
            entries: 0,
        })
    }
    pub fn has_resources(&self) -> bool {
        self.entries != 0
    }
}
/// Explicit endpoint conversion; no decoder, native profile or PlayAsset default
/// invokes this automatically. Native adapters may support only selected rates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopFrameRounding {
    Floor,
    Ceil,
}
pub fn resolve_loop_frames(
    range: crysirror_core::service::AudioLoopRange,
    source_rate: u32,
    rounding: LoopFrameRounding,
) -> Result<crysirror_core::service::AudioLoopRange> {
    use crysirror_core::service::{AudioLoopRange, AudioLoopTimebase};
    if !(8000..=384000).contains(&source_rate) {
        return Err(Error::Invalid("audio loop source rate"));
    }
    if range.end_frame.is_some_and(|end| end <= range.start_frame) {
        return Err(Error::Invalid("audio loop extent"));
    }
    let divisor = match range.timebase {
        AudioLoopTimebase::DecodedFrames => return Ok(range),
        AudioLoopTimebase::ReferenceFrames { sample_rate } => sample_rate,
    };
    if !(8000..=384000).contains(&divisor) {
        return Err(Error::Invalid("audio loop reference rate"));
    }
    let convert = |frame: u64| -> Result<u64> {
        let n = u128::from(frame) * u128::from(source_rate);
        let d = u128::from(divisor);
        let value = match rounding {
            LoopFrameRounding::Floor => n / d,
            LoopFrameRounding::Ceil => (n + d - 1) / d,
        };
        u64::try_from(value).map_err(|_| Error::Limit("audio loop converted frame"))
    };
    let start_frame = convert(range.start_frame)?;
    let end_frame = range.end_frame.map(convert).transpose()?;
    if end_frame.is_some_and(|end| end <= start_frame) {
        return Err(Error::Invalid("audio loop converted extent"));
    }
    Ok(AudioLoopRange {
        timebase: AudioLoopTimebase::DecodedFrames,
        start_frame,
        end_frame,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::AudioLoopRange;
    fn entry(asset: &str, start: u64, end: Option<u64>) -> AudioLoopMetadata {
        AudioLoopMetadata {
            asset: asset.into(),
            range: AudioLoopRange {
                timebase: AudioLoopTimebase::DecodedFrames,
                start_frame: start,
                end_frame: end,
            },
        }
    }
    #[test]
    fn scoped_atomic_explicit_lookup_and_owner_receipts() {
        let mut c = AudioLoopCatalog::default();
        let a = ClientId(1);
        let b = ClientId(2);
        let e = entry("music/a.ogg", 3, Some(10));
        assert_eq!(c.publish(a, &[e.clone()]).unwrap().revision, 1);
        assert_eq!(c.get(a, &e.asset).unwrap(), Some(e.clone()));
        assert_eq!(c.get(b, &e.asset).unwrap(), None);
        assert_eq!(c.publish(a, &[e.clone()]).unwrap().revision, 1);
        for invalid in [
            vec![entry("music/b.ogg", 4, Some(4))],
            vec![entry("../escape", 0, None)],
            vec![e.clone(), e.clone()],
            vec![entry("/absolute", 0, None)],
            vec![entry("music/b.ogg", 0, None); 4097],
        ] {
            assert!(c.publish(a, &invalid).is_err());
            assert_eq!(c.publish(a, &[e.clone()]).unwrap().revision, 1);
            assert_eq!(c.get(a, "music/b.ogg").unwrap(), None);
        }
        assert_eq!(c.clear(b).unwrap().revision, 0);
        assert!(c.has_resources());
        assert_eq!(c.clear(a).unwrap().revision, 2);
        assert!(!c.has_resources());
        assert_eq!(c.clear(a).unwrap().revision, 2);
    }
    #[test]
    fn client_and_revision_limits_do_not_partially_publish() {
        let mut c = AudioLoopCatalog::default();
        let a = ClientId(1);
        let e = entry("a", 0, None);
        c.publish(a, &[e.clone()]).unwrap();
        c.clients.get_mut(&a).unwrap().revision = u64::MAX;
        assert!(c.publish(a, &[entry("new", 0, None)]).is_err());
        assert_eq!(c.get(a, "new").unwrap(), None);
        assert!(c.clear(a).is_err());
        assert_eq!(c.get(a, "a").unwrap(), Some(e));
        for id in 2..=64 {
            c.publish(ClientId(id), &[entry("a", 0, None)]).unwrap();
        }
        assert!(c.publish(ClientId(65), &[entry("a", 0, None)]).is_err());
        assert_eq!(c.get(ClientId(65), "a").unwrap(), None);
    }
    #[test]
    fn reference_frames_have_explicit_rounding_and_checked_extent() {
        let r = AudioLoopRange {
            timebase: AudioLoopTimebase::ReferenceFrames { sample_rate: 44100 },
            start_frame: 5,
            end_frame: Some(11),
        };
        let floor = resolve_loop_frames(r, 22050, LoopFrameRounding::Floor).unwrap();
        assert_eq!((floor.start_frame, floor.end_frame), (2, Some(5)));
        let ceil = resolve_loop_frames(r, 22050, LoopFrameRounding::Ceil).unwrap();
        assert_eq!((ceil.start_frame, ceil.end_frame), (3, Some(6)));
        assert_eq!(
            resolve_loop_frames(r, 44100, LoopFrameRounding::Floor)
                .unwrap()
                .start_frame,
            5
        );
        assert!(resolve_loop_frames(r, 0, LoopFrameRounding::Floor).is_err());
        let overflow = AudioLoopRange {
            start_frame: u64::MAX,
            end_frame: None,
            timebase: AudioLoopTimebase::ReferenceFrames { sample_rate: 8000 },
        };
        assert!(resolve_loop_frames(overflow, 384000, LoopFrameRounding::Floor).is_err());
    }
}
