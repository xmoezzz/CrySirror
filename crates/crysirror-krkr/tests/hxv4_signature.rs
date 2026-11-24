// SPDX-License-Identifier: MPL-2.0
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

// Ephemeral test key and a signature of the literal "synthetic test message".
const PUBLIC: &str = "MIGJAoGBANYi8aHLfs4YZzdzR0JMtoQrY3oS9Gl9L1AtbhPdqlG0aSZltfZi7FYBgTH0Xa60SlJyU17c68GlmDmX0B5C88h6L0SR1DtSdL3ZPXI7UHJxY+6zfq1F00YUt7eS961q5cXmQN7edZySXjy9aK9QhgYqg+Tx39vdcLECyWYHxGyNAgMBAAE=";
const SIGNATURE: &str = "bk/B185+voQZbPCmbHMz0Zks60j7Yds+J3fwnVY+bguxbalej/YpdjD/yWA6pmRZPk4JNjh+OYBok/yODjaWjThRQ+JQw2I+uQ4BfovMZueKLXokXtDI9kp8WGWNaqp3c4Yd3c9bV9qsE6lZHUROYPZ09jmbF3t9WKFqvagWwKc=";

#[test]
fn signature_failures_return_false_to_the_calling_script() {
    let dir = tempfile::tempdir().unwrap();
    let assets = dir.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    let profile = json!({
        "family":"hxv4-bootstrap", "executable":"fixture.exe",
        "bootstrap_storage":"bres://./test/bootstrap", "bootstrap_prefix":"test",
        "archive_unique_key":"fixture", "media":"arc", "context_token":"12".repeat(32),
        "signature_public_key":format!("-----BEGIN RSA PUBLIC KEY-----\n{PUBLIC}\n-----END RSA PUBLIC KEY-----")
    });
    std::fs::write(assets.join("profile.json"), profile.to_string()).unwrap();
    std::fs::write(assets.join("signed.bin"), "synthetic test message").unwrap();
    std::fs::write(assets.join("other.bin"), "different message").unwrap();
    std::fs::write(
        assets.join("signed.bin.sig"),
        format!("-- SIGNATURE - SHA256/PSS/RSA --\n{SIGNATURE}"),
    )
    .unwrap();
    std::fs::write(assets.join("malformed.sig"), "invalid signature").unwrap();
    std::fs::write(assets.join("startup.tjs"), r#"
        Plugins.linkZ("bres://./test/bootstrap");
        if (!System.checkSignature("signed.bin")) throw new Exception("valid signature rejected");
        if (!System.checkSignature("signed.bin", "")) throw new Exception("invalid default path");
        if (System.checkSignature("other.bin", "signed.bin.sig")) throw new Exception("wrong message accepted");
        if (System.checkSignature("other.bin")) throw new Exception("missing sidecar accepted");
        if (System.checkSignature("missing.bin", "signed.bin.sig")) throw new Exception("missing target accepted");
        if (System.checkSignature("signed.bin", "malformed.sig")) throw new Exception("invalid wrapper accepted");
        Debug.message("signature-contract-ok");
    "#).unwrap();
    let project = serde_json::from_value(json!({
        "schema":2, "name":"signature contract", "content_revision":"1", "assets":"assets",
        "canvas":[32,32], "clients":[{"name":"native", "adapter":"krkr", "game":"signature",
        "config":{"canvas":[32,32], "bootstrap_profile":"profile.json"}}]
    }))
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut runtime = Runtime::with_registry(
        project,
        &path,
        &dir.path().join("state"),
        registry,
    )
    .unwrap();
    runtime.tick(0, InputFrame::default()).unwrap();
    let log = &runtime.inspect()["sessions"][0]["native"]["log_tail"];
    assert!(log.to_string().contains("signature-contract-ok"), "{log}");
}
