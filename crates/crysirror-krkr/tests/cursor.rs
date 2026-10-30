// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `Layer.cursor = "file.cur"` loads the cursor through storage, returns a
//! krkrz MouseCursor index and defines it in the shared UI cursor service.
//! The cursor bytes are synthesized here; no game data.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

/// 2x2 1-bpp `.cur` with hotspot (1,0).
fn tiny_cur() -> Vec<u8> {
    let mut dib = Vec::new();
    for v in [40u32, 2, 4] {
        dib.extend(v.to_le_bytes());
    }
    dib.extend(1u16.to_le_bytes());
    dib.extend(1u16.to_le_bytes());
    dib.extend([0u8; 24]);
    dib.extend([0, 0, 0, 0, 255, 255, 255, 0]);
    dib.extend([0b1000_0000, 0, 0, 0, 0b1100_0000, 0, 0, 0]);
    dib.extend([0, 0, 0, 0, 0b1000_0000, 0, 0, 0]);
    let mut f = vec![0, 0, 2, 0, 1, 0, 2, 2, 0, 0, 1, 0, 0, 0];
    f.extend((dib.len() as u32).to_le_bytes());
    f.extend(22u32.to_le_bytes());
    f.extend(dib);
    f
}

#[test]
fn layer_cursor_file_becomes_a_shared_custom_cursor() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    std::fs::write(d.path().join("assets/hand.cur"), tiny_cur()).unwrap();
    std::fs::write(
        d.path().join("assets/startup.tjs"),
        r#"
var w = new Window();
var root = new Layer(w, null);
root.cursor = "hand.cur";
var again = new Layer(w, root);
again.cursor = "hand.cur";
Debug.message("cursor " + root.cursor + " " + again.cursor);
root.cursor = crHandPoint;
Debug.message("standard " + root.cursor);
"#,
    )
    .unwrap();
    let p = serde_json::from_value(json!({"schema":2,"name":"cursor","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-cursor","config":{"canvas":[32,32]}}]})).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut a = AdapterRegistry::default();
    crysirror_krkr::register(&mut a).unwrap();
    let mut r = Runtime::with_registry(p, &path, &d.path().join("state"), a).unwrap();
    let result = r.tick(1, InputFrame::default());
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(result.is_ok(), "{result:?} {log}");
    assert!(log.contains("cursor 2 2"), "{log}");
    assert!(log.contains("standard -21"), "{log}");
    assert_eq!(r.host.engine.ui.cursors.revision, 1, "one definition for one file");
}
