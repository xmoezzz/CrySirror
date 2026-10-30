// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Self-authored native-ABI window/reveal/input fixture, not a game boot claim.
use anyhow::{Context, Result, ensure};
use crysirror_siglus_vm::{
    fixture::{SceneBuilder, pack_scene},
    runtime::{
        constants::{cd, fm},
        forms::codes,
    },
};
fn main() -> Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("mwnd_fixture <new output directory>")?,
    );
    ensure!(!out.exists(), "output directory must be new");
    std::fs::create_dir_all(out.join("assets"))?;
    let chain = |op| {
        [
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_MWND,
            codes::ELM_ARRAY,
            0,
            op,
        ]
    };
    let mut s = SceneBuilder::default();
    s.command_string(&chain(codes::ELM_MWND_NAMAE), 0, "CrySirror", &[], fm::VOID);
    s.string("Native Siglus VM. Engine GlyphRun, reveal and window motion.");
    s.code.push(cd::TEXT);
    s.i32(-1);
    s.command(&chain(codes::ELM_MWND_R), 0, &[], fm::VOID);
    s.command_string(
        &chain(codes::ELM_MWND_NAMAE),
        0,
        "Second page",
        &[],
        fm::VOID,
    );
    s.string("One DECIDE release advances this native message page.");
    s.code.push(cd::TEXT);
    s.i32(-1);
    s.command(&chain(codes::ELM_MWND_PP), 0, &[], fm::VOID);
    s.command(&chain(codes::ELM_MWND_CLOSE_WAIT), 0, &[], fm::VOID);
    s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[500], fm::VOID);
    s.command(&chain(codes::ELM_MWND_OPEN_WAIT), 0, &[], fm::VOID);
    s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[5000], fm::VOID);
    s.code.push(cd::EOF);
    std::fs::write(out.join("assets/Scene.pck"), pack_scene(&s.finish()))?;
    std::fs::write(
        out.join("assets/Gameexe.ini"),
        "#START_SCENE=\"entry\"\n#SCREEN_SIZE=640,360\n#MWND.000.WINDOW_POS=20,180\n#MWND.000.WINDOW_SIZE=600,160\n#MWND.000.MESSAGE_POS=15,55\n#MWND.000.MOJI_CNT=22,3\n#MWND.000.MOJI_SIZE=24\n#MWND.000.MOJI_SPACE=1,10\n#MWND.000.NAME_WINDOW_POS=15,12\n#MWND.000.NAME_WINDOW_SIZE=0,0\n#MWND.000.NAME_MOJI_SIZE=28\n#MWND.000.NAME_MOJI_SPACE=2,10\n#MWND.000.OPEN_ANIME_TYPE=3\n#MWND.000.OPEN_ANIME_TIME=500\n#MWND.000.CLOSE_ANIME_TYPE=3\n#MWND.000.CLOSE_ANIME_TIME=500\n#WAKU.000.FILTER_COLOR=16,36,64,240\n#WAKU.000.FILTER_CONFIG_COLOR=0\n#WAKU.000.FILTER_CONFIG_TR=0\n",
    )?;
    let p = serde_json::json!({"schema":2,"name":"Authored native Siglus MWND fixture","content_revision":"mwnd-1","assets":"assets","canvas":[640,360],"background":[32,48,70,255],"clients":[{"name":"siglus","game":"authored-mwnd","adapter":"siglus","config":{"scene":"Scene.pck","gameexe":"Gameexe.ini","string_codec":"plain"}}]});
    std::fs::write(out.join("project.json"), serde_json::to_vec_pretty(&p)?)?;
    println!("{}", out.join("project.json").display());
    Ok(())
}
