// SPDX-License-Identifier: MPL-2.0
//! Shared file-decoder contracts, independently of plugins, VMs and devices.
use anyhow::{bail, Result};
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{
    io::memory, read_all, read_exact_at, DecodeContext, DecodedFile, FileFormat, FileKind,
    FileRegistry, Probe,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
struct Decoder {
    id: &'static str,
    confidence: u8,
    probe_bytes: usize,
    mode: u8,
    calls: Arc<AtomicUsize>,
}
impl FileFormat for Decoder {
    fn id(&self) -> &'static str {
        self.id
    }
    fn kind(&self) -> FileKind {
        if self.mode == 4 {
            FileKind::Audio
        } else {
            FileKind::File
        }
    }
    fn probe(&self, _: &str, source: &dyn AssetReader) -> Result<Probe> {
        let mut bytes = vec![0; self.probe_bytes];
        read_exact_at(source, 0, &mut bytes)?;
        Ok(Probe {
            confidence: self.confidence,
        })
    }
    fn decode(
        &self,
        path: &str,
        _: Arc<dyn AssetReader>,
        cx: &DecodeContext,
    ) -> Result<DecodedFile> {
        let calls = self.calls.clone();
        let reader = cx.decode_bounded(8, || {
            calls.fetch_add(1, Ordering::SeqCst);
            if self.mode == 1 {
                bail!("synthetic codec failure");
            }
            Ok(vec![42; if self.mode == 2 { 9 } else { 4 }])
        })?;
        Ok(DecodedFile {
            reader,
            extension: if self.mode == 3 { "../bad" } else { "dat" }.into(),
            metadata: serde_json::json!({"source":cx.source_name,"requested":path}),
        })
    }
}
fn add(
    r: &mut FileRegistry,
    id: &'static str,
    confidence: u8,
    probe_bytes: usize,
    mode: u8,
) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    r.register(Arc::new(Decoder {
        id,
        confidence,
        probe_bytes,
        mode,
        calls: calls.clone(),
    }))
    .unwrap();
    calls
}
fn cx() -> DecodeContext {
    DecodeContext::new(ClientId(7))
}
#[test]
fn registry_requires_unique_detection_and_explicit_unknown_ids_fail() {
    let mut r = FileRegistry::default();
    let a = add(&mut r, "test/a", 100, 0, 0);
    let b = add(&mut r, "test/b", 100, 0, 4);
    let cx = cx();
    assert_eq!(r.formats(FileKind::File), ["test/a"]);
    assert_eq!(r.formats(FileKind::Audio), ["test/b"]);
    assert!(r.decode("x", memory(vec![]), None, &cx).is_err());
    assert!(r.decode("x", memory(vec![]), Some("missing"), &cx).is_err());
    assert_eq!(a.load(Ordering::SeqCst) + b.load(Ordering::SeqCst), 0);
    let (id, out) = r
        .decode("dir/input.bin", memory(vec![]), Some("test/a"), &cx)
        .unwrap();
    assert_eq!(id, "test/a");
    assert_eq!(out.metadata["source"], "dir/input.bin");
    assert_eq!(read_all(out.reader.as_ref(), 10).unwrap(), [42; 4]);
    assert_eq!(cx.resident_bytes(), 4);
    drop(out);
    assert_eq!(cx.resident_bytes(), 0);
    let mut absent = FileRegistry::default();
    add(&mut absent, "test/no", 0, 0, 0);
    assert!(absent.decode("x", memory(vec![]), None, &cx).is_err());
}
#[test]
fn confidence_winner_and_per_format_probe_quota_are_enforced() {
    let mut r = FileRegistry::default();
    let low = add(&mut r, "test/low", 50, 4, 0);
    let high = add(&mut r, "test/high", 100, 4, 0);
    let mut cx = cx();
    cx.limits.max_probe_bytes = 4;
    let (_, out) = r.decode("x", memory(vec![0; 8]), None, &cx).unwrap();
    assert_eq!(out.metadata["requested"], "x");
    assert_eq!(low.load(Ordering::SeqCst), 0);
    assert_eq!(high.load(Ordering::SeqCst), 1);
    drop(out);
    cx.limits.max_probe_bytes = 3;
    assert!(r.probe("x", memory(vec![0; 8]), &cx).is_err());
    assert_eq!(cx.resident_bytes(), 0);
    let mut invalid = FileRegistry::default();
    add(&mut invalid, "test/invalid", 101, 0, 0);
    assert!(invalid.probe("x", memory(vec![]), &cx).is_err());
}
#[test]
fn quotas_admit_before_codec_and_all_failure_paths_preserve_other_allocations() {
    let mut r = FileRegistry::default();
    let calls = add(&mut r, "test/ok", 100, 0, 0);
    let mut cx = cx();
    cx.limits.max_entry_size = 7;
    assert!(r.decode("x", memory(vec![]), Some("test/ok"), &cx).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(cx.resident_bytes(), 0);
    cx.limits.max_entry_size = 16;
    cx.limits.max_resident_bytes = 10;
    let retained = cx.memory(vec![9; 4]).unwrap();
    assert!(r.decode("x", memory(vec![]), Some("test/ok"), &cx).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(cx.resident_bytes(), 4);
    drop(retained);
    let (_, held) = r.decode("x", memory(vec![]), Some("test/ok"), &cx).unwrap();
    assert_eq!(cx.resident_bytes(), 4);
    for mode in [1, 2, 3] {
        let mut bad = FileRegistry::default();
        let ran = add(&mut bad, "test/bad", 100, 0, mode);
        assert!(bad
            .decode("x", memory(vec![]), Some("test/bad"), &cx)
            .is_err());
        assert_eq!(ran.load(Ordering::SeqCst), 0, "quota should precede codec");
        assert_eq!(cx.resident_bytes(), 4);
    }
    drop(held);
    for mode in [1, 2, 3] {
        let mut bad = FileRegistry::default();
        let ran = add(&mut bad, "test/bad", 100, 0, mode);
        assert!(bad
            .decode("x", memory(vec![]), Some("test/bad"), &cx)
            .is_err());
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert_eq!(cx.resident_bytes(), 0);
    }
}
#[test]
fn companion_paths_are_canonical_and_scoped_before_resolver_calls() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let mut cx = cx();
    cx.source_name = Some("images/main.wip".into());
    cx.resolver = Some(Arc::new(move |path: &str| {
        observed.lock().unwrap().push(path.to_string());
        Ok(memory(b"mask".to_vec()))
    }));
    let out = cx.open_sibling("main.msk").unwrap();
    assert_eq!(read_all(out.as_ref(), 8).unwrap(), b"mask");
    assert_eq!(*calls.lock().unwrap(), ["images/main.msk"]);
    for path in [
        "../secret",
        "/secret",
        "a//b",
        "a/./b",
        "a/../b",
        "a\\b",
        "C:/outside",
        "x\0y",
    ] {
        assert!(cx.open_sibling(path).is_err(), "{path:?}");
        assert!(cx.open_dependency(path).is_err(), "{path:?}");
    }
    assert_eq!(calls.lock().unwrap().len(), 1);
    cx.resolver = None;
    assert!(cx.open_sibling("main.msk").is_err());
}

#[test]
fn companion_cycles_and_deep_chains_fail_without_poisoning_later_reads() {
    let owner = Arc::new(Mutex::new(None::<DecodeContext>));
    let weak = Arc::downgrade(&owner);
    let mut context = cx();
    context.resolver = Some(Arc::new(
        move |path: &str| -> Result<Arc<dyn AssetReader>> {
            let context = weak
                .upgrade()
                .unwrap()
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .clone();
            match path {
                "cycle/a" => context.open_dependency("cycle/b"),
                "cycle/b" => context.open_dependency("cycle/a"),
                "good" => Ok(memory(b"ok".to_vec())),
                _ => {
                    let n = path.strip_prefix("depth/").unwrap().parse::<u32>()?;
                    context.open_dependency(&format!("depth/{}", n + 1))
                }
            }
        },
    ));
    *owner.lock().unwrap() = Some(context.clone());
    assert!(context
        .open_dependency("cycle/a")
        .err()
        .unwrap()
        .to_string()
        .contains("cycle"));
    assert!(context
        .open_dependency("depth/0")
        .err()
        .unwrap()
        .to_string()
        .contains("recursion"));
    assert_eq!(
        read_all(context.open_dependency("good").unwrap().as_ref(), 8).unwrap(),
        b"ok"
    );
    assert!(context.open_dependency("cycle/a").is_err());
}

#[test]
fn temporary_codec_charge_is_admitted_before_work_and_released_on_error() {
    let mut cx = cx();
    cx.limits.max_resident_bytes = 8;
    let retained = cx.decode_with(4, || Ok(vec![0; 4])).unwrap();
    let calls = AtomicUsize::new(0);
    assert!(cx
        .with_temporary_bytes(5, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let failure = cx.with_temporary_bytes(4, || -> Result<()> {
        assert_eq!(cx.resident_bytes(), 8);
        bail!("codec error")
    });
    assert!(failure.is_err());
    assert_eq!(cx.resident_bytes(), 4);
    cx.with_temporary_bytes(4, || Ok(())).unwrap();
    drop(retained);
    assert_eq!(cx.resident_bytes(), 0);
}
