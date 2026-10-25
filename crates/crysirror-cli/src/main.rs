// SPDX-License-Identifier: MPL-2.0
//! A data-driven service-contract replay tool, NOT a native game launcher.
#![forbid(unsafe_code)]
use crysirror_compat::{Bridge, CompatibilityProfile, EngineKind};
use crysirror_core::state::{
    AudioVoice, Color, Command, Rect, Sprite, TextRun, Visual, VisualNode,
};
use crysirror_host::ReferenceHost;
use serde_json::{json, Map, Value};
use std::{collections::BTreeSet, env, fs::File, io::Read, process::ExitCode};

type Result<T> = std::result::Result<T, String>;
const MAX_PROJECT: u64 = 4 * 1024 * 1024;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("CrySirror: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("status") if args.len() == 1 => {
            println!("{}", serde_json::to_string_pretty(&json!({
                "project": "CrySirror", "stage": "contract-replay",
                "scope": "crysirror-contract reference CLI only",
                "desktop_host": "separate crysirror executable",
                "native_vms": {"rfvp": false, "siglus": false},
                "renderer": false, "audio_output": false, "mcp_server": false,
                "console_targets": {"switch": "planned", "psv": "planned"},
                "service_contract_replay": true
            })).map_err(|e| e.to_string())?);
            Ok(())
        },
        Some("replay") if args.len() == 2 => {
            let input = load_json(&args[1])?;
            let output = replay(&input)?;
            println!("{}", serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?);
            Ok(())
        },
        Some("run") => Err("native RFVP/Siglus VM wiring is not implemented; replay accepts synthetic host-service fixtures only".into()),
        _ => Err("usage: crysirror-contract status | crysirror-contract replay <project.json>".into()),
    }
}

fn load_json(path: &str) -> Result<Value> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_PROJECT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_PROJECT {
        return Err("project exceeds 4 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn object<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>> {
    let map = value.as_object().ok_or("expected an object")?;
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown property: {key}"));
        }
    }
    Ok(map)
}
fn field<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    map.get(key)
        .ok_or_else(|| format!("missing property: {key}"))
}
fn string<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    field(map, key)?
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
fn uint(map: &Map<String, Value>, key: &str) -> Result<u32> {
    let n = field(map, key)?
        .as_u64()
        .ok_or_else(|| format!("{key} must be an unsigned integer"))?;
    u32::try_from(n).map_err(|_| format!("{key} exceeds u32"))
}
fn sint(map: &Map<String, Value>, key: &str) -> Result<i32> {
    let n = field(map, key)?
        .as_i64()
        .ok_or_else(|| format!("{key} must be an integer"))?;
    i32::try_from(n).map_err(|_| format!("{key} exceeds i32"))
}
fn rect(map: &Map<String, Value>) -> Result<Rect> {
    Ok(Rect {
        x: sint(map, "x")?,
        y: sint(map, "y")?,
        width: uint(map, "width")?,
        height: uint(map, "height")?,
    })
}
fn rgba(map: &Map<String, Value>) -> Result<Color> {
    let a = field(map, "rgba")?
        .as_array()
        .ok_or("rgba must be an array")?;
    if a.len() != 4 {
        return Err("rgba needs four channels".into());
    }
    let mut color = [0u8; 4];
    for (slot, value) in color.iter_mut().zip(a) {
        *slot = u8::try_from(
            value
                .as_u64()
                .ok_or("rgba channels must be unsigned integers")?,
        )
        .map_err(|_| "rgba channel exceeds 255")?;
    }
    Ok(Color(color))
}

fn lower(value: &Value, profile: &CompatibilityProfile) -> Result<Command> {
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or("missing op")?;
    match op {
        "text.put" => {
            let m = object(
                value,
                &[
                    "op", "id", "text", "font", "size_px", "x", "y", "width", "height", "z", "rgba",
                ],
            )?;
            Ok(Command::PutVisual {
                id: uint(m, "id")?,
                node: VisualNode {
                    z: sint(m, "z")?,
                    visual: Visual::Text(TextRun {
                        text: string(m, "text")?.into(),
                        font: string(m, "font")?.into(),
                        size_px: u16::try_from(uint(m, "size_px")?)
                            .map_err(|_| "font size exceeds u16")?,
                        bounds: rect(m)?,
                        color: rgba(m)?,
                        layout_profile: profile.layout_profile.clone(),
                    }),
                },
            })
        }
        "sprite.put" => {
            let m = object(
                value,
                &[
                    "op", "id", "asset", "x", "y", "width", "height", "z", "rgba",
                ],
            )?;
            Ok(Command::PutVisual {
                id: uint(m, "id")?,
                node: VisualNode {
                    z: sint(m, "z")?,
                    visual: Visual::Sprite(Sprite {
                        asset: profile
                            .asset_path(string(m, "asset")?)
                            .map_err(|e| e.to_string())?,
                        bounds: rect(m)?,
                        tint: rgba(m)?,
                    }),
                },
            })
        }
        "visual.remove" => {
            let m = object(value, &["op", "id"])?;
            Ok(Command::RemoveVisual { id: uint(m, "id")? })
        }
        "audio.play" => {
            let m = object(value, &["op", "channel", "asset", "looping", "gain_milli"])?;
            Ok(Command::PlayAudio {
                channel: uint(m, "channel")?,
                voice: AudioVoice {
                    asset: profile
                        .asset_path(string(m, "asset")?)
                        .map_err(|e| e.to_string())?,
                    looping: field(m, "looping")?
                        .as_bool()
                        .ok_or("looping must be boolean")?,
                    gain_milli: u16::try_from(uint(m, "gain_milli")?)
                        .map_err(|_| "gain exceeds u16")?,
                },
            })
        }
        "audio.stop" => {
            let m = object(value, &["op", "channel"])?;
            Ok(Command::StopAudio {
                channel: uint(m, "channel")?,
            })
        }
        _ => Err(format!("unsupported fixture operation: {op}")),
    }
}

fn replay(input: &Value) -> Result<Value> {
    let root = object(
        input,
        &["schema", "name", "console_targets", "clients", "expected"],
    )?;
    if uint(root, "schema")? != 1 {
        return Err("unsupported project schema".into());
    }
    if string(root, "name")?.is_empty() {
        return Err("name must not be empty".into());
    }
    let targets = field(root, "console_targets")?
        .as_array()
        .ok_or("console_targets must be an array")?;
    if targets.len() != 2
        || !targets.iter().any(|v| v.as_str() == Some("switch"))
        || !targets.iter().any(|v| v.as_str() == Some("psv"))
    {
        return Err("the console scope is exactly switch and psv".into());
    }
    let clients = field(root, "clients")?
        .as_array()
        .ok_or("clients must be an array")?;
    if clients.is_empty() || clients.len() > 64 {
        return Err("clients must contain 1..64 entries".into());
    }
    let mut host = ReferenceHost::default();
    let mut names = BTreeSet::new();
    for client in clients {
        let c = object(
            client,
            &["name", "engine", "game", "layout_profile", "commands"],
        )?;
        let name = string(c, "name")?;
        if name.is_empty() || !names.insert(name) {
            return Err("client names must be nonempty and unique".into());
        }
        let engine = match string(c, "engine")? {
            "rfvp" => EngineKind::Rfvp,
            "siglus" => EngineKind::Siglus,
            other => return Err(format!("unknown adapter: {other}")),
        };
        let id = host
            .register(engine.id(), string(c, "game")?)
            .map_err(|e| e.to_string())?;
        let profile = CompatibilityProfile::new(engine, string(c, "layout_profile")?.into())
            .map_err(|e| e.to_string())?;
        let commands = field(c, "commands")?
            .as_array()
            .ok_or("commands must be an array")?;
        if commands.len() > host.limits.batch_commands {
            return Err("too many commands".into());
        }
        let lowered: Vec<_> = commands
            .iter()
            .map(|v| lower(v, &profile))
            .collect::<Result<_>>()?;
        Bridge::new(id, profile)
            .submit(&mut host, &lowered)
            .map_err(|e| e.to_string())?;
    }
    let expected = object(field(root, "expected")?, &["visuals", "voices"])?;
    if uint(expected, "visuals")? as usize != host.scene.visuals().len()
        || uint(expected, "voices")? as usize != host.scene.voices().len()
    {
        return Err("service state does not match expected counts".into());
    }
    let visuals: Vec<Value> = host.scene.draw_order().iter().map(|(id, node)| {
        let (kind, payload) = match &node.visual {
            Visual::Text(t) => ("text", t.text.as_str()),
            Visual::Sprite(s) => ("sprite", s.asset.relative()),
        };
        json!({"client": id.client.0, "id": id.local, "z": node.z, "kind": kind, "payload": payload})
    }).collect();
    Ok(json!({
        "kind": "synthetic-service-contract-replay", "name": string(root, "name")?,
        "native_vm_connected": false, "pixels_rendered": false, "audio_emitted": false,
        "visual_count": host.scene.visuals().len(), "voice_count": host.scene.voices().len(),
        "visuals": visuals,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_accepts_the_repository_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../examples/fusion.json")).unwrap();
        let result = replay(&fixture).unwrap();
        assert_eq!(result["visual_count"], 4);
        assert_eq!(result["voice_count"], 2);
        assert_eq!(result["native_vm_connected"], false);
    }
    #[test]
    fn unknown_commands_fail_instead_of_succeeding_as_noops() {
        let p = CompatibilityProfile::new(EngineKind::Rfvp, "fixture".into()).unwrap();
        assert!(lower(&json!({"op":"pretend.3d"}), &p).is_err());
    }
    #[test]
    fn rejects_duplicate_client_names() {
        let mut fixture: Value =
            serde_json::from_str(include_str!("../../../examples/fusion.json")).unwrap();
        fixture["clients"][1]["name"] = fixture["clients"][0]["name"].clone();
        assert!(replay(&fixture).is_err());
    }
}
