// SPDX-License-Identifier: MPL-2.0
//! Keep the complete fullscreen post stack valid across backend/compiler upgrades.
#[test]
fn fullscreen_post_shaders_validate_with_current_naga() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
    let fullscreen = std::fs::read_to_string(root.join("fullscreen.wgsl")).unwrap();
    for name in ["ssao", "taa", "ssr", "brdf", "prefilter", "tonemap", "bloom"] {
        let source = format!("{fullscreen}\n{}", std::fs::read_to_string(root.join(format!("{name}.wgsl"))).unwrap());
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    }
}
