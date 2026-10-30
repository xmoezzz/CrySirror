//! Native argument precedence before the first script; self-authored data only.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

#[test]
fn player_arguments_override_embedded_defaults_before_startup() {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("game");
    std::fs::create_dir(&assets).unwrap();
    let mut exe = b"MZ fixture XOPT_EMBED_AREA_".to_vec();
    exe.extend(128u32.to_le_bytes());
    exe.extend(b"debugwin=\"yes\"\nremain=\"embedded\"\0\0");
    std::fs::write(assets.join("game.exe"), exe).unwrap();
    std::fs::write(
        assets.join("startup.tjs"),
        r#"
        if (System.getArgument("-debugwin") !== "no") throw "wrong player override";
        if (System.getArgument("-remain") !== "embedded") throw "lost executable default";
        if (System.getArgument("-label") !== "with spaces") throw "lost argument text";
        if (System.getArgument("-absent") !== void) throw "invented absent argument";
        System.setArgument("-debugwin", "yes");
        if (System.getArgument("-debugwin") !== "yes") throw "native mutation failed";
        Plugins.link("win32dialog.dll");
        var rejected = false;
        try { (new WIN32Dialog()).open(); } catch (e) { rejected = true; }
        if (!rejected) throw "unmapped dialog silently accepted";
        Debug.message("native startup arguments passed");
    "#,
    )
    .unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"arguments",
        "content_revision":"1","assets":"game","canvas":[640,480],
        "clients":[{"name":"game","game":"authored","adapter":"krkr","config":{
            "canvas":[640,480],"executable":"game.exe",
            "arguments":{"-debugwin":"no","-label":"with spaces"}}}]}))
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut rt =
        Runtime::with_asset_root(project, &assets, &temp.path().join("state"), registry).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let info = rt.inspect();
    assert!(info["sessions"][0]["native"]["log_tail"]
        .to_string()
        .contains("native startup arguments passed"));
}

#[test]
fn malformed_and_oversized_native_arguments_are_rejected() {
    for arguments in [
        json!({"debugwin":"no"}),
        json!({"-bad name":"value"}),
        json!({"-bad":"has\0nul"}),
        json!({"-large":"x".repeat(4097)}),
    ] {
        let cfg: crysirror_krkr::Config = serde_json::from_value(json!({
            "canvas":[640,480],"arguments":arguments}))
        .unwrap();
        assert!(cfg.validate().is_err());
    }
    let arguments: serde_json::Map<String, serde_json::Value> =
        (0..65).map(|i| (format!("-a{i}"), json!("v"))).collect();
    let cfg: crysirror_krkr::Config = serde_json::from_value(json!({
        "canvas":[640,480],"arguments":arguments}))
    .unwrap();
    assert!(cfg.validate().is_err());
}
