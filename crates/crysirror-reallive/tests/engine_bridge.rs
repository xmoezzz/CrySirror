// SPDX-License-Identifier: MPL-2.0
//! Native bytecode -> public engine services. No original engine, fonts or GPU.
use crysirror_core::{
    service::{HostServices, InputFrame, buttons},
    state::ClientId,
};
use crysirror_reallive::vm::{
    archive,
    bytecode::{Opcode, asm},
    expr::encode,
    scenario,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
fn command(module: u8, opcode: u16, args: &[Vec<u8>]) -> Vec<u8> {
    asm::command(Opcode::new(1, module, opcode, 0), args.len() as u16, args)
}
fn fixture(code: Vec<u8>) -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    std::fs::write(
        t.path().join("assets/Gameexe.ini"),
        "#SEEN_START=1\n#REGNAME=\"synthetic\"\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("assets/Seen.txt"),
        archive::build(&[(1, scenario::build(&code, &[1_000_000], &[]))]),
    )
    .unwrap();
    let p:crysirror_runtime::Project=serde_json::from_value(json!({"schema":2,"name":"bridge test","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"reallive","game":"synthetic","config":{}}]})).unwrap();
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_reallive::register(&mut registry).unwrap();
    let r = Runtime::with_registry(
        p,
        &t.path().join("project.json"),
        &t.path().join("state"),
        registry,
    )
    .unwrap();
    (t, r)
}
#[test]
fn native_draw_and_wait_produce_engine_owned_nodes_editable_without_vm() {
    let mut code = command(4, 0, &[encode::quoted(b"native engine title")]);
    code.extend(command(
        33,
        31,
        &[
            encode::int(0),
            encode::int(12),
            encode::int(34),
            encode::int(56),
        ],
    ));
    code.extend(command(4, 100, &[encode::int(1000)]));
    code.extend(command(4, 0, &[encode::quoted(b"after wait")]));
    let (_t, mut r) = fixture(code);
    r.tick(0, InputFrame::default()).unwrap();
    assert_eq!(
        r.host.engine.ui.title.as_ref().unwrap().1,
        "native engine title"
    );
    let surface = r.host.engine.canvases.surface(ClientId(1), 0).unwrap();
    assert_eq!(surface.nodes.len(), 1);
    let before = r.inspect()["sessions"][0]["native"]["instructions"].clone();
    r.tick(999, InputFrame::default()).unwrap();
    assert_eq!(before, r.inspect()["sessions"][0]["native"]["instructions"]);
    // Public service call changes the live retained drawing while native PC waits.
    r.host
        .submit_engine(
            ClientId(1),
            1,
            &serde_json::to_vec(&json!({"canvases":[{"op":"clear","id":0,"color":[255,0,0,255]}]}))
                .unwrap(),
        )
        .unwrap();
    let value = serde_json::to_value(r.host.engine.canvases.surface(ClientId(1), 0)).unwrap();
    assert_eq!(
        value["nodes"][0]["layer"]["paint"]["color"],
        json!([255, 0, 0, 255])
    );
    assert_eq!(before, r.inspect()["sessions"][0]["native"]["instructions"]);
    r.tick(1, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1, "after wait");
    assert_eq!(
        r.inspect()["sessions"][0]["native"]["owns_framebuffer"],
        false
    );
    assert!(
        r.host
            .query_engine(
                ClientId(999),
                crysirror_core::service::EngineQuery::Canvas(0)
            )
            .is_err()
    );
}
#[test]
fn unsupported_operation_fails_at_original_pc_and_never_acknowledges() {
    let code = command(99, 60000, &[]);
    let (_t, mut r) = fixture(code);
    let error = format!("{:#}", r.tick(0, InputFrame::default()).unwrap_err());
    assert!(error.contains("unsupported native syscall"), "{error}");
    let before = r.inspect()["sessions"][0]["native"]["instructions"].clone();
    assert!(r.tick(16, InputFrame::default()).is_err());
    assert_eq!(before, r.inspect()["sessions"][0]["native"]["instructions"]);
}
#[test]
fn native_name_dialog_reads_shared_text_field_and_resumes_only_on_submit() {
    let mut code = command(4, 1302, &[encode::int(0), encode::quoted(b"Name")]);
    code.extend(command(4, 0, &[encode::quoted(b"submitted")]));
    let (_t, mut r) = fixture(code);
    r.tick(0, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.get(1, 610000).unwrap().text, "");
    r.host
        .submit_engine(
            ClientId(1),
            1,
            &serde_json::to_vec(&json!({"ui":[{"op":"insert_text","text":"Alice"}]})).unwrap(),
        )
        .unwrap();
    r.tick(
        16,
        InputFrame {
            buttons_pressed: buttons::ENTER,
            ..Default::default()
        },
    )
    .unwrap();
    r.tick(16, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1, "submitted");
    assert!(r.host.engine.ui.get(1, 610000).is_none());
}
