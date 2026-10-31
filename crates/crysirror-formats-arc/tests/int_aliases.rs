// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored INT namespace fixtures; no owner archive bytes or keys.
use crysirror_core::{service::MemoryAssetReader, state::ClientId, vfs::VfsProvider};
use crysirror_resource::{ArchiveMount, DecodeContext};
use std::sync::Arc;
struct CheckContext(Box<dyn crysirror_resource::Archive>);
impl crysirror_resource::Archive for CheckContext {
    fn entries(&self) -> &[crysirror_resource::ArchiveEntry] {
        self.0.entries()
    }
    fn open(
        &self,
        id: u32,
        cx: &DecodeContext,
    ) -> anyhow::Result<Arc<dyn crysirror_core::service::AssetReader>> {
        cx.check_cancelled()?;
        self.0.open(id, cx)
    }
}
fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = b"KIF\0".to_vec();
    b.extend((files.len() as u32).to_le_bytes());
    let mut at = 8 + 72 * files.len();
    for (name, data) in files {
        let mut name = name.as_bytes().to_vec();
        name.resize(64, 0);
        b.extend(name);
        b.extend((at as u32).to_le_bytes());
        b.extend((data.len() as u32).to_le_bytes());
        at += data.len();
    }
    for (_, data) in files {
        b.extend_from_slice(data);
    }
    b
}
#[test]
fn equivalent_rows_have_one_mounted_name_and_recoverable_native_identity() {
    let bytes = archive(&[
        ("dir//same.bin", b"first"),
        ("dir/same.bin", b"first"),
        ("other.bin", b"second"),
    ]);
    let format = crysirror_formats_arc::legacy_archive("cat-system/int").unwrap();
    let mut cx = DecodeContext::new(ClientId(1));
    let token = crysirror_tasks::Cancellation::default();
    cx = cx.with_cancellation(token.clone());
    let native = format
        .open(Arc::new(MemoryAssetReader(bytes.into())), &cx)
        .unwrap();
    assert_eq!(native.entries().len(), 2);
    let rows = &native.entries()[0].metadata["identical_native_rows"];
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["entry_id"], 0);
    assert_eq!(rows[1]["entry_id"], 1);
    assert_eq!(rows[0]["native_index"], 0);
    assert_eq!(rows[1]["native_index"], 1);
    let mounted = ArchiveMount::new(Box::new(CheckContext(native)), cx).unwrap();
    token.cancel(); // completed indexing cancellation cannot poison live mounts
    assert_eq!(mounted.read("dir/same.bin").unwrap(), b"first");
    assert_eq!(mounted.read("other.bin").unwrap(), b"second");
}
#[test]
fn different_duplicate_contents_and_canceled_work_fail_before_publication() {
    let format = crysirror_formats_arc::legacy_archive("cat-system/int").unwrap();
    let cx = DecodeContext::new(ClientId(1));
    let source = Arc::new(MemoryAssetReader(
        archive(&[("same", b"first"), ("same", b"other")]).into(),
    ));
    let error = format.open(source.clone(), &cx).err().unwrap().to_string();
    assert!(error.contains("conflicting duplicate payload"), "{error}");
    let token = crysirror_tasks::Cancellation::default();
    token.cancel();
    assert!(format
        .open(source, &cx.with_cancellation(token))
        .err()
        .unwrap()
        .to_string()
        .contains("cancelled"));
}

#[test]
fn alias_compare_rejects_work_over_budget_before_reading_large_bodies() {
    struct Sparse {
        header: Vec<u8>,
    }
    impl crysirror_core::service::AssetReader for Sparse {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(40 * 1024 * 1024)
        }
        fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            let at = at as usize;
            assert!(
                at + out.len() <= self.header.len(),
                "body read before alias budget admission"
            );
            out.copy_from_slice(&self.header[at..at + out.len()]);
            Ok(out.len())
        }
    }
    let mut header = archive(&[("same", b""), ("same", b"")]);
    for (i, offset) in [(0, 152u32), (1, 153u32)] {
        let at = 8 + i * 72 + 64;
        header[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        header[at + 4..at + 8].copy_from_slice(&(33u32 * 1024 * 1024).to_le_bytes());
    }
    let f = crysirror_formats_arc::legacy_archive("cat-system/int").unwrap();
    let error = f
        .open(
            Arc::new(Sparse { header }),
            &DecodeContext::new(ClientId(1)),
        )
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("comparison byte budget"), "{error}");
}
