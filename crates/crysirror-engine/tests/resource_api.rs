// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "arc-formats")]
use crysirror_core::{
    path::{Mount, VfsPath},
    service::FileSystem,
};
use crysirror_engine::{App, Value};
use crysirror_resource::MemoryKeyProvider;
use crysirror_runtime::Runtime;
use serde_json::json;
use std::sync::Arc;
fn sar(name: &str, bytes: &[u8]) -> Vec<u8> {
    let start = 6 + name.len() + 1 + 8;
    let mut out = 1u16.to_be_bytes().to_vec();
    out.extend((start as u32).to_be_bytes());
    out.extend(name.as_bytes());
    out.push(0);
    out.extend(0u32.to_be_bytes());
    out.extend((bytes.len() as u32).to_be_bytes());
    out.extend(bytes);
    out
}
fn app(root: &std::path::Path) -> App {
    std::fs::create_dir(root.join("assets")).unwrap();
    std::fs::write(root.join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"resources","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    App::new(
        Runtime::load(&root.join("project.json"), &root.join("state")).unwrap(),
        "a",
    )
    .unwrap()
}
fn call(
    app: &mut App,
    class: &'static str,
    method: &str,
    args: serde_json::Value,
) -> anyhow::Result<Value> {
    let cx = app.context();
    let obj = cx.singleton(class)?;
    let args = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect::<Vec<_>>();
    cx.call(obj, method, &args)
}
#[test]
fn archive_mount_drives_shared_bitmap_assets_and_capability_storage() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let image = crysirror_image::ImageSurface {
        width: 2,
        height: 1,
        rgba: vec![12, 34, 56, 255, 78, 90, 12, 255],
    };
    let png = crysirror_image::encode_png(&image).unwrap();
    std::fs::write(
        tmp.path().join("assets/picture.sar"),
        sar("image/pixel.png", &png),
    )
    .unwrap();
    let listed = call(
        &mut app,
        "Resources",
        "list",
        json!(["picture.sar",{"format":"nscripter/sar"}]),
    )
    .unwrap()
    .to_json();
    assert_eq!(listed["entries"][0]["path"], "image/pixel.png");
    assert_eq!(listed["total"], 1);
    call(
        &mut app,
        "Resources",
        "mount",
        json!(["picture.sar","pack",{"format":"nscripter/sar"}]),
    )
    .unwrap();
    call(
        &mut app,
        "Bitmap",
        "load",
        json!([3, "pack/image/pixel.png"]),
    )
    .unwrap();
    let runtime = app.runtime_mut().unwrap();
    let a = runtime.client_id("a").unwrap();
    let b = runtime.client_id("b").unwrap();
    assert_eq!(
        runtime
            .assets
            .surface(a, "__surface/3")
            .unwrap()
            .rgba
            .as_ref(),
        image.rgba
    );
    assert_eq!(
        runtime
            .assets
            .read_for(a, "pack/image/pixel.png")
            .unwrap()
            .as_ref(),
        png
    );
    assert!(runtime.assets.read_for(b, "pack/image/pixel.png").is_err());
    let path = VfsPath::new(Mount::Asset, "pack/image/pixel.png").unwrap();
    assert_eq!(
        crysirror_resource::read_all(
            runtime
                .host
                .fs
                .storage(a)
                .unwrap()
                .open_asset(&path)
                .unwrap()
                .as_ref(),
            1024
        )
        .unwrap(),
        png
    );
    assert!(runtime.host.fs.read(b, &path).is_err());
    assert!(call(
        &mut app,
        "Resources",
        "mount",
        json!(["picture.sar","../outside",{"format":"nscripter/sar"}])
    )
    .is_err());
    assert!(call(
        &mut app,
        "Resources",
        "list",
        json!(["picture.sar",{"format":"invented"}])
    )
    .is_err());
}
#[test]
fn scoped_host_key_profile_decrypts_through_the_existing_fs_api() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let encrypted = b"archive text".iter().map(|b| b ^ 0x5a).collect::<Vec<_>>();
    std::fs::write(
        tmp.path().join("assets/text.sar"),
        sar("text.txt", &encrypted),
    )
    .unwrap();
    let client = app.runtime().unwrap().client_id("a").unwrap();
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(client, "demo", vec![0x5a]).unwrap();
    app.context().state().unwrap().resources.keys = keys;
    let profile = json!({"rules":[{"format":"nscripter/sar","path":"text.txt","transforms":[{"algorithm":"xor","key_id":"demo"}]}]});
    call(
        &mut app,
        "Resources",
        "defineProfile",
        json!(["keyed", profile]),
    )
    .unwrap();
    call(
        &mut app,
        "Resources",
        "mount",
        json!(["text.sar","text",{"format":"nscripter/sar","profile":"keyed"}]),
    )
    .unwrap();
    assert_eq!(
        call(&mut app, "Fs", "readText", json!(["text/text.txt"]))
            .unwrap()
            .as_str()
            .unwrap(),
        "archive text"
    );
    assert!(call(&mut app, "Resources", "defineProfile", json!(["keyed", {}])).is_err());
    let other = app.runtime().unwrap().client_id("b").unwrap();
    app.context().state().unwrap().client = other;
    assert!(call(
        &mut app,
        "Resources",
        "mount",
        json!(["text.sar","text",{"format":"nscripter/sar","profile":"keyed"}])
    )
    .is_err());
}
#[test]
fn mounting_invalidates_old_disk_cache_but_preserves_generated_resources() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let client = app.runtime().unwrap().client_id("a").unwrap();
    std::fs::write(tmp.path().join("assets/file.txt"), b"old disk").unwrap();
    std::fs::write(
        tmp.path().join("assets/files.sar"),
        sar("file.txt", b"archive"),
    )
    .unwrap();
    assert_eq!(
        app.runtime_mut()
            .unwrap()
            .assets
            .read_for(client, "file.txt")
            .unwrap()
            .as_ref(),
        b"old disk"
    );
    app.runtime_mut()
        .unwrap()
        .assets
        .register_generated(client, "__crysirror/keep", b"generated".to_vec())
        .unwrap();
    call(
        &mut app,
        "Resources",
        "mount",
        json!(["files.sar","",{"format":"nscripter/sar"}]),
    )
    .unwrap();
    assert_eq!(
        app.runtime_mut()
            .unwrap()
            .assets
            .read_for(client, "file.txt")
            .unwrap()
            .as_ref(),
        b"archive"
    );
    assert_eq!(
        app.runtime_mut()
            .unwrap()
            .assets
            .read_for(client, "__crysirror/keep")
            .unwrap()
            .as_ref(),
        b"generated"
    );
}
