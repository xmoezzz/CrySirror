// SPDX-License-Identifier: MPL-2.0
//! Every typed engine command must be reachable through the OOP API (and so
//! from scripts, whose bindings are generated from the same registry).
use crysirror_engine::{services::COMMAND_API, App};
use crysirror_runtime::Runtime;
use crysirror_scene::{layer::LayerCommand, ui::UiCommand, world::WorldCommand};
use serde::de::DeserializeOwned;

/// Variant names from serde's own "expected one of" diagnostic, so a new
/// command variant cannot be added without this test noticing.
fn variants<T: DeserializeOwned>() -> Vec<String> {
    let Err(err) = serde_json::from_value::<T>(serde_json::json!({"op": "__probe__"})) else {
        panic!("probe accepted")
    };
    let err = err.to_string();
    let list = err
        .split("expected one of ")
        .nth(1)
        .expect("serde variant list");
    list.split(',')
        .map(|s| {
            s.trim()
                .trim_matches(|c| c == '`' || c == ' ')
                .split('`')
                .next()
                .unwrap()
                .to_owned()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn every_command_variant_has_an_api_method_that_exists() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"cov","content_revision":"1","canvas":[64,64],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"cov"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("s")).unwrap(),
        "game",
    )
    .unwrap();
    let engine = app.engine().unwrap();
    let mut missing = Vec::new();
    for (system, ops) in [
        ("world", variants::<WorldCommand>()),
        ("layer", variants::<LayerCommand>()),
        ("ui", variants::<UiCommand>()),
        ("input_route", variants::<crysirror_scene::input_routes::InputRouteCommand>()),
    ] {
        assert!(ops.len() >= 6, "{system}: {ops:?}");
        for op in ops {
            match COMMAND_API
                .iter()
                .find(|(s, o, _)| *s == system && *o == op)
            {
                None => missing.push(format!("{system}.{op}")),
                Some((_, _, api)) => {
                    let (class, member) = api.split_once('.').unwrap();
                    let info = engine
                        .classes
                        .get(class)
                        .unwrap_or_else(|_| panic!("class {class}"));
                    let info = info.info();
                    assert!(
                        info.method(member).is_ok()
                            || info
                                .property(member)
                                .is_ok_and(|p| p.writable || member == "position"),
                        "{api} does not exist"
                    );
                }
            }
        }
    }
    assert!(missing.is_empty(), "commands without API: {missing:?}");
    // Every mapped API also appears in the generated script typings.
    let dts = app.typescript_declarations().unwrap();
    for (_, _, api) in COMMAND_API {
        let member = api.split_once('.').unwrap().1;
        assert!(
            dts.contains(&format!("  {member}(")) || dts.contains(&format!("  {member}:")),
            "{api} missing from .d.ts"
        );
    }
}
