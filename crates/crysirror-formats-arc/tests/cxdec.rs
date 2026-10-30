// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Opt-in check against an owner-authorized local CXDEC game. The game stays
//! an explicit runtime input (`CRYSIRROR_CXDEC_GAME`, `CRYSIRROR_CXDEC_MODULES`
//! comma-separated companion names); no game bytes or parameters are shipped.
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{read_all, ArchiveFormat, DecodeContext, DependencyResolver};
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
        anyhow::ensure!(
            !path.split('/').any(|s| s == ".."),
            "companion escapes root"
        );
        Ok(Arc::new(Reader(Mutex::new(std::fs::File::open(
            self.0.join(path),
        )?))))
    }
}
#[test]
#[ignore = "requires explicitly authorized CRYSIRROR_CXDEC_GAME directory; no game data shipped"]
fn actual_cxdec_archives_open_and_sample_entries_verify() {
    let root = PathBuf::from(std::env::var_os("CRYSIRROR_CXDEC_GAME").expect("game directory"));
    let modules: Vec<String> = std::env::var("CRYSIRROR_CXDEC_MODULES")
        .expect("companion module names")
        .split(',')
        .map(str::to_owned)
        .collect();
    let stride: usize = std::env::var("CRYSIRROR_CXDEC_STRIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(97);
    let mut archives: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.to_ascii_lowercase().ends_with(".xp3"))
        .collect();
    archives.sort();
    let mut failures = 0;
    for name in archives {
        let mut ctx = DecodeContext::new(ClientId(1));
        ctx.source_name = Some(name.clone());
        ctx.resolver = Some(Arc::new(Companions(root.clone())));
        ctx.limits.max_entry_size = 1024 * 1024 * 1024;
        ctx.limits.max_index_size = 256 * 1024 * 1024;
        ctx.limits.max_entries = 1 << 20;
        ctx.parameters.insert(
            "kirikiri/xp3".into(),
            serde_json::json!({"cxdec_modules": modules}),
        );
        let source = Arc::new(Reader(Mutex::new(
            std::fs::File::open(root.join(&name)).unwrap(),
        )));
        let started = std::time::Instant::now();
        let archive = match crysirror_formats_arc::Xp3.open(source, &ctx) {
            Ok(a) => a,
            Err(e) => {
                println!("{name}: OPEN FAILED: {e:#}");
                failures += 1;
                continue;
            }
        };
        let opened = started.elapsed();
        let entries = archive.entries();
        let protected = entries
            .iter()
            .filter(|e| e.metadata.get("cxdec").is_some())
            .count();
        let (mut ok, mut bad, mut bytes) = (0, 0, 0u64);
        for e in entries.iter().step_by(stride.max(1)) {
            match archive
                .open(e.id, &ctx)
                .and_then(|r| read_all(r.as_ref(), 1024 * 1024 * 1024))
            {
                Ok(d) => {
                    ok += 1;
                    bytes += d.len() as u64;
                }
                Err(err) => {
                    bad += 1;
                    println!("{name}: entry {} {:?}: {err:#}", e.id, e.path);
                }
            }
        }
        // Opt-in local diagnostic: write entries whose names end with one of
        // CRYSIRROR_CXDEC_DUMP_EXT under CRYSIRROR_CXDEC_DUMP/<archive>/.
        if let Some(dir) = std::env::var_os("CRYSIRROR_CXDEC_DUMP") {
            let exts: Vec<String> = std::env::var("CRYSIRROR_CXDEC_DUMP_EXT")
                .unwrap_or_else(|_| ".tjs,.ks".into())
                .split(',')
                .map(|x| x.to_ascii_lowercase())
                .collect();
            for e in entries.iter() {
                let lower = e.path.to_ascii_lowercase();
                if !exts.iter().any(|x| lower.ends_with(x.as_str())) || e.path.contains("..") {
                    continue;
                }
                let d = read_all(archive.open(e.id, &ctx).unwrap().as_ref(), 256 << 20).unwrap();
                let out = PathBuf::from(&dir).join(&name).join(&e.path);
                std::fs::create_dir_all(out.parent().unwrap()).unwrap();
                std::fs::write(out, d).unwrap();
            }
        }
        if let Some(s) = entries
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case("startup.tjs"))
        {
            let d = read_all(archive.open(s.id, &ctx).unwrap().as_ref(), 16 << 20).unwrap();
            println!(
                "{name}: startup.tjs {} bytes head={:?}",
                d.len(),
                &d[..d.len().min(8)]
            );
        }
        let report = entries
            .iter()
            .find_map(|e| e.metadata.get("cxdec"))
            .cloned();
        println!(
            "{name}: entries={} protected={protected} sampled_ok={ok} sampled_failed={bad} bytes={bytes} open={:.2}s cxdec={}",
            entries.len(),
            opened.as_secs_f64(),
            report.map(|r| r.to_string()).unwrap_or_default()
        );
        failures += bad;
    }
    assert_eq!(failures, 0);
}
