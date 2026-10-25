// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{service::AssetReader, state::ClientId, vfs::VfsProvider};
use crysirror_resource::*;
use serde_json::json;
use std::{io::Write, sync::Arc};

struct Sparse(u64);
impl AssetReader for Sparse {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.0)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let n = out
            .len()
            .min(self.0.saturating_sub(offset).min(usize::MAX as u64) as usize);
        for (i, b) in out[..n].iter_mut().enumerate() {
            *b = (offset + i as u64) as u8;
        }
        Ok(n)
    }
}
#[test]
fn archive_ranges_stream_gigabytes_without_allocating_the_file() {
    let source: Arc<dyn AssetReader> = Arc::new(Sparse(3 * 1024 * 1024 * 1024));
    assert!(subrange(source.clone(), u64::MAX - 5, 10).is_err());
    let part = subrange(source, 2 * 1024 * 1024 * 1024 - 2, 20).unwrap();
    let joined = concat(vec![part, io::memory(vec![1, 2, 3])]).unwrap();
    let mut out = [0; 7];
    read_exact_at(joined.as_ref(), 16, &mut out).unwrap();
    assert_eq!(out, [14, 15, 16, 17, 1, 2, 3]);
    assert_eq!(joined.read_at(23, &mut out).unwrap(), 0);
    assert!(read_all(joined.as_ref(), 22).is_err());
}
fn context() -> DecodeContext {
    DecodeContext::new(ClientId(7))
}
#[test]
fn compressed_entries_check_lengths_corruption_bombs_and_shared_budget_lifetimes() {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&vec![42; 90_000]).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut ctx = context();
    ctx.limits.max_entry_size = 100_000;
    ctx.limits.max_resident_bytes = 100_000;
    let mut spec = TransformSpec::new("zlib");
    spec.expected_size = Some(90_000);
    let decoded = ctx
        .apply(&spec, io::memory(compressed.clone()), &json!(null))
        .unwrap();
    assert_eq!(ctx.resident_bytes(), 90_000);
    assert!(ctx
        .clone()
        .apply(&spec, io::memory(compressed.clone()), &json!(null))
        .is_err());
    assert_eq!(
        read_all(decoded.as_ref(), 100_000).unwrap(),
        vec![42; 90_000]
    );
    drop(decoded);
    assert_eq!(ctx.resident_bytes(), 0);
    let truncated = compressed[..compressed.len() - 2].to_vec();
    assert!(ctx
        .apply(&spec, io::memory(truncated), &json!(null))
        .is_err());
    assert_eq!(ctx.resident_bytes(), 0);
    spec.expected_size = Some(100);
    assert!(ctx
        .apply(&spec, io::memory(compressed.clone()), &json!(null))
        .is_err());
    spec.expected_size = Some(95_000);
    assert!(ctx
        .apply(&spec, io::memory(compressed), &json!(null))
        .is_err());
    assert_eq!(ctx.resident_bytes(), 0);
}
#[test]
fn key_authority_is_scoped_and_xor_supports_unaligned_random_reads() {
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(ClientId(7), "game", vec![0x51, 0x72, 0x93])
        .unwrap();
    assert!(keys.key(ClientId(8), "game").is_err());
    assert_eq!(
        format!("{:?}", keys.key(ClientId(7), "game").unwrap()),
        "KeyMaterial([redacted])"
    );
    let mut ctx = context();
    ctx.keys = keys;
    let spec = TransformSpec {
        algorithm: "xor".into(),
        key_id: Some("game".into()),
        parameters: json!({"offset":2}),
        expected_size: Some(19),
    };
    let reader = ctx
        .apply(&spec, io::memory(vec![0; 19]), &json!(null))
        .unwrap();
    let mut out = [0; 7];
    read_exact_at(reader.as_ref(), 5, &mut out).unwrap();
    assert_eq!(out, [0x72, 0x93, 0x51, 0x72, 0x93, 0x51, 0x72]);
    ctx.client = ClientId(8);
    assert!(ctx
        .apply(&spec, io::memory(vec![0; 19]), &json!(null))
        .is_err());
}
fn hex(s: &str) -> Vec<u8> {
    assert_eq!(s.len() % 2, 0);
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn aes_ctr_matches_nist_sp800_38a_f5_and_seeks_across_blocks() {
    // NIST SP 800-38A F.5.1, a published known-answer vector, not a self-roundtrip.
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(ClientId(7), "nist", hex("2b7e151628aed2a6abf7158809cf4f3c"))
        .unwrap();
    let mut ctx = context();
    ctx.keys = keys;
    let ciphertext=hex("874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff5ae4df3edbd5d35e5b4f09020db03eab");
    let plain=hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52ef");
    let spec = TransformSpec {
        algorithm: "aes-ctr128be".into(),
        key_id: Some("nist".into()),
        parameters: json!({"iv":hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff")}),
        expected_size: None,
    };
    let reader = ctx
        .apply(&spec, io::memory(ciphertext.clone()), &json!(null))
        .unwrap();
    assert_eq!(read_all(reader.as_ref(), 48).unwrap(), plain);
    let mut out = [0; 27];
    read_exact_at(reader.as_ref(), 11, &mut out).unwrap();
    assert_eq!(out, plain[11..38]);
    let mut sliced = spec.clone();
    sliced.parameters["offset"] = json!(11);
    let reader = ctx
        .apply(&sliced, io::memory(ciphertext[11..].to_vec()), &json!(null))
        .unwrap();
    assert_eq!(read_all(reader.as_ref(), 48).unwrap(), plain[11..]);
}
struct Fixture {
    entries: Vec<ArchiveEntry>,
}
impl Archive for Fixture {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }
    fn open(&self, id: u32, _: &DecodeContext) -> anyhow::Result<Arc<dyn AssetReader>> {
        anyhow::ensure!(id == 0, "missing");
        Ok(io::memory(b"pixels".to_vec()))
    }
}
fn entry(path: &str, id: u32) -> ArchiveEntry {
    ArchiveEntry {
        id,
        path: path.into(),
        size: 6,
        stored_size: 6,
        metadata: json!({}),
    }
}
#[test]
fn mounts_reject_traversal_alias_collisions_and_duplicate_ids() {
    for entries in [
        vec![entry("../escape", 0)],
        vec![entry("a/./b", 0), entry("a/b", 1)],
        vec![entry("a", 0), entry("b", 0)],
    ] {
        assert!(ArchiveMount::new(Box::new(Fixture { entries }), context()).is_err());
    }
    let mount = ArchiveMount::new(
        Box::new(Fixture {
            entries: vec![entry("image\\pixel.bin", 0)],
        }),
        context(),
    )
    .unwrap();
    assert_eq!(mount.list().unwrap(), vec!["image/pixel.bin"]);
    assert_eq!(
        read_all(mount.open("image/pixel.bin").unwrap().as_ref(), 6).unwrap(),
        b"pixels"
    );
    assert!(matches!(
        mount.open("missing"),
        Err(crysirror_core::Error::NotFound)
    ));
}
struct Detector(&'static str);
impl ArchiveFormat for Detector {
    fn id(&self) -> &'static str {
        self.0
    }
    fn probe(&self, source: &dyn AssetReader) -> anyhow::Result<Probe> {
        let mut b = [0];
        read_exact_at(source, 0, &mut b)?;
        Ok(Probe::YES)
    }
    fn open(&self, _: Arc<dyn AssetReader>, _: &DecodeContext) -> anyhow::Result<Box<dyn Archive>> {
        Ok(Box::new(Fixture {
            entries: vec![entry("a", 0)],
        }))
    }
}
#[test]
fn ambiguous_or_unknown_formats_fail_and_probing_is_bounded() {
    let mut formats = FormatRegistry::default();
    formats.register(Arc::new(Detector("a"))).unwrap();
    formats.register(Arc::new(Detector("b"))).unwrap();
    assert!(formats.open(io::memory(vec![0]), None, &context()).is_err());
    assert_eq!(
        formats
            .open(io::memory(vec![0]), Some("a"), &context())
            .unwrap()
            .0,
        "a"
    );
    assert!(formats
        .open(io::memory(vec![0]), Some("missing"), &context())
        .is_err());
    let mut ctx = context();
    ctx.limits.max_probe_bytes = 0;
    assert!(formats.probe(io::memory(vec![0]), &ctx).is_err());
}
#[test]
fn encryption_requires_explicit_profiles_and_unknown_algorithms_fail() {
    let ctx = context();
    let ent = entry("a", 0);
    assert!(ctx
        .apply_pipeline("fixture", &ent, io::memory(b"pixels".to_vec()), true)
        .is_err());
    assert!(ctx
        .apply(
            &TransformSpec::new("invented"),
            io::memory(vec![]),
            &json!(null)
        )
        .is_err());
    assert!(ctx
        .apply(
            &TransformSpec::new("xor"),
            io::memory(vec![1]),
            &json!(null)
        )
        .is_err());
}

#[test]
fn custom_codec_reserves_before_allocating_and_releases_after_error() {
    let mut context = context();
    context.limits.max_resident_bytes = 10;
    let invoked = std::cell::Cell::new(false);
    assert!(context
        .decode_with(11, || {
            invoked.set(true);
            Ok(vec![0; 11])
        })
        .is_err());
    assert!(!invoked.get());
    assert!(context
        .decode_with(10, || anyhow::bail!("malformed custom codec"))
        .is_err());
    assert_eq!(context.resident_bytes(), 0);
    assert!(context.decode_with(10, || Ok(vec![0; 9])).is_err());
    assert_eq!(context.resident_bytes(), 0);
}
