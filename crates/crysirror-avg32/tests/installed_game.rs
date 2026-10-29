// SPDX-License-Identifier: MPL-2.0
//! A self-authored AVG32 installation (Gameexe.ini, TPC32 scene, PDT10
//! picture, WAV track) run by the original interpreter through engine
//! services. No game data or fonts are used.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame, buttons},
    state::ClientId,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::{Value, json};
use std::path::Path;

fn number(value: u32) -> Vec<u8> {
    let mut len = 1usize;
    while len < 5 && (value >> 4) >> (8 * (len - 1)) != 0 {
        len += 1;
    }
    let mut out = vec![((len as u8) << 4) | (value as u8 & 15)];
    for i in 0..len - 1 {
        out.push((value >> (4 + 8 * i)) as u8);
    }
    out
}

fn scene(code: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 0x63];
    bytes[..5].copy_from_slice(b"TPC32");
    bytes[0x28..0x2c].copy_from_slice(&1i32.to_le_bytes());
    bytes[0x5f..0x63].copy_from_slice(&1i32.to_le_bytes());
    bytes.extend_from_slice(code);
    bytes
}

/// A solid 640x480 PDT10: one literal pixel, then distance-1 runs of 16.
fn pdt10(rgb: [u8; 3]) -> Vec<u8> {
    let mut pdt = vec![0u8; 0x20];
    pdt[..5].copy_from_slice(b"PDT10");
    pdt[0x0c..0x10].copy_from_slice(&640u32.to_le_bytes());
    pdt[0x10..0x14].copy_from_slice(&480u32.to_le_bytes());
    let mut remaining: u32 = 640 * 480 - 1;
    let mut first = true;
    while remaining > 0 {
        let mut flags = 0u8;
        let mut body = Vec::new();
        for bit in 0..8 {
            if first {
                flags |= 0x80 >> bit;
                body.extend_from_slice(&[rgb[2], rgb[1], rgb[0]]);
                first = false;
            } else if remaining > 0 {
                body.extend_from_slice(&0x000fu16.to_le_bytes());
                remaining = remaining.saturating_sub(16);
            }
        }
        pdt.push(flags);
        pdt.extend(body);
    }
    let size = pdt.len() as u32;
    pdt[8..12].copy_from_slice(&size.to_le_bytes());
    pdt
}

fn wav() -> Vec<u8> {
    let frames = 4410u32;
    let bytes = frames * 4;
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&44100u32.to_le_bytes());
    out.extend_from_slice(&(44100u32 * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    out.resize(out.len() + bytes as usize, 0);
    out
}

const GAMEEXE: &str = "#CAPTION=\"TEST\"\r\n#REGNAME=\"KEY\\KANON\"\r\n#SEEN_START=001\r\n\
#MUSIC_TYPE=003\r\n#CDTRACK=02:00:00:00-02:01:00:00-02:00:00:00=\"002\"\r\n\
#DIRC.PDT=\"PDT\" =N\r\n#DIRC.TXT=\"DAT\" =N\r\n#DIRC.***=\"DAT\" =N\r\n\
#WINDOW_MSG_POS=022,350\r\n#MESSAGE_SIZE=023,003\r\n#MSG_MOJI_SIZE=012,029\r\n#FONT_SIZE=026\r\n";

/// テスト in Shift-JIS.
const TEXT: [u8; 6] = [0x83, 0x65, 0x83, 0x58, 0x83, 0x67];

fn fixture(code: &[u8]) -> (tempfile::TempDir, Runtime) {
    let temp = tempfile::tempdir().unwrap();
    let game = temp.path().join("assets/game");
    for dir in ["PDT", "DAT", "BGM"] {
        std::fs::create_dir_all(game.join(dir)).unwrap();
    }
    std::fs::write(game.join("GAMEEXE.INI"), GAMEEXE).unwrap();
    std::fs::write(game.join("DAT/SEEN001.TXT"), scene(code)).unwrap();
    std::fs::write(game.join("PDT/BG.PDT"), pdt10([40, 80, 120])).unwrap();
    std::fs::write(game.join("BGM/TRACK02.WAV"), wav()).unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"avg32-install","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"avg32","game":"synthetic","config":{"root":"game","font":"default"}}]})).unwrap();
    let path = temp.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_avg32::register(&mut registry).unwrap();
    let runtime =
        Runtime::with_registry(project, &path, &temp.path().join("state"), registry).unwrap();
    (temp, runtime)
}

fn diag(r: &Runtime) -> Value {
    r.inspect()["sessions"][0]["native"].clone()
}

fn layer(r: &Runtime, id: u32) -> Value {
    r.inspect()["engine"]["layers"]["clients"]["1"][id.to_string()].clone()
}

fn ticks(r: &mut Runtime, count: usize, input: InputFrame) {
    for _ in 0..count {
        r.tick(16, input).unwrap();
    }
}

/// `0x0b:09 "BG" 0` load into buffer 0, `0x0e:01 "002"` loop BGM, a text
/// packet, `0x01` click wait, then `0x3b` sets value 1 to 7.
fn story() -> Vec<u8> {
    let mut code = vec![0x0b, 0x09, b'B', b'G', 0];
    code.extend(number(0));
    code.extend([0x0e, 0x01]);
    code.extend(b"002\0");
    code.push(0xff);
    code.extend(TEXT);
    code.push(0);
    code.push(0x01);
    code.push(0x3b);
    code.extend(number(1));
    code.extend(number(7));
    code.extend([0x60, 0x20]);
    code
}

#[test]
fn installed_game_draws_through_shared_raster_glyph_and_audio_services() {
    let (_temp, mut r) = fixture(&story());
    ticks(&mut r, 240, InputFrame::default());
    let d = diag(&r);
    assert_eq!(d["owns_framebuffer"], false);
    assert_eq!(d["version"], 1613, "{d}");
    assert_eq!(d["warnings"], json!([]), "{d}");
    // The presented PDT screen is one engine surface layer.
    let screen = layer(&r, 1);
    let asset = screen["paint"]["asset"].as_str().unwrap().to_owned();
    assert!(asset.starts_with("__surface/"), "{screen}");
    assert_eq!(screen["rect"], json!([0.0, 0.0, 640.0, 480.0]));
    // Message text is a retained engine GlyphRun, not pixels in the PDT.
    let text = layer(&r, 2);
    assert_eq!(text["paint"]["kind"], "glyph_run", "{text}");
    let glyphs: String = text["paint"]["glyphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["character"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(glyphs, "テスト");
    // `#CDTRACK` "002" resolves to the ripped BGM/TRACK02.WAV.
    let audio = d["recent_audio"].to_string();
    assert!(audio.contains("play 20000"), "{audio}");
    // The click wait holds the following arithmetic until a release.
    let mut values = d["scene"].clone();
    assert_eq!(values, 1);
    let held = InputFrame {
        buttons_down: buttons::MOUSE_LEFT,
        ..InputFrame::default()
    };
    ticks(&mut r, 2, held);
    ticks(&mut r, 30, InputFrame::default());
    values = diag(&r)["position"].clone();
    assert!(values.as_u64().unwrap() as usize >= story().len() - 2, "{values}");
}

#[test]
fn overwriting_buffer_zero_removes_the_glyphs_standing_there() {
    // Text, click, then reload the picture over the whole screen.
    let mut code = vec![0xff];
    code.extend(TEXT);
    code.push(0);
    code.push(0x01);
    code.extend([0x0b, 0x09, b'B', b'G', 0]);
    code.extend(number(0));
    code.extend([0x60, 0x20]);
    let (_temp, mut r) = fixture(&code);
    ticks(&mut r, 240, InputFrame::default());
    assert_eq!(diag(&r)["glyphs"], 3);
    let held = InputFrame {
        buttons_down: buttons::MOUSE_LEFT,
        ..InputFrame::default()
    };
    ticks(&mut r, 2, held);
    ticks(&mut r, 30, InputFrame::default());
    assert_eq!(diag(&r)["glyphs"], 0, "{}", diag(&r));
}

#[test]
fn movies_report_the_missing_shared_decoder_instead_of_decoding_privately() {
    let mut code = vec![0x0e, 0x50];
    code.extend(b"OP\0");
    code.extend([0x60, 0x20]);
    let (temp, mut r) = fixture(&code);
    std::fs::write(temp.path().join("assets/game/DAT/OP.AVI"), b"RIFF").unwrap();
    ticks(&mut r, 10, InputFrame::default());
    let warnings = diag(&r)["warnings"].to_string();
    assert!(warnings.contains("shared engine video decoder"), "{warnings}");
}

#[test]
fn original_files_are_read_only_and_saves_use_the_save_mount() {
    let (temp, mut r) = fixture(&story());
    ticks(&mut r, 2, InputFrame::default());
    let storage = r.host.storage(ClientId(1)).unwrap();
    let fs = crysirror_avg32::vm::fs::Storage {
        storage: storage.clone(),
        root: "game".into(),
    };
    crysirror_avg32::vm::fs::with(&fs, || {
        assert!(crysirror_avg32::vm::fs::write(Path::new("GAMEEXE.INI"), b"x").is_err());
        crysirror_avg32::vm::fs::write(Path::new("@save/SAVE.INI"), b"slot").unwrap();
        assert_eq!(
            crysirror_avg32::vm::fs::read(Path::new("@save/SAVE.INI")).unwrap(),
            b"slot"
        );
    });
    assert_eq!(
        std::fs::read(temp.path().join("assets/game/GAMEEXE.INI")).unwrap(),
        GAMEEXE.as_bytes()
    );
    let saved = storage
        .read(&VfsPath::new(Mount::Save, "SAVE.INI").unwrap())
        .unwrap();
    assert_eq!(saved, b"slot");
}


#[test]
fn choice_highlight_recolours_glyphs_instead_of_removing_them() {
    // `0x58:02` in-window choice into value 5 with two items.
    let mut code = vec![0x58, 0x02];
    code.extend(number(5));
    code.push(0x22);
    for _ in 0..2 {
        code.push(0xff);
        code.extend(TEXT);
        code.extend([0, 0]);
    }
    code.push(0x23);
    code.extend([0x60, 0x20]);
    let (_temp, mut r) = fixture(&code);
    ticks(&mut r, 60, InputFrame::default());
    let colours = |r: &Runtime| -> Vec<String> {
        layer(r, 2)["paint"]["glyphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["color"].to_string())
            .collect()
    };
    let before = colours(&r);
    assert_eq!(before.len(), 6, "{before:?}");
    let mut highlighted = None;
    for y in (330..480).step_by(4) {
        let hover = InputFrame {
            pointer_x: 200,
            pointer_y: y,
            ..InputFrame::default()
        };
        ticks(&mut r, 2, hover);
        let now = colours(&r);
        assert_eq!(now.len(), 6, "hover at y={y} removed glyphs: {now:?}");
        if now != before {
            highlighted = Some(now);
            break;
        }
    }
    let highlighted = highlighted.expect("no row highlighted");
    assert_eq!(
        highlighted.iter().zip(&before).filter(|(a, b)| a != b).count(),
        3,
        "{highlighted:?}"
    );
}
