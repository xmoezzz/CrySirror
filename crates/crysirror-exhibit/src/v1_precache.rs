// SPDX-License-Identifier: MPL-2.0
//! RetouchSystem::cmdPreCache. The v1 handler (0x67960), 2011 handler
//! (0xb3150), and late FCFile workers use the same 8 MiB read boundary.
use anyhow::Result;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::AssetReader,
};
use crysirror_tasks::Cancellation;

pub(super) fn path(raw: &[u8]) -> Option<VfsPath> {
    let name = crysirror_resource::text_stream::decode_shift_jis(raw, 4096).ok()?;
    VfsPath::new(Mount::Asset, &name.replace('\\', "/")).ok()
}

pub(super) fn read(reader: &dyn AssetReader, mib: u32, cancel: &Cancellation) -> Result<()> {
    const BLOCK: u32 = 8 << 20;
    let Ok(length) = reader.len() else {
        return Ok(());
    };
    // The original GetFileSize call has no high DWORD output.
    let length = length as u32;
    if length == u32::MAX || length == 0 {
        return Ok(());
    }
    let limit = if mib == 0 {
        length
    } else {
        length.min(mib << 20)
    };
    let chunk = limit.min(BLOCK) as usize;
    if chunk == 0 {
        return Ok(());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(chunk)?;
    bytes.resize(chunk, 0);
    let mut offset = 0u64;
    loop {
        cancel.check()?;
        let Ok(count) = reader.read_at(offset, &mut bytes) else {
            break;
        };
        offset += count as u64;
        // On files longer than 8 MiB the limited path reads full blocks,
        // including the one containing the requested prefix's last byte.
        // The unlimited path continues until a short read, even at exact EOF.
        if count != chunk || limit <= BLOCK || (limit < length && offset >= u64::from(limit)) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Reads {
        length: u64,
        requests: Mutex<Vec<(u64, usize)>>,
    }
    impl AssetReader for Reads {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self.length)
        }
        fn read_at(&self, offset: u64, bytes: &mut [u8]) -> crysirror_core::Result<usize> {
            self.requests.lock().unwrap().push((offset, bytes.len()));
            Ok((self.length.saturating_sub(offset)).min(bytes.len() as u64) as usize)
        }
    }
    #[test]
    fn native_prefix_boundaries_and_whole_file_eof_reads() -> Result<()> {
        let mib = 1 << 20;
        for (length, requested, expected) in [
            (13, 1, vec![(0, 13)]),
            (20 * mib, 3, vec![(0, 3 * mib)]),
            (20 * mib, 9, vec![(0, 8 * mib), (8 * mib, 8 * mib)]),
            (
                16 * mib,
                16,
                vec![(0, 8 * mib), (8 * mib, 8 * mib), (16 * mib, 8 * mib)],
            ),
            (10 * mib, 0, vec![(0, 8 * mib), (8 * mib, 8 * mib)]),
            (u64::from(u32::MAX), 9, vec![]),
        ] {
            let reader = Reads {
                length,
                requests: Mutex::default(),
            };
            read(&reader, requested, &Cancellation::default())?;
            let expected: Vec<_> = expected.into_iter().map(|(i, n)| (i, n as usize)).collect();
            assert_eq!(*reader.requests.lock().unwrap(), expected);
        }
        assert_eq!(path(b".\\res\\a.gyu").unwrap().relative(), "res/a.gyu");
        assert!(path(b"..\\a.gyu").is_none());
        Ok(())
    }
}
