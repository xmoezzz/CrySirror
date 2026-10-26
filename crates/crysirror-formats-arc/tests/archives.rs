// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored archives; no game or upstream binary fixtures are distributed.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
    vfs::VfsProvider,
};
use crysirror_formats_arc::{BgiArc, MajiroArc, Nscripter, Xp3};
use crysirror_resource::{
    read_all, ArchiveFormat, ArchiveMount, DecodeContext, FormatRegistry, MemoryKeyProvider,
    PipelineRules, TransformSpec,
};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
fn memory(data: Vec<u8>) -> Arc<dyn AssetReader> {
    Arc::new(MemoryAssetReader(Arc::from(data)))
}
fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    out.write_all(data).unwrap();
    out.finish().unwrap()
}
fn name(name: &str) -> Vec<u8> {
    let (out, _, errors) = encoding_rs::SHIFT_JIS.encode(name);
    assert!(!errors);
    out.into_owned()
}
fn bgi(modern: bool, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let (n, skip) = if modern { (96, 24) } else { (16, 8) };
    let mut data = if modern {
        b"BURIKO ARC20".to_vec()
    } else {
        b"PackFile    ".to_vec()
    };
    data.extend((files.len() as u32).to_le_bytes());
    let mut offset = 0u32;
    for (path, body) in files {
        let mut p = name(path);
        p.resize(n, 0);
        data.extend(p);
        data.extend(offset.to_le_bytes());
        data.extend((body.len() as u32).to_le_bytes());
        data.extend(vec![0; skip]);
        offset += body.len() as u32;
    }
    for (_, body) in files {
        data.extend(body);
    }
    data
}
fn majiro(version: u32, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut names = Vec::new();
    for (path, _) in files {
        names.extend(name(path));
        names.push(0);
    }
    let names_at = 28 + files.len() * (if version == 3 { 16 } else { 12 });
    let data_at = names_at + names.len();
    let mut offset = data_at as u32;
    let mut out = format!("MajiroArcV{version}.000\0").into_bytes();
    out.extend((files.len() as u32).to_le_bytes());
    out.extend((names_at as u32).to_le_bytes());
    out.extend((data_at as u32).to_le_bytes());
    for (i, (_, body)) in files.iter().enumerate() {
        if version == 3 {
            out.extend((0x1122334400000000u64 + i as u64).to_le_bytes());
        } else {
            out.extend((0x12345678u32 + i as u32).to_le_bytes());
        }
        out.extend(offset.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        offset += body.len() as u32;
    }
    out.extend(names);
    for (_, body) in files {
        out.extend(body);
    }
    out
}
fn nsar(nsa: bool, files: &[(&str, u8, Vec<u8>, u32)]) -> Vec<u8> {
    let table_size: usize = files
        .iter()
        .map(|(path, _, _, _)| name(path).len() + 1 + if nsa { 13 } else { 8 })
        .sum();
    let mut out = (files.len() as u16).to_be_bytes().to_vec();
    out.extend((6 + table_size as u32).to_be_bytes());
    let mut offset = 0u32;
    for (path, kind, body, decoded) in files {
        out.extend(name(path));
        out.push(0);
        if nsa {
            out.push(*kind);
        }
        out.extend(offset.to_be_bytes());
        out.extend((body.len() as u32).to_be_bytes());
        if nsa {
            out.extend(decoded.to_be_bytes());
        }
        offset += body.len() as u32;
    }
    for (_, _, body, _) in files {
        out.extend(body);
    }
    out
}
fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = tag.to_vec();
    out.extend((body.len() as u64).to_le_bytes());
    out.extend(body);
    out
}
fn xp3(version: u32, index_compressed: bool, encrypted: bool) -> Vec<u8> {
    let mut out = b"XP3\r\n \n\x1a\x8bg\x01".to_vec();
    out.extend(0u64.to_le_bytes());
    if version == 2 {
        out[11..19].copy_from_slice(&23u64.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.push(0);
        out.extend(0u64.to_le_bytes());
        out.extend(0u64.to_le_bytes());
    }
    let key = b"test-only";
    let first = b"first-";
    let second = b"second";
    let mut body = Vec::new();
    body.extend(first);
    body.extend(second);
    if encrypted {
        for (i, b) in body.iter_mut().enumerate() {
            *b ^= key[i % key.len()];
        }
    }
    let split = first.len();
    let pieces = [
        (body[..split].to_vec(), false),
        (zlib(&body[split..]), true),
    ];
    let mut segm = Vec::new();
    let mut stored = 0;
    for ((packed, compressed), size) in pieces.iter().zip([first.len(), second.len()]) {
        let offset = out.len() as u64;
        out.extend(packed);
        segm.extend(u32::from(*compressed).to_le_bytes());
        segm.extend(offset.to_le_bytes());
        segm.extend((size as u64).to_le_bytes());
        segm.extend((packed.len() as u64).to_le_bytes());
        stored += packed.len();
    }
    let path: Vec<_> = "scene/日本.txt".encode_utf16().collect();
    let mut info = if encrypted { 0x80000000u32 } else { 0 }
        .to_le_bytes()
        .to_vec();
    info.extend((body.len() as u64).to_le_bytes());
    info.extend((stored as u64).to_le_bytes());
    info.extend((path.len() as u16).to_le_bytes());
    for unit in path {
        info.extend(unit.to_le_bytes());
    }
    let mut file = chunk(b"time", &12345678u64.to_le_bytes());
    file.extend(chunk(b"segm", &segm));
    file.extend(chunk(b"adlr", &0x33445566u32.to_le_bytes()));
    file.extend(chunk(b"info", &info));
    let table = chunk(b"File", &file);
    let table_offset = out.len() as u64;
    if version == 2 {
        out[32..40].copy_from_slice(&table_offset.to_le_bytes());
    } else {
        out[11..19].copy_from_slice(&table_offset.to_le_bytes());
    }
    out.push(u8::from(index_compressed));
    let packed = if index_compressed {
        zlib(&table)
    } else {
        table.clone()
    };
    out.extend((packed.len() as u64).to_le_bytes());
    if index_compressed {
        out.extend((table.len() as u64).to_le_bytes());
    }
    out.extend(packed);
    out
}
#[test]
fn bgi_and_majiro_variants_mount_paths_and_random_access_without_unpacking() {
    let files = vec![
        ("abc.txt", b"first".to_vec()),
        ("日本.txt", b"second".to_vec()),
    ];
    let ctx = DecodeContext::new(ClientId(7));
    for (format, data) in [
        (
            Box::new(BgiArc) as Box<dyn ArchiveFormat>,
            bgi(false, &files),
        ),
        (Box::new(BgiArc), bgi(true, &files)),
        (Box::new(MajiroArc), majiro(2, &files)),
        (Box::new(MajiroArc), majiro(3, &files)),
    ] {
        let source = memory(data);
        assert_eq!(format.probe(source.as_ref()).unwrap().confidence, 100);
        let archive = format.open(source, &ctx).unwrap();
        assert_eq!(archive.entries().len(), 2);
        let mount = ArchiveMount::new(archive, ctx.clone()).unwrap();
        assert_eq!(mount.read("日本.txt").unwrap(), b"second");
        let reader = mount.open_entry("abc.txt").unwrap();
        let mut part = [0; 3];
        assert_eq!(reader.read_at(1, &mut part).unwrap(), 3);
        assert_eq!(&part, b"irs");
        assert_eq!(reader.read_at(5, &mut part).unwrap(), 0);
        assert_eq!(ctx.resident_bytes(), 0);
    }
}
#[test]
fn nscripter_plain_and_lzss_keep_native_big_endian_fields_and_overlap() {
    let ctx = DecodeContext::new(ClientId(1));
    let plain = [("dir/日本.txt", 0, b"ABC".to_vec(), 3)];
    for (format, data) in [
        (Nscripter::sar(), nsar(false, &plain)),
        (Nscripter::nsa(), nsar(true, &plain)),
    ] {
        let archive = format.open(memory(data), &ctx).unwrap();
        assert_eq!(archive.entries()[0].path, "dir/日本.txt");
        assert_eq!(
            read_all(archive.open(0, &ctx).unwrap().as_ref(), 100).unwrap(),
            b"ABC"
        );
    }
    // Literal A at dictionary 239 followed by a 5-byte overlapping back-reference.
    let mut bits = Vec::new();
    for (n, value) in [(1, 1u32), (8, 65), (1, 0), (8, 239), (4, 3)] {
        for bit in (0..n).rev() {
            bits.push(((value >> bit) & 1) as u8);
        }
    }
    let packed: Vec<u8> = bits
        .chunks(8)
        .map(|part| {
            part.iter()
                .enumerate()
                .fold(0, |n, (i, b)| n | (*b << (7 - i)))
        })
        .collect();
    let archive = Nscripter::nsa()
        .open(memory(nsar(true, &[("repeat.txt", 2, packed, 6)])), &ctx)
        .unwrap();
    assert_eq!(
        read_all(archive.open(0, &ctx).unwrap().as_ref(), 20).unwrap(),
        b"AAAAAA"
    );
    let bad = Nscripter::nsa()
        .open(memory(nsar(true, &[("bad.txt", 2, vec![], 1)])), &ctx)
        .unwrap();
    assert!(bad.open(0, &ctx).is_err());
    let spb = Nscripter::nsa()
        .open(memory(nsar(true, &[("image.bmp", 1, vec![0; 4], 4)])), &ctx)
        .unwrap();
    assert!(spb.open(0, &ctx).err().unwrap().to_string().contains("SPB"));
}
#[test]
fn xp3_versions_zlib_indices_and_multisegment_entries_use_shared_transforms() {
    for version in [1, 2] {
        for compressed in [false, true] {
            let ctx = DecodeContext::new(ClientId(3));
            let archive = Xp3
                .open(memory(xp3(version, compressed, false)), &ctx)
                .unwrap();
            assert_eq!(archive.entries()[0].path, "scene/日本.txt");
            let reader = archive.open(0, &ctx).unwrap();
            assert_eq!(read_all(reader.as_ref(), 100).unwrap(), b"first-second");
            let mut p = [0; 4];
            assert_eq!(reader.read_at(4, &mut p).unwrap(), 4);
            assert_eq!(&p, b"t-se");
            assert_eq!(archive.entries()[0].metadata["adler32"], 0x33445566u32);
        }
    }
}
#[test]
fn encrypted_xp3_requires_client_scoped_profile_and_never_embeds_a_game_key() {
    let mut ctx = DecodeContext::new(ClientId(10));
    let archive = Xp3.open(memory(xp3(1, true, true)), &ctx).unwrap();
    assert!(archive.open(0, &ctx).is_err());
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(ClientId(10), "fixture-key", b"test-only".to_vec())
        .unwrap();
    ctx.keys = keys;
    let mut rules = PipelineRules::default();
    rules.rules.insert(
        ("kirikiri/xp3".into(), None),
        vec![TransformSpec {
            algorithm: "xor".into(),
            key_id: Some("fixture-key".into()),
            parameters: serde_json::Value::Null,
            expected_size: Some(12),
        }],
    );
    ctx.pipelines = Arc::new(rules);
    assert_eq!(
        read_all(archive.open(0, &ctx).unwrap().as_ref(), 100).unwrap(),
        b"first-second"
    );
    let mut other = ctx.clone();
    other.client = ClientId(11);
    assert!(archive.open(0, &other).is_err());
}
#[test]
fn malformed_sizes_versions_paths_and_budget_fail_before_exposing_data() {
    let ctx = DecodeContext::new(ClientId(1));
    let mut data = bgi(false, &[("a", b"x".to_vec())]);
    data[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(BgiArc.open(memory(data), &ctx).is_err());
    let bad = majiro(1, &[("a", vec![1])]);
    assert!(MajiroArc.open(memory(bad), &ctx).is_err());
    let archive = BgiArc
        .open(memory(bgi(false, &[("../escape", vec![1])])), &ctx)
        .unwrap();
    assert!(ArchiveMount::new(archive, ctx.clone()).is_err());
    let mut tiny = ctx.clone();
    tiny.limits.max_index_size = 1;
    assert!(Xp3.open(memory(xp3(1, true, false)), &tiny).is_err());
    tiny = ctx;
    tiny.limits.max_entry_size = 1;
    assert!(BgiArc
        .open(memory(bgi(true, &[("a", vec![1, 2])])), &tiny)
        .is_err());
}
struct Sparse {
    header: Vec<u8>,
    size: u64,
    read: AtomicUsize,
}
impl AssetReader for Sparse {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.size)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let count = out.len().min(self.size.saturating_sub(offset) as usize);
        self.read.fetch_add(count, Ordering::Relaxed);
        for (i, b) in out[..count].iter_mut().enumerate() {
            *b = self
                .header
                .get(offset as usize + i)
                .copied()
                .unwrap_or(b'Z');
        }
        Ok(count)
    }
}
#[test]
fn large_plain_entries_stay_streamed_even_when_resident_budget_is_tiny() {
    let mut header = bgi(false, &[("huge.dat", vec![])]);
    let size = 100 * 1024 * 1024u64;
    header[36..40].copy_from_slice(&(size as u32).to_le_bytes());
    let source = Arc::new(Sparse {
        header,
        size: 48 + size,
        read: AtomicUsize::new(0),
    });
    let mut ctx = DecodeContext::new(ClientId(1));
    ctx.limits.max_entry_size = size;
    ctx.limits.max_resident_bytes = 64;
    let archive = BgiArc.open(source.clone(), &ctx).unwrap();
    let reader = archive.open(0, &ctx).unwrap();
    assert_eq!(source.read.load(Ordering::Relaxed), 48);
    let mut p = [0; 2];
    assert_eq!(reader.read_at(size - 2, &mut p).unwrap(), 2);
    assert_eq!(&p, b"ZZ");
    assert_eq!(ctx.resident_bytes(), 0);
}
#[test]
fn explicit_registration_lists_only_implemented_decoders() {
    let mut registry = FormatRegistry::default();
    crysirror_formats_arc::register(&mut registry).unwrap();
    let mut expected = vec![
        "bgi/arc",
        "kirikiri/xp3",
        "majiro/arc",
        "nscripter/nsa",
        "nscripter/sar",
    ];
    expected.extend(crysirror_formats_arc::legacy_archive_ids());
    expected.sort();
    assert_eq!(registry.names(), expected);
    assert!(crysirror_formats_arc::register(&mut registry).is_err());
}
