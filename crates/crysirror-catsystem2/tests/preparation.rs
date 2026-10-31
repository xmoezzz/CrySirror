//! The real Cat worker/Runtime preparation boundary, using authored KCS bytes.
#![cfg(feature = "runtime")]
use anyhow::Result;
use crysirror_core::{
    service::{buttons, HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_runtime::{model::ClientSpec, runtime::VmFactory, AdapterRegistry, Project, Runtime};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

struct Peer(Arc<AtomicU64>);
impl VmFactory for Peer {
    fn create(&self, _: ClientId, _: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        Ok(Box::new(Self(self.0.clone())))
    }
}
impl VmAdapter for Peer {
    fn engine_id(&self) -> &'static str {
        "authored-peer"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        _: &mut dyn HostServices,
        input: InputFrame,
        _: u32,
    ) -> crysirror_core::Result<VmYield> {
        self.0.store(input.buttons_pressed, Ordering::Relaxed);
        Ok(VmYield::Halted)
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(crysirror_core::Error::Unsupported("authored peer snapshot"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::Unsupported("authored peer restore"))
    }
}
#[test]
fn preparing_original_format_does_not_run_bytecode_or_consume_first_frame() -> Result<()> {
    // PUSH7, HALT. This is authored bytecode, not owner game content.
    let code = [0x25, 3, 7, 0, 0, 0, 0x3b];
    let mut h = [0u32; 13];
    h[0] = u32::from_le_bytes(*b"KCS\0");
    h[1] = 2;
    h[3] = 52;
    h[4] = code.len() as u32;
    h[5] = 52 + h[4];
    h[8] = 64;
    h[9] = 1024;
    h[10] = h[5];
    h[12] = h[4];
    let mut bytes = h.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&code)?;
    bytes.extend(z.finish()?);
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("main.kcs"), bytes)?;
    let project: Project = serde_json::from_value(
        serde_json::json!({"schema":2,"name":"authored Cat startup","content_revision":"1","assets":"game","canvas":[4,4],"clients":[{"name":"native","adapter":"catsystem2","game":"native","config":{"script":"main.kcs","font":"default","heap_bytes":4096,"canvas":[4,4]}},{"name":"peer","adapter":"authored-peer","game":"peer"}]}),
    )?;
    let first = Arc::new(AtomicU64::new(0));
    let mut registry = AdapterRegistry::default();
    crysirror_catsystem2::register(&mut registry)?;
    registry.register("authored-peer", Arc::new(Peer(first.clone())))?;
    let mut rt =
        Runtime::with_asset_root(project, root.path(), &root.path().join("state"), registry)?;
    let start = std::time::Instant::now();
    while rt.poll_vm_preparations()? {
        anyhow::ensure!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "Cat preparation worker timeout"
        );
        std::thread::yield_now();
    }
    let prepared = rt.inspect();
    assert_eq!(prepared["sessions"][0]["native"]["instructions"], 0);
    assert_eq!(prepared["sessions"][0]["native"]["pc"], 0);
    assert_eq!(prepared["frame"], 0);
    assert_eq!(prepared["time_ms"], 0);
    assert_eq!(first.load(Ordering::Relaxed), 0);
    assert!(!rt.poll_vm_preparations()?);
    rt.tick(
        16,
        InputFrame {
            buttons_down: buttons::CTRL,
            buttons_pressed: buttons::CTRL,
            ..Default::default()
        },
    )?;
    let started = rt.inspect();
    assert_eq!(started["sessions"][0]["native"]["instructions"], 2);
    assert_eq!(started["sessions"][0]["yield"], "Halted");
    assert_eq!(started["frame"], 1);
    assert_eq!(started["time_ms"], 16);
    assert_eq!(first.load(Ordering::Relaxed), buttons::CTRL);
    Ok(())
}
