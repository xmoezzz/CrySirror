// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
fn runtime() -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&serde_json::json!({"schema":2,"name":"dag","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"game","adapter":"crysirror-script-v1","game":"dag","systems":{"entities":[{"id":1,"name":"moving","geometry":{"kind":"cube"},"velocity":[1,0,0]}]}}]})).unwrap()).unwrap();
    let r = Runtime::load(&p, &t.path().join("state")).unwrap();
    (t, r)
}
struct Release(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
#[test]
fn delayed_extraction_does_not_block_simulation_or_observe_later_mutations() {
    let (_t, mut rt) = runtime();
    let pool = rt.assets.cpu_tasks();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Release(gate.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let jobs: Vec<_> = (0..pool.stats().workers)
        .map(|_| {
            let g = gate.clone();
            let tx = tx.clone();
            pool.spawn(move |_| {
                tx.send(()).unwrap();
                let mut ready = g.0.lock().unwrap();
                while !*ready {
                    ready = g.1.wait(ready).unwrap();
                }
                Ok(())
            })
            .unwrap()
        })
        .collect();
    for _ in 0..jobs.len() {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    rt.assets
        .put_surface(ClientId(1), 1, 1, 1, vec![255, 0, 0, 255])
        .unwrap();
    let mut pending = rt.schedule_render_snapshot().unwrap();
    rt.assets
        .put_surface(ClientId(1), 1, 1, 1, vec![0, 255, 0, 255])
        .unwrap();
    assert!(pending.poll(&rt).unwrap().is_none());
    for _ in 0..10 {
        rt.tick(16, InputFrame::default()).unwrap();
    }
    assert_eq!(rt.now_ms(), 160);
    drop(release);
    for job in jobs {
        job.wait().unwrap();
    }
    let old = pending.wait(&rt).unwrap();
    assert_eq!(old.clock_ms, 0);
    let result: anyhow::Result<()> = rt.assets.with_frame_surfaces(old.surfaces, |assets| {
        assert_eq!(
            assets
                .surface(ClientId(1), "__surface/1")
                .unwrap()
                .rgba
                .as_ref(),
            &[255, 0, 0, 255]
        );
        anyhow::bail!("fixture")
    });
    assert!(result.is_err());
    assert_eq!(
        rt.assets
            .surface(ClientId(1), "__surface/1")
            .unwrap()
            .rgba
            .as_ref(),
        &[0, 255, 0, 255]
    );
    let new = rt.schedule_render_snapshot().unwrap().wait(&rt).unwrap();
    assert_ne!(
        serde_json::to_value(old.simple).unwrap(),
        serde_json::to_value(new.simple).unwrap()
    );
    assert!(
        rt.profiler.chrome_trace()["traceEvents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "extract.ready")
    );
}
#[test]
fn restored_world_rejects_a_pre_restore_snapshot() {
    let (_t, mut rt) = runtime();
    let save = rt.snapshot().unwrap();
    let pending = rt.schedule_render_snapshot().unwrap();
    rt.tick(16, InputFrame::default()).unwrap();
    rt.restore(&save).unwrap();
    assert!(pending.wait(&rt).is_err());
    assert!(rt.host.engine.world.handle(ClientId(1), 1).is_ok());
}
