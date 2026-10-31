//! Self-authored inline alternate-name records; no installed game bytes.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_formats_arc::Xp3;
use crysirror_resource::{read_all, ArchiveFormat, DecodeContext};
use std::sync::Arc;
fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = tag.to_vec();
    out.extend((body.len() as u64).to_le_bytes());
    out.extend(body);
    out
}
fn archive(
    tag: &[u8; 4],
    name: &str,
    raw: &str,
    hash: u32,
    tail: &[u8],
    dangling: bool,
) -> Vec<u8> {
    let mut out = b"XP3\r\n \n\x1a\x8bg\x01".to_vec();
    out.extend(0u64.to_le_bytes());
    let at = out.len() as u64;
    out.extend(b"payload");
    let index = out.len() as u64;
    out[11..19].copy_from_slice(&index.to_le_bytes());
    let mut info = 0u32.to_le_bytes().to_vec();
    info.extend(7u64.to_le_bytes());
    info.extend(7u64.to_le_bytes());
    info.extend((raw.encode_utf16().count() as u16).to_le_bytes());
    info.extend(raw.encode_utf16().flat_map(u16::to_le_bytes));
    let mut seg = 0u32.to_le_bytes().to_vec();
    seg.extend(at.to_le_bytes());
    seg.extend(7u64.to_le_bytes());
    seg.extend(7u64.to_le_bytes());
    let mut file = chunk(b"info", &info);
    file.extend(chunk(b"segm", &seg));
    file.extend(chunk(b"adlr", &0x11223344u32.to_le_bytes()));
    let mut inline = hash.to_le_bytes().to_vec();
    inline.extend((name.encode_utf16().count() as u16).to_le_bytes());
    inline.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
    inline.extend(tail);
    let mut table = chunk(tag, &inline);
    if !dangling {
        table.extend(chunk(b"File", &file));
    }
    out.push(0);
    out.extend((table.len() as u64).to_le_bytes());
    out.extend(table);
    out
}
fn open(bytes: Vec<u8>) -> anyhow::Result<Box<dyn crysirror_resource::Archive>> {
    Xp3.open(
        Arc::new(MemoryAssetReader(bytes.into())) as Arc<dyn AssetReader>,
        &DecodeContext::new(ClientId(1)),
    )
}
#[test]
fn dynamic_inline_name_tags_accept_exact_or_nul_and_validate_next_file() {
    for tag in [b"Yuzu", b"M2X!"] {
        for tail in [&[][..], &[0, 0][..]] {
            let a = open(archive(
                tag,
                "scene/Test.tjs",
                "5b4d4c00648c6b38ad46cebe6039fbed",
                0x11223344,
                tail,
                false,
            ))
            .unwrap();
            assert_eq!(a.entries()[0].path, "scene/Test.tjs");
            assert_eq!(
                read_all(
                    a.open(0, &DecodeContext::new(ClientId(1)))
                        .unwrap()
                        .as_ref(),
                    100
                )
                .unwrap(),
                b"payload"
            );
        }
    }
    let a = open(archive(b"free", "a.txt", "a.txt", 0x11223344, &[], false)).unwrap();
    assert_eq!(a.entries()[0].path, "a.txt");
}
#[test]
fn malformed_mismatched_and_dangling_inline_names_fail_visibly() {
    for b in [
        archive(b"Yuzu", "a.txt", "a.txt", 0, &[], false),
        archive(
            b"Yuzu",
            "a.txt",
            "0123456789abcdef0123456789abcdef",
            0x11223344,
            &[],
            false,
        ),
        archive(b"Yuzu", "a.txt", "a.txt", 0x11223344, &[1, 0], false),
        archive(b"Yuzu", "a.txt", "a.txt", 0x11223344, &[], true),
        archive(b"Yuzu", "../outside", "../outside", 0x11223344, &[], false),
    ] {
        assert!(open(b).is_err());
    }
}

#[test]
#[ignore = "reads explicitly selected owner archive/modules; no original executable is run"]
fn owner_inline_data_archive_profiles() {
    use std::{
        io::{Read, Seek, SeekFrom},
        path::{Path, PathBuf},
        sync::Mutex,
    };
    struct Reader(Mutex<std::fs::File>);
    impl AssetReader for Reader {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .metadata()
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))?
                .len())
        }
        fn read_at(&self, p: u64, b: &mut [u8]) -> crysirror_core::Result<usize> {
            let mut f = self.0.lock().unwrap();
            f.seek(SeekFrom::Start(p))
                .and_then(|_| f.read(b))
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
    }
    struct Siblings(PathBuf);
    impl crysirror_resource::DependencyResolver for Siblings {
        fn open(&self, p: &str) -> anyhow::Result<Arc<dyn AssetReader>> {
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, p)?;
            Ok(Arc::new(Reader(Mutex::new(std::fs::File::open(
                self.0.join(p),
            )?))))
        }
    }
    let root = PathBuf::from(std::env::var_os("CRYSIRROR_KRKR_OWNER_ROOT").expect("owner root"));
    let mut cx = DecodeContext::new(ClientId(1));
    cx.source_name = Some("data.xp3".into());
    cx.resolver = Some(Arc::new(Siblings(root.clone())));
    cx.parameters.insert(
        "kirikiri/xp3".into(),
        serde_json::json!({"hash_fold_xor":{}}),
    );
    let a = Xp3
        .open(
            Arc::new(Reader(Mutex::new(
                std::fs::File::open(Path::new(&root).join("data.xp3")).unwrap(),
            ))),
            &cx,
        )
        .unwrap();
    assert!(a
        .entries()
        .iter()
        .any(|e| e.path.eq_ignore_ascii_case("startup.tjs")));
    let profile = a
        .entries()
        .iter()
        .find_map(|e| e.metadata.get("hash_fold_xor"))
        .unwrap()
        .clone();
    println!("entries={} profile={profile}", a.entries().len());
    let startup = a
        .entries()
        .iter()
        .find(|e| e.path == "startup.tjs")
        .unwrap();
    assert_eq!(
        startup.metadata["xp3_shadowed_native_ids"],
        serde_json::json!([1])
    );
    assert_ne!(startup.id, 1);
    let native_info = startup.metadata["xp3_info_name"].as_str().unwrap();
    assert_eq!(a.resolve(native_info), Some(startup.id));
    assert!(read_all(a.open(1, &cx).unwrap().as_ref(), 1024)
        .unwrap()
        .starts_with(b"TJS2"));

    for (name, magic) in [
        ("startup.tjs", &b"//"[..]),
        ("AppConfig.tjs", &b"//"[..]),
        ("image/attention.png", &b"\x89PNG\r\n\x1a\n"[..]),
        ("image/bar1.png", &b"\x89PNG\r\n\x1a\n"[..]),
    ] {
        let entry = a
            .entries()
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case(name))
            .unwrap();
        let bytes = read_all(a.open(entry.id, &cx).unwrap().as_ref(), 16 * 1024 * 1024).unwrap();
        assert!(bytes.starts_with(magic), "unexpected decoded {name}");
        println!("verified {name} bytes={}", bytes.len());
    }
    cx.limits.max_entry_size = 256 * 1024 * 1024;
    for filename in [
        "bgimage.xp3",
        "bgm.xp3",
        "data.xp3",
        "evimage.xp3",
        "fgimage.xp3",
        "scn.xp3",
        "video.xp3",
        "voice.xp3",
    ] {
        cx.source_name = Some(filename.into());
        cx.parameters.insert(
            "kirikiri/xp3".into(),
            serde_json::json!({"hash_fold_xor":profile}),
        );
        let archive = Xp3
            .open(
                Arc::new(Reader(Mutex::new(
                    std::fs::File::open(root.join(filename)).unwrap(),
                ))),
                &cx,
            )
            .unwrap();
        let protected: Vec<_> = archive
            .entries()
            .iter()
            .filter(|e| e.metadata.get("hash_fold_xor").is_some())
            .collect();
        let mut ids = std::collections::BTreeSet::new();
        ids.insert(protected.iter().min_by_key(|e| e.size).unwrap().id);
        ids.insert(protected.iter().max_by_key(|e| e.size).unwrap().id);
        if let Some(entry) = protected.iter().find(|e| {
            let h = e.metadata["adler32"].as_u64().unwrap() as u32;
            ((h ^ (h >> 8) ^ (h >> 16) ^ (h >> 24)) as u8)
                == profile["key_bias"].as_u64().unwrap() as u8
        }) {
            ids.insert(entry.id);
        }
        let mut verified = 0u64;
        for id in &ids {
            let entry = archive.entries().iter().find(|e| e.id == *id).unwrap();
            let reader = archive.open(*id, &cx).unwrap();
            assert_eq!(reader.len().unwrap(), entry.size);
            verified += entry.size;
            drop(reader);
        }
        println!(
            "archive={filename} protected={} verified={} bytes={verified}",
            protected.len(),
            ids.len()
        );
        assert_eq!(cx.resident_bytes(), 0);
    }
    assert_eq!(cx.resident_bytes(), 0);
}
