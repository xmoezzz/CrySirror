// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! The external game remains an explicit runtime input, never a fixture asset.
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{
    read_all, ArchiveFormat, ArchiveMount, DecodeContext, DependencyResolver,
};
use std::{
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Reader(Mutex<std::fs::File>);
impl AssetReader for Reader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.0
            .lock()
            .unwrap()
            .metadata()
            .map(|v| v.len())
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
    fn read_at(&self, o: u64, b: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start(o))
            .and_then(|_| f.read(b))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
struct Companions(PathBuf);
impl DependencyResolver for Companions {
    fn open(&self, path: &str) -> anyhow::Result<Arc<dyn AssetReader>> {
        let exe = std::env::var("CRYSIRROR_HXV4_EXE").unwrap_or_else(|_| "limelight_lj_trial.exe".into());
        anyhow::ensure!(path == exe, "unexpected companion");
        Ok(Arc::new(Reader(Mutex::new(std::fs::File::open(
            self.0.join(path),
        )?))))
    }
}
#[test]
#[ignore = "requires explicitly authorized CRYSIRROR_HXV4_GAME directory; no game data shipped"]
fn actual_hxv4_authenticates_index_and_decodes_startup_and_samples() {
    let root =
        PathBuf::from(std::env::var_os("CRYSIRROR_HXV4_GAME").expect("explicit game directory"));
    let source = Arc::new(Reader(Mutex::new(
        std::fs::File::open(root.join("data.xp3")).unwrap(),
    )));
    let mut ctx = DecodeContext::new(ClientId(1));
    ctx.source_name = Some("data.xp3".into());
    ctx.resolver = Some(Arc::new(Companions(root)));
    ctx.limits.max_entry_size = 256 * 1024 * 1024;
    ctx.parameters.insert(
        "kirikiri/xp3".into(),
        serde_json::json!({"hxv4_executable":"limelight_lj_trial.exe","hxv4_startup_target":"__bootstrap/startup.tjs"}),
    );
    let archive = crysirror_formats_arc::Xp3.open(source, &ctx).unwrap();
    let startup = archive
        .entries()
        .iter()
        .find(|e| e.path == "startup.tjs")
        .expect("native startup")
        .id;
    let data = read_all(archive.open(startup, &ctx).unwrap().as_ref(), 1024 * 1024).unwrap();
    assert!(
        data.starts_with(b"TJS2100\0") || data.starts_with(b"\xff\xfe"),
        "startup header {:?}",
        &data[..data.len().min(12)]
    );
    let ids: Vec<_> = archive
        .entries()
        .iter()
        .filter(|e| e.size < 2 * 1024 * 1024 && e.metadata.get("hxv4").is_some())
        .step_by(127)
        .take(32)
        .map(|e| e.id)
        .collect();
    for id in &ids {
        let _ = read_all(archive.open(*id, &ctx).unwrap().as_ref(), 2 * 1024 * 1024).unwrap();
    }
    let reader = archive.open(startup, &ctx).unwrap();
    let mut offset = [0; 17];
    assert_eq!(reader.read_at(23, &mut offset).unwrap(), 17);
    assert_eq!(&offset, &data[23..40]);
    let embedded=archive.entries().iter().find(|e|e.path=="__bootstrap/startup.tjs").expect("embedded bootstrap");
    assert!(embedded.metadata["derived"].as_bool().unwrap());
    let actual_boot=read_all(archive.open(embedded.id,&ctx).unwrap().as_ref(),1024*1024).unwrap();
    assert!(actual_boot.starts_with(b"TJS2100\0"));
    assert_ne!(actual_boot,data,"embedded native startup must remain distinct from archive sentinel");
    let largest=archive.entries().iter().max_by_key(|e|e.size).unwrap();
    let reader=archive.open(largest.id,&ctx).expect("largest entry must verify without a second full buffer");
    let mut tail=[0;32];assert_eq!(reader.read_at(largest.size-32,&mut tail).unwrap(),32);
    println!("largest entry verified {} bytes; retained decoded budget {} bytes",largest.size,ctx.resident_bytes());
    drop(reader);
    let count = archive.entries().len();
    let mount = ArchiveMount::new(archive, ctx).unwrap();
    let again = mount.open_entry("startup.tjs").unwrap();
    assert_eq!(read_all(again.as_ref(), 1024 * 1024).unwrap(), data);
    println!("authenticated catalog {count} entries; startup {} bytes; {} protected samples passed Adler",data.len(),ids.len());
}
#[test]
#[ignore = "requires explicitly authorized CRYSIRROR_HXV4_GAME directory; no game data shipped"]
fn actual_hxv4_hashed_root_resolves_requested_names_only() {
    let root =
        PathBuf::from(std::env::var_os("CRYSIRROR_HXV4_GAME").expect("explicit game directory"));
    let source = Arc::new(Reader(Mutex::new(
        std::fs::File::open(root.join("data.xp3")).unwrap(),
    )));
    let mut ctx = DecodeContext::new(ClientId(1));
    ctx.source_name = Some("data.xp3".into());
    ctx.resolver = Some(Arc::new(Companions(root)));
    ctx.limits.max_entry_size = 256 * 1024 * 1024;
    ctx.parameters.insert(
        "kirikiri/xp3".into(),
        serde_json::json!({"hxv4_executable":"limelight_lj_trial.exe",
            "hxv4_bootstrap_profile":"__bootstrap/protection.json","hxv4_hashed_root":"__hxv4"}),
    );
    let archive = crysirror_formats_arc::Xp3.open(source, &ctx).unwrap();
    let plain = archive.entries().iter().find(|e| e.path == "startup.tjs").unwrap().id;
    let hashed = archive.resolve("__hxv4/startup.tjs").expect("hashed startup");
    assert_ne!(plain, hashed, "hashed lookup must not return the plain sentinel");
    assert_eq!(archive.resolve("startup.tjs"), None);
    // Directory hashes use the trailing-slash spelling.
    assert!(archive.resolve("__hxv4/system/Initialize.tjs").is_some() || archive.resolve("__hxv4/system/initialize.tjs").is_some());
    let data = read_all(archive.open(hashed, &ctx).unwrap().as_ref(), 4 * 1024 * 1024).unwrap();
    assert!(data.starts_with(b"TJS2100\0") || data.starts_with(b"\xff\xfe"));
    let profile = archive.entries().iter().find(|e| e.path == "__bootstrap/protection.json").unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&read_all(archive.open(profile.id, &ctx).unwrap().as_ref(), 65536).unwrap()).unwrap();
    assert_eq!(json["family"], "hxv4-bootstrap");
    assert_eq!(json["hashed_root"], "__hxv4");
}
#[test]
#[ignore = "diagnostic dump: CRYSIRROR_HXV4_GAME, CRYSIRROR_HXV4_EXE, CRYSIRROR_HXV4_DUMP=dir, CRYSIRROR_HXV4_NAMES=a,b"]
fn dump_named_hxv4_entries() {
    let root = PathBuf::from(std::env::var_os("CRYSIRROR_HXV4_GAME").unwrap());
    let out = PathBuf::from(std::env::var_os("CRYSIRROR_HXV4_DUMP").unwrap());
    let source = Arc::new(Reader(Mutex::new(std::fs::File::open(root.join("data.xp3")).unwrap())));
    let mut ctx = DecodeContext::new(ClientId(1));
    ctx.source_name = Some("data.xp3".into());
    ctx.resolver = Some(Arc::new(Companions(root)));
    ctx.limits.max_entry_size = 256 * 1024 * 1024;
    ctx.parameters.insert("kirikiri/xp3".into(),
        serde_json::json!({"hxv4_executable":std::env::var("CRYSIRROR_HXV4_EXE").unwrap_or_else(|_| "limelight_lj_trial.exe".into()),"hxv4_hashed_root":"__hxv4"}));
    let archive = crysirror_formats_arc::Xp3.open(source, &ctx).unwrap();
    for name in std::env::var("CRYSIRROR_HXV4_NAMES").unwrap().split(',') {
        match archive.resolve(&format!("__hxv4/{name}")) {
            Some(id) => {
                let data = read_all(archive.open(id, &ctx).unwrap().as_ref(), 64 << 20).unwrap();
                let path = out.join(name.replace('/', "__"));
                std::fs::write(&path, data).unwrap();
                println!("DUMP {name} -> {}", path.display());
            }
            None => println!("MISS {name}"),
        }
    }
}
