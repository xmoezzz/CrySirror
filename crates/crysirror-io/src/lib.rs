// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared capability read-at to standard cursor bridge, without codecs/devices.
#![forbid(unsafe_code)]
use anyhow::Result;
use crysirror_core::service::AssetReader;
use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

pub struct AssetIo {
    source: Arc<dyn AssetReader>,
    position: u64,
    len: u64,
}
impl AssetIo {
    /// Length captured on the opening worker; querying it performs no I/O.
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn new(source: Arc<dyn AssetReader>) -> Result<Self> {
        let len = source.len().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self {
            source,
            position: 0,
            len,
        })
    }
}
impl Read for AssetIo {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self
            .source
            .read_at(self.position, out)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        if n > out.len() {
            return Err(std::io::Error::other("invalid asset read length"));
        }
        self.position = self
            .position
            .checked_add(n as u64)
            .ok_or_else(|| std::io::Error::other("stream offset overflow"))?;
        Ok(n)
    }
}
impl Seek for AssetIo {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let n = match position {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.len) + i128::from(n),
        };
        if !(0..=i128::from(u64::MAX)).contains(&n) {
            return Err(std::io::Error::other("invalid resource seek"));
        }
        self.position = n as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::MemoryAssetReader;
    #[test]
    fn capability_cursor_handles_partial_reads_end_relative_seeks_and_overflow() {
        let mut io = AssetIo::new(Arc::new(MemoryAssetReader(Arc::from(&b"abcdef"[..])))).unwrap();
        assert_eq!(io.len(), 6);
        let mut out = [0; 4];
        assert_eq!(io.read(&mut out).unwrap(), 4);
        assert_eq!(&out, b"abcd");
        assert_eq!(io.seek(SeekFrom::End(-2)).unwrap(), 4);
        assert_eq!(io.read(&mut out).unwrap(), 2);
        assert_eq!(&out[..2], b"ef");
        assert_eq!(io.read(&mut out).unwrap(), 0);
        assert!(io.seek(SeekFrom::Start(0)).is_ok());
        assert!(io.seek(SeekFrom::Current(-1)).is_err());
        assert_eq!(io.stream_position().unwrap(), 0);
        io.seek(SeekFrom::Start(u64::MAX)).unwrap();
        assert!(io.seek(SeekFrom::Current(1)).is_err());
        assert_eq!(io.stream_position().unwrap(), u64::MAX);
    }
    #[test]
    fn provider_cannot_report_more_bytes_than_the_destination() {
        struct Bad;
        impl AssetReader for Bad {
            fn len(&self) -> crysirror_core::Result<u64> {
                Ok(8)
            }
            fn read_at(&self, _: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
                Ok(out.len() + 1)
            }
        }
        assert!(
            AssetIo::new(Arc::new(Bad))
                .unwrap()
                .read(&mut [0; 2])
                .is_err()
        );
    }
}
