// SPDX-License-Identifier: MPL-2.0
//! Generate only self-authored native fixture data, outside the source export.
use std::path::PathBuf;
fn main() -> anyhow::Result<()> {
    let out = PathBuf::from(std::env::args_os().nth(1).ok_or_else(|| {
        anyhow::anyhow!(
            "usage: cargo run -p crysirror-siglus-vm --example fixture -- /tmp/native-fixture"
        )
    })?);
    std::fs::create_dir_all(out.join("assets"))?;
    std::fs::write(
        out.join("assets/Scene.pck"),
        crysirror_siglus_vm::fixture::pack_scene(&crysirror_siglus_vm::fixture::image_scene()),
    )?;
    std::fs::write(
        out.join("assets/native.g00"),
        crysirror_siglus_vm::fixture::pixel_g00(),
    )?;
    let project = serde_json::json!({"schema":2,"name":"Siglus original VM on CrySirror","content_revision":"native-fixture-1","canvas":[640,360],"assets":"assets","clients":[{"name":"original-siglus","adapter":"siglus","game":"synthetic-native","config":{"scene":"Scene.pck","string_codec":"plain","resources":{"native.g00":"native.g00"}}}],"scene3d":{"camera":{"eye":[4,3,6],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.1,"far":100},"meshes":[{"name":"floor","geometry":{"kind":"cube"},"translation":[0,-1,0],"scale":[6,0.2,6],"color":[70,85,110,255]},{"name":"native-overlay-and-shared-world","geometry":{"kind":"cube"},"color":[225,145,70,255]}]}});
    std::fs::write(
        out.join("project.json"),
        serde_json::to_vec_pretty(&project)?,
    )?;
    println!("{}", out.join("project.json").display());
    Ok(())
}
