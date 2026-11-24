// SPDX-License-Identifier: MPL-2.0
//! Prepare seekable input for Resources when the installation wraps an archive in bzip2.
use anyhow::{Context, Result, ensure};
use bzip2::read::MultiBzDecoder;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AssetReader, ClientStorage, HostServices},
    state::ClientId,
    vfs::VfsProvider,
};
use crysirror_resource::AssetIo;
use crysirror_runtime::Runtime;
use crysirror_tasks::Cancellation;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
    sync::Arc,
    time::Duration,
};

// Only the installer's outer stream is spooled. Inner entries remain random-access
// KISS resources. Bound scratch independently of the decoded archive size.
const MAX_DECODED: u64 = 8 << 30;
const BUFFER: usize = 256 << 10;

struct HashReader<R>(R, Sha256);

impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = self.0.read(buf)?;
        self.1.update(&buf[..count]);
        Ok(count)
    }
}

fn prepare(
    storage: &dyn ClientStorage,
    cache: &Path,
    source: &str,
    cancel: &Cancellation,
) -> Result<Arc<dyn AssetReader>> {
    let input = storage.open_asset(&VfsPath::new(Mount::Asset, source)?)?;
    let mut buffer = vec![0; BUFFER];
    let mut scan = HashReader(AssetIo::new(input.clone())?, Sha256::new());
    loop {
        cancel.check()?;
        if scan.read(&mut buffer)? == 0 {
            break;
        }
    }
    let digest = scan.1.finalize();
    let filename = format!("bzip2-v1-{digest:x}.arc");
    let path = VfsPath::new(Mount::Cache, &filename)?;
    match storage.open_read(&path) {
        Ok(reader) => {
            ensure!(
                (1..=MAX_DECODED).contains(&reader.len()?),
                "invalid cached archive size"
            );
            return Ok(reader);
        }
        Err(crysirror_core::Error::NotFound) => {}
        Err(e) => return Err(e.into()),
    }
    log::info!("preparing compressed archive {source} in the game cache");
    let compressed = HashReader(AssetIo::new(input)?, Sha256::new());
    let mut decoder = MultiBzDecoder::new(compressed);
    let mut temporary = tempfile::NamedTempFile::new_in(cache)?;
    let mut total = 0u64;
    loop {
        cancel.check()?;
        let count = decoder.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(total <= MAX_DECODED, "compressed archive expansion limit");
        temporary.write_all(&buffer[..count])?;
    }
    ensure!(
        total != 0 && decoder.into_inner().1.finalize() == digest,
        "empty archive or compressed input changed during preparation"
    );
    cancel.check()?;
    temporary.as_file().sync_all()?;
    temporary.persist(cache.join(filename))?;
    Ok(storage.open_read(&path)?)
}

struct Prepared {
    name: String,
    file: Arc<dyn AssetReader>,
}

impl VfsProvider for Prepared {
    fn kind(&self) -> &'static str {
        "bzip2-cache"
    }
    fn read(&self, relative: &str) -> crysirror_core::Result<Vec<u8>> {
        let reader = self.open(relative)?;
        crysirror_resource::read_all(reader.as_ref(), crysirror_core::limits::LARGE_ASSET_BYTES)
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
    fn open(&self, relative: &str) -> crysirror_core::Result<Arc<dyn AssetReader>> {
        if relative == self.name {
            Ok(self.file.clone())
        } else {
            Err(crysirror_core::Error::NotFound)
        }
    }
    fn exists(&self, relative: &str) -> bool {
        relative == self.name
    }
    fn list(&self) -> crysirror_core::Result<Vec<String>> {
        Ok(vec![self.name.clone()])
    }
}

pub(super) fn mount(
    rt: &mut Runtime,
    source: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<String> {
    let client = ClientId(1);
    let storage = rt.host.storage(client)?;
    let cache = rt.host.fs.desktop_roots(client)?.cache;
    let compressed = source.to_owned();
    let mut job = rt
        .assets
        .cpu_tasks()
        .spawn(move |cancel| prepare(storage.as_ref(), &cache, &compressed, &cancel))?;
    let file = loop {
        ensure!(!cancelled(), "native launch cancelled");
        if let Some(file) = job.try_take()? {
            break file;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let logical = source
        .get(..source.len() - 4)
        .context("invalid compressed archive name")?;
    let (dir, name) = logical.rsplit_once('/').unwrap_or(("", logical));
    rt.mount_provider(
        client,
        dir,
        Arc::new(Prepared {
            name: name.into(),
            file,
        }),
    )?;
    Ok(logical.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_runtime::vfs::DiskVfs;

    #[test]
    fn cache_reopens_random_ranges_and_rejects_incomplete_replacements() -> Result<()> {
        let source = tempfile::tempdir()?;
        let writable = tempfile::tempdir()?;
        let client = ClientId(1);
        let mut fs = DiskVfs::new(source.path(), writable.path())?;
        fs.register(client, "test", "archive")?;
        let cache = fs.desktop_roots(client)?.cache;
        let storage = fs.storage(client)?;
        let member = "motion.arc.bz2";
        let compressed_path = source.path().join(member);
        let payload: Vec<_> = (0..BUFFER + 71).map(|index| (index % 251) as u8).collect();
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        encoder.write_all(&payload)?;
        let compressed = encoder.finish()?;
        std::fs::write(&compressed_path, &compressed)?;
        let cancel = Cancellation::default();
        for _ in 0..2 {
            let reader = prepare(storage.as_ref(), &cache, member, &cancel)?;
            assert_eq!(reader.len()?, payload.len() as u64);
            let mut tail = [0; 10];
            let pos = payload.len() - tail.len();
            crysirror_resource::read_exact_at(reader.as_ref(), pos as u64, &mut tail)?;
            assert_eq!(&tail, &payload[pos..]);
        }
        std::fs::write(&compressed_path, &compressed[..compressed.len() - 4])?;
        assert!(prepare(storage.as_ref(), &cache, member, &cancel).is_err());
        cancel.cancel();
        assert!(prepare(storage.as_ref(), &cache, member, &cancel).is_err());
        assert_eq!(std::fs::read_dir(cache)?.count(), 1);
        Ok(())
    }
}
