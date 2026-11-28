// SPDX-License-Identifier: MPL-2.0
//! Opt-in read-only corpus scan for the admitted Midori 2011 profile.
use super::*;
use crysirror_core::service::{AssetReader, ClientStorage, MemoryAssetReader};
use std::{fs, path::PathBuf};

struct ReadOnly(PathBuf);
impl ClientStorage for ReadOnly {
    fn read(&self, path: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        if path.mount() != Mount::Asset {
            return Err(Error::ReadOnly);
        }
        let path = self.0.join(path.relative());
        let read = || -> std::io::Result<Vec<u8>> {
            let mut file = fs::File::open(path)?;
            use std::io::Read;
            let mut bytes = Vec::new();
            file.by_ref()
                .take(crysirror_resource::pe::MAX_PE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > crysirror_resource::pe::MAX_PE_BYTES {
                return Err(std::io::Error::other("midori census file limit"));
            }
            Ok(bytes)
        };
        read().map_err(|e| Error::Io(e.to_string()))
    }
    fn open_read(&self, path: &VfsPath) -> crysirror_core::Result<Arc<dyn AssetReader>> {
        Ok(Arc::new(MemoryAssetReader(self.read(path)?.into())))
    }
    fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
    fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
    fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
        Err(Error::Unsupported("unused Midori census list"))
    }
}

/// Check the real outer definition predecessor without executing either
/// commercial scenario. Header flags in the high byte of the block CSV's
/// numeric mode are not the native low marker bit.
fn verify_definition_frontier(bytes: &[u8]) -> Result<()> {
    let mut script = Script::new(bytes.to_vec(), 3)?;
    ensure!(script.pc == 0x114 && script.remaining == 82, "different stock definition header");
    ensure!(matches!(script.next()?, Action::DefineDebug(_)), "different stock definition prefix");
    ensure!(script.pc == 0x155 && word(bytes, script.pc)? == 0x1100_0005,
        "different first stock definition checkpoint offset");
    let (raw, _) = cstring(bytes, script.pc + 4)?;
    ensure!(raw.splitn(5, |&b| b == b',').take(4).eq(
        [b"-1".as_slice(), b"16777216", b"-1", b"*"]),
        "different first definition checkpoint condition"
    );
    let Action::Block(first) = script.next()? else {
        bail!("different first stock definition checkpoint");
    };
    ensure!(!first.marker && first.checkpoint_flag == Some(false),
        "stock first definition record is not a passive block marker");
    ensure!(script.pc == 0x16c, "different first definition checkpoint extent");
    ensure!(word(bytes, 0x120c)? == 0x1101_0014 && word(bytes, 0x1210)? == 200
        && cstring(bytes, 0x1214)?.0 == b"Q3\x81\x83104",
        "different definition OS capability branch");
    ensure!(word(bytes, 0x20c8)? == 0x0100_00bf
        && cstring(bytes, 0x20cc)?.0.splitn(7, |&b| b == b',').last() == Some(br"res\c\settings.user.rnf"),
        "different definition RNF program");
    ensure!(word(bytes, 0x20f3)? == 0x2100_0011
        && cstring(bytes, 0x20f7)?.0 == b"defChara,*,0",
        "different nested definition handoff");
    Ok(())
}

#[test]
#[ignore = "requires the user's installed Midori 2011 tree; static reads only"]
fn static_midori_2011_corpus() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("CRYSIRROR_EXHIBIT_MIDORI_ROOT")
            .context("set CRYSIRROR_EXHIBIT_MIDORI_ROOT to the 2011 fixture root")?,
    );
    let seeds = profile::Seeds::read(
        &ReadOnly(root.clone()),
        &profile::Sources {
            exe: "ExHIBIT.exe".into(),
            ini: "ExHIBIT.ini".into(),
            resident: "resident.dll".into(),
        },
    )?;
    let mut ops = BTreeMap::<(u16, u8, u8), [usize; 2]>::new();
    // Record selectors, not history, textual operands or resource names.
    let mut calls = BTreeMap::<(u16, Option<i32>, Option<i32>), [usize; 2]>::new();
    let mut files = 0;
    let mut saw_start = false;
    let mut saw_definition_frontier = false;
    let mut marker = None;
    let mut marker_offsets = Vec::new();
    let mut debug_offsets = Vec::new();
    let mut control_part = None;
    let mut control_sound = None;
    let mut control_bundle = None;
    let mut shortcut_offsets = Vec::new();
    let mut shortcuts = midori_shortcuts::Catalog::default();
    let mut authored_actors = character::Catalog::new(37);
    let mut actor_slots = midori_actor::Slots::default();
    let mut ambience_ids = std::collections::BTreeSet::new();
    let mut portraits = Vec::new();
    let mut background_assets = BTreeMap::new();
    let mut solid_backgrounds = [0; 2];
    let mut filter_captures = BTreeMap::new();
    let mut stock_filter = None;
    let mut music = BTreeMap::<i32, u64>::new();
    let mut used_ambience = std::collections::BTreeSet::new();
    let mut tally = [0usize; 5]; // files' prefixes, opcodes, numbers, strings, total
    for entry in fs::read_dir(root.join("rld"))? {
        let path = entry?.path();
        if !path
            .extension()
            .is_some_and(|n| n.eq_ignore_ascii_case("rld"))
        {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .context("Midori scenario filename")?;
        ensure!(
            fs::metadata(&path)?.len() <= 8 << 20,
            "oversized Midori scenario"
        );
        let mut bytes = fs::read(&path)?;
        seeds.decode(name, &mut bytes)?;
        let mut script = Script::new(bytes, 3)?;
        let mut at = script.pc;
        tally[0] += at;
        tally[4] += script.bytes.len();
        let count = script.remaining;
        if name.eq_ignore_ascii_case("start") {
            ensure!(script.pc == 0x114, "different Midori startup entry");
            let Action::Block(first) = script.next()? else {
                anyhow::bail!("different Midori startup descriptor");
            };
            ensure!(
                first.id == -1
                    && first.false_target == 11
                    && !first.marker
                    && first.checkpoint_flag == Some(false),
                "different Midori startup block form"
            );
            ensure!(script.pc == 0x133, "different Midori first action offset");
            ensure!(
                matches!(script.next()?, Action::Change(_)),
                "different Midori first action"
            );
            ensure!(script.pc == 0x148, "different Midori second action offset");
            ensure!(
                word(&script.bytes, script.pc)? == 0x010000bf,
                "different Midori action header"
            );
            let (raw, _) = cstring(&script.bytes, script.pc + 4)?;
            ensure!(
                raw.split(|b| *b == b',').next() == Some(b"250".as_slice()),
                "different Midori second action"
            );
            saw_start = true;
        }
        if name.eq_ignore_ascii_case("def") {
            verify_definition_frontier(&script.bytes)?;
            saw_definition_frontier = true;
        }
        for _ in 0..count {
            let pc = at;
            let header = word(&script.bytes, at)?;
            let op = header as u16;
            let arity = (header >> 16) as u8;
            let flags = (header >> 24) as u8;
            at += 4 + 4 * usize::from(arity);
            ensure!(at <= script.bytes.len(), "truncated Midori operand vector");
            tally[1] += 4;
            tally[2] += 4 * usize::from(arity);
            let mut nested = None;
            for i in 0..(flags & 15) {
                let (raw, end) = cstring(&script.bytes, at)?;
                tally[3] += end - at;
                if i == 0 && matches!(op, 0x0b | 0xbf) {
                    let mut fields = raw.splitn(3, |b| *b == b',');
                    let selector = fields.next().and_then(|v| branch::number(v).ok());
                    // The second word of 0xbf is a result destination, not an
                    // action-mode selector; do not count it as a method.
                    let sub = if op == 0x0b {
                        fields.next().and_then(|v| branch::number(v).ok())
                    } else {
                        None
                    };
                    nested = Some((selector, sub));
                }
                at = end;
            }
            script.pc = pc;
            script.remaining = 1;
            let decoded = script.next();
            let accepted = usize::from(decoded.is_ok());
            if accepted == 1 {
                ensure!(script.pc == at, "partial Midori opcode admission {op:#06x}");
            }
            match decoded {
                Ok(Action::DefineDebug(def)) => {
                    ensure!(name == "def" && def.mode == 0 && def.pairs == [[-1, 0]; 8], "different stock Midori debug mode");
                    debug_offsets.push(pc);
                }
                Ok(Action::DefineMidoriMarker(def)) => {
                    ensure!(name == "def" && marker.as_ref().is_none_or(|first| first == &def), "different stock Midori marker definitions");
                    marker_offsets.push(pc);
                    marker = Some(def);
                }
                Ok(Action::DefineMidoriControlPart(part)) => {
                    ensure!(name == "def" && pc == 0x6f7 && part.slot == 5 && part.position == [736, 374]
                        && control_part.is_none(), "different stock Midori control part");
                    control_part = Some(part);
                }
                Ok(Action::DefineMidoriControlParts(parts)) => {
                    ensure!(name == "def" && pc == 0x689 && parts.0.len() == 2
                        && parts.0.iter().enumerate().all(|(id,p)| p.slot as usize == id && p.position == [736,374])
                        && control_bundle.is_none(), "different stock Midori bundled controls");
                    control_bundle = Some(parts);
                }
                Ok(Action::DefineMidoriShortcut(def)) => {
                    ensure!(name == "def", "different stock Midori shortcut scenario");
                    shortcut_offsets.push(pc);
                    shortcuts.define(def);
                }
                Ok(Action::MidoriControlSounds(update)) => {
                    ensure!(
                        name == "def" && pc == 0x224 && update.execute && update.first == 0
                            && update.values == [299,-1,299,299,299,299,299,299,299,299,299,-1,-1,-1,-1,299,-1,299,-1]
                            && control_sound.is_none(), "different stock Midori control sound bank"
                    );
                    control_sound = Some(update);
                }
                Ok(Action::MidoriMusic(track)) => {
                    let total = if let Some(total) = music.get(&track.id) {
                        *total
                    } else {
                        let path = track.path()?;
                        let read = ReadOnly(root.clone());
                        let body = read.open_read(&path)?;
                        let bytes = crysirror_resource::read_all(body.as_ref(), 16 << 20)?;
                        let total = stock_ogg_frames(&bytes)?;
                        music.insert(track.id, total);
                        total
                    };
                    ensure!(
                        track.loop_start < total,
                        "Midori music repeat start outside source"
                    );
                }
                Ok(Action::DrawMidoriBackground(draw)) => {
                    if let Some(path) = draw.asset() {
                        background_assets
                            .entry((draw.crop.is_some(), draw.duration > 0, path))
                            .or_insert_with(Vec::new)
                            .push(draw);
                    } else {
                        solid_backgrounds[usize::from(draw.duration > 0)] += 1;
                    }
                }
                Ok(Action::MidoriColorFilter(filter)) => {
                    if name == "M301C101" && pc == 0x38f {
                        stock_filter = Some(filter);
                    }
                    *filter_captures
                        .entry(filter.capture(b"stale").context("missing stale T4 write")?)
                        .or_insert(0usize) += 1;
                }
                Ok(Action::DrawMidoriActor(draw)) => {
                    portraits.push((name.to_owned(), pc, draw));
                }
                Ok(Action::DefineMidoriActorSlot(def)) => {
                    ensure!(
                        name.eq_ignore_ascii_case("def"),
                        "unexpected Midori draw slot scope"
                    );
                    actor_slots.define(def)?;
                }
                Ok(Action::DefineMidoriAmbient(def)) => {
                    ensure!(
                        name.eq_ignore_ascii_case("def"),
                        "unexpected Midori ambience scope"
                    );
                    ensure!(
                        ambience_ids.insert(def.id()),
                        "duplicate Midori ambience fixture"
                    );
                    let source = fs::read(root.join(def.path()?.relative()))?;
                    ensure!(
                        source.len() >= 44
                            && &source[..4] == b"RIFF"
                            && &source[8..16] == b"WAVEfmt "
                            && word(&source, 16)? == 16
                            && matches!(word(&source, 20)?, 0x0001_0001 | 0x0002_0001)
                            && word(&source, 24)? == 22050
                            && matches!(word(&source, 32)?, 0x0010_0002 | 0x0010_0004)
                            && &source[36..40] == b"data",
                        "different Midori ambience format"
                    );
                }
                Ok(Action::MidoriAmbient(midori_ambient::Command::Play { id, .. })) => {
                    used_ambience.insert(id);
                }
                Ok(Action::DefineCharacter(definition)) => {
                    ensure!(
                        name.eq_ignore_ascii_case("defChara"),
                        "unexpected Midori actor scope"
                    );
                    authored_actors.define(definition)?;
                }
                Ok(Action::DefineBody2011(definition)) => {
                    ensure!(
                        name.eq_ignore_ascii_case("defChara"),
                        "unexpected Midori body scope"
                    );
                    authored_actors.define_body_2011(definition)?;
                }
                _ => {}
            }
            ops.entry((op, flags, arity)).or_default()[accepted] += 1;
            if matches!(op, 0x0b | 0xbf) {
                let (selector, sub) = nested.unwrap_or((None, None));
                calls.entry((op, selector, sub)).or_default()[accepted] += 1;
            }
        }
        ensure!(at == script.bytes.len(), "trailing Midori record data");
        files += 1;
    }
    ensure!(
        tally[..4].iter().sum::<usize>() == tally[4],
        "unaccounted Midori bytes"
    );
    ensure!(saw_start && saw_definition_frontier && files == 379, "different Midori corpus");
    println!("MIDORI_BOOTSTRAP def_first_capture=0x155 os_branch=0x120c settings_user=0x20c8 nested_definition=0x20f3 direct_start_capture=0x114");
    ensure!(
        (authored_actors.len(), authored_actors.body_count()) == (34, 270),
        "different Midori actor declaration inventory"
    );
    assert_eq!(
        actor_slots.startup_layers(),
        [Some([137, 138]), Some([139, 140]), Some([141, 142])]
    );
    ensure!(
        ambience_ids.len() == 14
            && used_ambience.len() == 9
            && used_ambience.is_subset(&ambience_ids),
        "different Midori authored ambience inventory"
    );
    use crysirror_core::image_processing::ImageInput;
    ensure!(
        music.len() == 17 && music.get(&11) == Some(&4_190_619),
        "different Midori music inventory"
    );
    println!("MIDORI_MUSIC verified_vorbis_tracks={}", music.len());
    let storage = ReadOnly(root);
    let mut distinct_portraits = BTreeMap::new();
    let mut filtered_asset = None;
    let mut scoped_positions = 0;
    let mut observed = [None; 2];
    for (scenario, pc, portrait) in &portraits {
        let plan = portrait.face_plan(&authored_actors)?;
        let input = plan.read(&storage, Default::default())?;
        if scenario == "M301C101" && *pc == 0x445 {
            ensure!(plan.cells > 1, "missing stock filtered face atlas");
            let filter = stock_filter.context("missing stock portrait filter")?;
            filtered_asset = Some((input.clone(), plan.read(&storage, filter)?));
        }
        let base = match &input {
            ImageInput::Composite { base, .. } => base.as_ref(),
            image => image,
        };
        let ImageInput::Encoded { size, .. } = base else {
            bail!("expected stock indexed body")
        };
        if portrait.is_packed() {
            scoped_positions += 1;
            let point = portrait.origin(*size, plan.correction, [1024, 576])?;
            if scenario == "M1020301" && *pc == 0x2245 {
                observed[0] = Some(point);
            }
            if scenario == "M1150201" && *pc == 0x2441 {
                observed[1] = Some(point);
            }
        }
        distinct_portraits
            .entry((plan.actor, plan.body))
            .or_insert(input);
    }
    ensure!(
        portraits.len() == 116
            && distinct_portraits.len() == 25
            && scoped_positions == 64
            && observed == [Some([-180, 390]), Some([-34, 395])],
        "different Midori immediate portrait inventory or positions"
    );
    ensure!(
        filter_captures.get(&(4, String::new())) == Some(&4)
            && filter_captures.get(&(4, "0,16416,48".to_owned())) == Some(&16)
            && filter_captures.len() == 2
            && ops.get(&(0xa7, 1, 0)) == Some(&[1, 20]),
        "different Midori filter mode census"
    );
    let mut admitted_backgrounds = [0; 2];
    let mut scaled_only = [0; 2];
    let mut distinct_backgrounds = [0; 2];
    let mut cropped_backgrounds = [0; 2];
    let mut background_draws = BTreeMap::new();
    for ((cropped, fading, path), draws) in background_assets {
        let mode = usize::from(fading);
        let count = draws.len();
        let draw = draws[0];
        let mut header = [0u8; 36];
        std::io::Read::read_exact(&mut fs::File::open(storage.0.join(&path))?, &mut header)?;
        let size = [16, 20].map(|at| u32::from_le_bytes(header[at..at + 4].try_into().unwrap()));
        if cropped {
            for draw in draws {
                ensure!(
                    draw.inspect(&header)? == size,
                    "invalid retained asset crop geometry"
                );
            }
            background_draws.insert(path, draw);
            cropped_backgrounds[mode] += count;
        } else if size != midori_background::VIEW {
            ensure!(
                (!fading && size == [1024, 577]) || (fading && size == [2048, 1152]),
                "different held background geometry"
            );
            ensure!(
                draw.inspect(&header).is_err(),
                "unsupported size not guarded"
            );
            scaled_only[mode] += count;
        } else {
            draw.inspect(&header)?;
            distinct_backgrounds[mode] += 1;
            background_draws.insert(path, draw);
            admitted_backgrounds[mode] += count;
        }
    }
    ensure!(
        solid_backgrounds == [4, 49]
            && admitted_backgrounds == [90, 1219]
            && scaled_only == [2, 3]
            && distinct_backgrounds == [47, 330]
            && cropped_backgrounds == [35, 1436]
            && ops.get(&(0x4a, 0, 16)) == Some(&[1021, 2838])
            && ops.get(&(0x50, 0, 8)) == Some(&[72, 8]),
        "different Midori renderer fixture coverage"
    );
    ensure!(debug_offsets == [0x114] && marker_offsets == [0x158f, 0x1a37], "different Midori prelude command offsets");
    let marker = marker.context("missing Midori prelude marker")?.stock_input(&storage)?;
    let mut parts = control_bundle.context("missing Midori control bundle")?.0;
    parts.push(control_part.context("missing Midori control part")?);
    let control_parts = parts.iter().map(|part| part.stock_input(&storage)).collect::<Result<Vec<_>>>()?;
    ensure!(
        shortcut_offsets == [0x10ce, 0x10e5, 0x1122, 0x1136, 0x114a, 0x115f, 0x1174, 0x1188, 0x119d, 0x11b2, 0x11c7, 0x11dc]
            && shortcuts.ordinary_count() == 11,
        "different stock Midori shortcuts"
    );
    let shortcut_image = serde_json::to_value(&shortcuts)?;
    ensure!(
        shortcut_image["right"]["action"] == 135
            && shortcut_image["keys"]["112"]["parameter"] == "57, 14, 0x00100001, 4, 0, 850,10-14,-1,*",
        "different stock Midori shortcut route inputs"
    );
    let control_audio = control_sound.context("missing Midori plain sound bank")?.stock_audio_paths()?
        .into_iter().collect::<std::collections::BTreeSet<_>>();
    ensure!(control_audio.len() == 1, "different stock Midori control audio paths");
    for path in control_audio {
        let bytes = storage.read(&path)?;
        let data = usize::try_from(word(&bytes, 40)?)?;
        ensure!(bytes.len() >= 44, "short control WAV");
        ensure!(
            bytes.get(..4) == Some(b"RIFF") && bytes.get(8..16) == Some(b"WAVEfmt ")
                && [16,20,24,32,36,40].map(|offset| word(&bytes, offset).unwrap()) ==
                    [16,0x0001_0001,22050,0x0010_0002,u32::from_le_bytes(*b"data"),18756]
                && bytes.len() == 44 + data && data % 2 == 0
                && bytes[44..].chunks_exact(2).any(|p| p != [0,0]),
            "different stock Midori control sound container or samples"
        );
    }

    let (comparisons, body_only, filtered_pixels, background_pixels) = check_images(
        distinct_portraits.into_values(),
        filtered_asset.context("missing stock indexed filter fixture")?,
        background_draws.values().map(|draw| draw.read(&storage)),
        marker,
        control_parts,
    )?;
    println!(
        "MIDORI_PORTRAITS placements={} packed={scoped_positions} body_profiles=25 body_only={body_only} compared_pixels={comparisons}",
        portraits.len()
    );
    println!("MIDORI_PRELUDE debug_clear=1 marker_definitions=2 marker_rgba_pixels=7200 control_part_definitions=1 control_part_rgba_pixels=32604 control_bundle_definitions=1 shortcuts=12 control_sound_slots=19 control_sound_pcm_frames=9378");
    println!("MIDORI_FILTER mode0=20 mode1_held=1 compared_pixels={filtered_pixels}");
    println!(
        "MIDORI_BACKGROUND mode255_asset_ready=94 mode9_asset_ready=1268 mode9_requires_prior_live_background=1 held_size=5 retained_crop=1471 ordinary_pan_tuples=8 distinct_assets={} decoded_pixels={background_pixels}",
        background_draws.len()
    );
    println!(
        "MIDORI_AMBIENCE definitions={} used={}",
        ambience_ids.len(),
        used_ambience.len()
    );
    println!(
        "MIDORI_ACTORS definitions={} bodies={}",
        authored_actors.len(),
        authored_actors.body_count()
    );
    let records = ops.values().map(|[r, a]| r + a).sum::<usize>();
    ensure!(records == 51_797, "different Midori record census");
    println!(
        "MIDORI_CENSUS files={files} records={records} signatures={}",
        ops.len()
    );
    println!(
        "MIDORI_BYTES prefix={} codes={} numeric={} strings={} leftovers=0",
        tally[0], tally[1], tally[2], tally[3]
    );
    for ((op, flags, arity), [rejected, admitted]) in ops {
        println!("MIDORI_OP {op:04x}\t{flags:02x}\t{arity}\t{admitted}\t{rejected}");
    }
    for ((op, primary, raw_sub), [rejected, admitted]) in calls {
        println!("MIDORI_CALL {op:04x}\t{primary:?}\t{raw_sub:?}\t{admitted}\t{rejected}");
    }
    Ok(())
}

fn stock_ogg_frames(bytes: &[u8]) -> Result<u64> {
    // Walk physical pages, not a magic-string scan through compressed payload.
    let mut rest = bytes;
    let mut serial = None;
    let mut sequence = 0u32;
    let mut end = None;
    while !rest.is_empty() {
        let header = rest.get(..27).context("short Midori music Ogg page")?;
        ensure!(
            &header[..5] == b"OggS\0" && end.is_none(),
            "different Ogg stream framing"
        );
        let n = usize::from(header[26]);
        let lacing = rest
            .get(27..27 + n)
            .context("short Midori Ogg segment table")?;
        let payload = lacing.iter().map(|&v| usize::from(v)).sum::<usize>();
        let body = rest
            .get(27 + n..27 + n + payload)
            .context("short Midori Ogg payload")?;
        let stream = word(header, 14)?;
        ensure!(
            word(header, 18)? == sequence,
            "noncanonical Midori Ogg page order"
        );
        sequence += 1;
        if let Some(first) = serial {
            ensure!(
                stream == first && header[5] & 2 == 0,
                "chained Midori Ogg fixture"
            );
        } else {
            ensure!(
                header[5] & 2 != 0
                    && body.starts_with(b"\x01vorbis")
                    && word(body, 7)? == 0
                    && body.get(11) == Some(&2)
                    && word(body, 12)? == 44100,
                "not ordinary Midori stereo Vorbis"
            );
            serial = Some(stream);
        }
        if header[5] & 4 != 0 {
            end = Some(u64::from_le_bytes(header[6..14].try_into().unwrap()));
        }
        rest = &rest[27 + n + payload..];
    }
    end.filter(|&n| n != u64::MAX && n > 0)
        .context("missing Midori Ogg end granule")
}

#[test]
#[ignore = "requires the installed Midori 2011 ordinary audio asset; CPU-only"]
fn midori_stock_vorbis_crosses_authored_loop_boundary_offline() -> Result<()> {
    use crysirror_core::service::AudioPreparePhase;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let root = PathBuf::from(
        std::env::var_os("CRYSIRROR_EXHIBIT_MIDORI_ROOT")
            .context("set CRYSIRROR_EXHIBIT_MIDORI_ROOT for Midori offline audio")?,
    );
    let dir = std::env::temp_dir().join(format!(
        "exhibit-midori-bgm-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let asset = dir.join("assets/res/s/m");
    fs::create_dir_all(&asset)?;
    let verified = (|| -> Result<()> {
        fs::copy(root.join("res/s/m/m11.ogg"), asset.join("m11.ogg"))?;
        let path = dir.join("project.json");
        fs::write(&path, br#"{"schema":2,"name":"music-asset-only","content_revision":"1","canvas":[4,4],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
        let mut runtime = crysirror_runtime::Runtime::load(&path, &dir.join("state"))?;
        // This exercises the decoder and mixer only, not the app, ExHibit
        // dispatcher clock, any original game, or a physical audio device.
        runtime.set_audio_nonblocking(false);
        let c = ClientId(1);
        let mut regs = crate::native_state_tests::fresh(3)?;
        regs.set_system(30, 0)?;
        regs.set_system(31, 0x0100_0100)?;
        let mut music = midori_music::Music::default();
        let track = midori_music::Track::parse(b"11,0,144,144,1,0,0,32581,-1")?;
        assert!(music.play(&mut runtime.host, c, &regs, track, false)?);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            runtime.host.channel_audio.render(0)?;
            if music.poll(&mut runtime.host, c, &mut regs)? {
                break;
            }
            ensure!(Instant::now() < deadline, "Midori BGM open deadline");
            std::thread::yield_now();
        }
        let info = runtime
            .host
            .audio_info(c, 0)?
            .context("no Midori installed BGM")?;
        assert!(info.looping && info.source_rate == 44100);
        let frames = info.source_frames.context("no decoder frame count")?;
        assert_eq!(frames, 4_190_619);
        let seek = runtime.host.begin_audio_seek(c, 0, frames - 64)?;
        loop {
            let status = runtime.host.audio_seek_preparation(c, seek)?;
            match status.phase {
                AudioPreparePhase::Pending => {
                    ensure!(Instant::now() < deadline, "Midori BGM seek deadline");
                    std::thread::yield_now();
                }
                AudioPreparePhase::Ready => break,
                AudioPreparePhase::Failed => bail!("Midori BGM seek failed: {:?}", status.error),
            }
        }
        runtime.host.release_audio_seek(c, seek)?;
        runtime.host.channel_audio.render(5)?;
        let after = runtime
            .host
            .audio_info(c, 0)?
            .context("loop retired erroneously")?;
        assert!(after.first_pass_complete);
        let snapshot = midori_sound_capture::SoundCapture::from_owners(
            &music,
            &midori_ambient::Ambient::default(),
        )
        .context("missing settled stock music capture")?;
        assert_eq!(snapshot.normal, (61, 0x80_0000));
        assert!(snapshot.text.1.starts_with("5;0,11,2.144.144,65,0;"));
        assert!((track.loop_start..track.loop_start + 256).contains(&after.position_frames));
        assert!(
            runtime
                .host
                .channel_audio
                .take_output()
                .iter()
                .flatten()
                .any(|v| v.abs() > 0.0001)
        );
        assert!(!music.stop(&mut runtime.host, c, 0, false)?);
        assert!(!runtime.host.audio_playing(c, 0)?);
        println!(
            "MIDORI_MUSIC_LOOP frames={frames} restart={} observed={}",
            track.loop_start, after.position_frames
        );
        Ok(())
    })();
    fs::remove_dir_all(dir)?;
    verified
}

fn check_images(
    inputs: impl Iterator<Item = crysirror_core::image_processing::ImageInput>,
    filtered: (
        crysirror_core::image_processing::ImageInput,
        crysirror_core::image_processing::ImageInput,
    ),
    backgrounds: impl Iterator<Item = Result<crysirror_core::image_processing::ImageInput>>,
    marker: crysirror_core::image_processing::ImageInput,
    control_parts: Vec<crysirror_core::image_processing::ImageInput>,
) -> Result<(usize, usize, usize, usize)> {
    use crysirror_core::image_processing::{ImageInput, ImageProcessState};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let dir = std::env::temp_dir().join(format!(
        "exhibit-midori-cpu-pixels-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(dir.join("assets"))?;
    let verified = (|| -> Result<(usize, usize, usize, usize)> {
        // Tiny synthetic asset-only host: the real fixture inputs enter as
        // immutable bytes. No commercial scenario or device is launched.
        let path = dir.join("project.json");
        fs::write(&path, br#"{"schema":2,"name":"portrait-assets-only","content_revision":"1","canvas":[4,4],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
        let mut runtime = crysirror_runtime::Runtime::load(&path, &dir.join("state"))?;
        let meta = runtime.host.image_metadata(ClientId(1))?;
        let decode = |input| -> Result<_> {
            let mut job = meta.begin_process(input, Arc::from(&b"[]"[..]))?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match job.poll()? {
                    ImageProcessState::Pending => {
                        ensure!(
                            Instant::now() < deadline,
                            "Midori stock portrait CPU deadline"
                        );
                        std::thread::yield_now();
                    }
                    ImageProcessState::Ready(image) => return Ok(image),
                    ImageProcessState::Failed(e) => bail!("Midori stock portrait CPU image: {e}"),
                }
            }
        };
        let atlas = decode(marker)?;
        let rgba = atlas.rgba();
        ensure!(
            atlas.size() == [240, 30]
                && rgba.len() == 7200 * 4
                && rgba.chunks_exact(4).any(|p| p[3] == 0)
                && rgba.chunks_exact(4).any(|p| p[3] != 0),
            "wrong Midori marker atlas dimensions or alpha"
        );
        let mut previous = None;
        for input in control_parts {
            let part = decode(input)?;
            let rgba = part.rgba();
            ensure!(
                part.size() == [286, 38] && rgba.len() == 10868 * 4
                    && rgba.chunks_exact(4).any(|p| p[3] == 0)
                    && rgba.chunks_exact(4).any(|p| p[3] != 0)
                    && previous.as_ref() != Some(&rgba),
                "wrong Midori control part dimensions, alpha or duplicate pixels"
            );
            previous = Some(rgba);
        }
        let mut total_pixels = 0;
        let mut changed = false;
        let mut body_only = 0;
        for input in inputs {
            let ImageInput::Composite { base, layers } = &input else {
                // Native 0x386f0 does not even open F for a one-face body.
                let decoded = decode(input)?;
                let alpha = decoded.rgba();
                ensure!(
                    alpha.chunks_exact(4).any(|p| p[3] == 0)
                        && alpha.chunks_exact(4).any(|p| p[3] > 0),
                    "missing stock bare-body alpha"
                );
                body_only += 1;
                continue;
            };
            ensure!(layers.len() == 1, "expected one stock face replacement");
            let part = &layers[0];
            let body = decode(*base.clone())?;
            let atlas = decode(*part.input.clone())?;
            let composed = decode(input.clone())?;
            let [w, h] = body.size().map(|v| v as usize);
            let src_w = atlas.size()[0] as usize;
            let [src_x, src_y, cell_w, cell_h] = part.source.map(|v| v as usize);
            let [dx, dy] = part.destination.map(|v| v as usize);
            let a = body.rgba();
            let b = atlas.rgba();
            let actual = composed.rgba();
            ensure!(
                body.size() == composed.size() && actual.len() == a.len(),
                "portrait geometry changed"
            );
            for (i, pixel) in actual.chunks_exact(4).enumerate() {
                let (x, y) = (i % w, i / w);
                let base = &a[i * 4..i * 4 + 4];
                let rgb = if (dx..dx + cell_w).contains(&x) && (dy..dy + cell_h).contains(&y) {
                    let p = 4 * ((y - dy + src_y) * src_w + (x - dx + src_x));
                    &b[p..p + 3]
                } else {
                    &base[..3]
                };
                changed |= rgb != &base[..3];
                ensure!(
                    pixel[..3] == *rgb && pixel[3] == base[3],
                    "wrong body-alpha/face-index replacement at ({x},{y})"
                );
            }
            total_pixels += w * h;
        }
        ensure!(changed, "all stock face comparisons were vacuous");
        let stock = decode(filtered.0)?;
        let tinted = decode(filtered.1)?;
        ensure!(
            stock.size() == tinted.size(),
            "filtered stock portrait changed dimensions"
        );
        let before = stock.rgba();
        let after = tinted.rgba();
        let mut tinted_nontransparent = false;
        for (original, output) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
            let expected = std::array::from_fn::<_, 4, _>(|i| {
                if i == 3 {
                    return original[3];
                }
                let from = i32::from(original[i]);
                (from + (([0, 64, 32][i] - from) * 48).div_euclid(256)) as u8
            });
            ensure!(
                output == expected,
                "native filter mismatch on stock portrait RGBA"
            );
            tinted_nontransparent |= original[3] > 0 && original != output;
        }
        ensure!(
            tinted_nontransparent && before.len() == after.len(),
            "vacuous stock filter comparison"
        );
        let mut background_pixels = 0;
        let mut stock_pan_pixels = 0;
        for input in backgrounds {
            let input = input?;
            let ImageInput::Encoded { size, name, .. } = &input else {
                bail!("missing encoded background")
            };
            let size = *size;
            let stock_pan = name == "res/g/ev/19/1967.gyu";
            let scene = decode(input)?;
            ensure!(
                scene.size() == size && scene.opaque(),
                "decoded stock background is not an opaque full canvas"
            );
            background_pixels += scene.rgba().len() / 4;
            if stock_pan {
                let full = scene.rgba();
                for [x, y] in [[402usize, 567usize], [402, 168]] {
                    let mut job = midori_background::crop_job(
                        &mut runtime.host,
                        ClientId(1),
                        scene.clone(),
                        [x as i32, y as i32],
                    )?;
                    let deadline = Instant::now() + Duration::from_secs(10);
                    let cropped = loop {
                        match job.poll()? {
                            ImageProcessState::Ready(image) => break image,
                            ImageProcessState::Failed(e) => bail!("Midori stock pan crop: {e}"),
                            ImageProcessState::Pending => {
                                ensure!(
                                    Instant::now() < deadline,
                                    "Midori stock pan crop deadline"
                                );
                                std::thread::yield_now();
                            }
                        }
                    };
                    let bytes = cropped.rgba();
                    for row in 0..576 {
                        let at = ((y + row) * 2048 + x) * 4;
                        ensure!(
                            bytes[row * 4096..(row + 1) * 4096] == full[at..at + 4096],
                            "Midori stock crop differs from source row"
                        );
                    }
                    stock_pan_pixels += bytes.len() / 4;
                }
            }
        }
        ensure!(
            stock_pan_pixels == 2 * 1024 * 576,
            "missing pinned stock crop proof"
        );
        println!("MIDORI_STOCK_PAN exact_crop_pixels={stock_pan_pixels}");
        Ok((total_pixels, body_only, after.len() / 4, background_pixels))
    })();
    fs::remove_dir_all(dir)?;
    verified
}

#[test]
fn taken_2011_block_preflight_does_not_install_a_v1_checkpoint() -> Result<()> {
    let mut rld = b"\0DLR".to_vec();
    for word in [3u32, 12, 1] {
        rld.extend(word.to_le_bytes());
    }
    rld.extend([0x05, 0, 0, 0x21]);
    rld.extend_from_slice(b"-1,33554432,11,*,0,0,synthetic\0");
    let mut script = Script::new(rld, 3)?;
    let Action::Block(block) = script.next()? else {
        anyhow::bail!("expected 2011 block descriptor");
    };
    let mut regs = crate::native_state_tests::fresh(3)?;
    regs.set_normal(4, 0x97)?;
    regs.set_normal(7, 12)?;
    let before = regs.read(Value::Local(2))?;
    let mut flag = true;
    let result = script.visit_block(
        block,
        &mut regs,
        &mut flag,
        &[0; 19],
        &[0; 12],
        [false, false],
    );
    assert!(
        format!(
            "{:#}",
            result.expect_err("taken checkpoint is still unsupported")
        )
        .contains("camera, layer and printer checkpoint")
    );
    assert_eq!(script.block, 0);
    assert!(flag);
    assert_eq!(regs.read(Value::Local(2))?, before);
    assert_eq!(regs.read(Value::Local(4))?, 0x97);
    assert_eq!(regs.read(Value::Local(7))?, 12);
    assert!(regs.text(1)?.is_empty());
    Ok(())
}

/// These two READ-ONLY probes use the installed resident's own initialization
/// parameters. They decode definitions and keep the ordinary entry independent.
#[test]
#[ignore = "requires both installed late residents; static reads only"]
fn late_installed_definitions_and_entry_resources() -> Result<()> {
    for (variable, exe, expected, records) in [
        ("CRYSIRROR_EXHIBIT_VOL1_ROOT", "StraySheepVol1.exe", profile::Resident::LateVol1, 76),
        ("CRYSIRROR_EXHIBIT_VOL2_ROOT", "StraySheepVol2.exe", profile::Resident::LateVol2, 68),
    ] {
        let root = PathBuf::from(std::env::var_os(variable).with_context(|| format!("set {variable}"))?);
        let storage = ReadOnly(root);
        let seeds = profile::Seeds::read(&storage, &profile::Sources {
            exe: exe.into(), ini: "ExHIBIT.ini".into(), resident: "resident.dll".into()
        })?;
        assert_eq!(seeds.resident, expected);
        let mut definition = storage.read(&script_path("def")?)?;
        seeds.decode("def", &mut definition)?;
        let mut script = Script::new(definition, 3)?;
        script.resident = Some(expected);
        assert_eq!(script.remaining, records);
        let mut pc = script.pc;
        let mut counts = BTreeMap::new();
        let mut services = BTreeMap::new();
        let mut actions = BTreeMap::new();
        let mut executed = BTreeMap::new();
        let mut registers = crate::native_state_tests::fresh(3)?;
        let mut names = midori_state::EntryNames::default();
        let mut controls = midori_control_parts::Definitions::default();
        let mut draw_slots = midori_actor::Slots::default();
        while script.remaining != 0 {
            let (header, first, end) = branch::envelope(&script.bytes, pc)?;
            let op = header as u16;
            *counts.entry(op).or_insert(0) += 1;
            if op == 0x47 {
                let Action::DefineDebug(debug) = script.next()? else { bail!("expected packed debug definition") };
                assert_eq!(debug.mode, 0);
                assert_eq!(debug.pairs, [[-1, 0]; 8]);
            } else if op == 0x42 {
                let Action::DefineV3Dialog(_) = script.next()? else { bail!("expected installed dialog definition") };
            } else if op == 0x100 {
                let Action::DefineNotification(notification) = script.next()? else { bail!("expected installed notification definition") };
                controls.set_notification(notification);
            } else if op == 0x2c {
                let Action::DefineNotifications(notifications) = script.next()? else { bail!("expected installed notification bundle") };
                for notification in notifications {
                    controls.set_notification(notification);
                }
            } else if op == 0x40 {
                let Action::DefineV3Control(definition) = script.next()? else { bail!("expected installed local-control definition") };
                controls.set_control(*definition);
            } else if op == 0xfb {
                let Action::DefineMidoriActorSlot(definition) = script.next()? else { bail!("expected installed portrait pair") };
                draw_slots.define(definition)?;
            } else if op == 0x3d {
                let Action::Change(Change::MidoriDefineDb(db)) = script.next()
                    .with_context(|| format!("{expected:?} definition at {pc:#x}, header {header:#x}"))?
                else {
                    bail!("expected installed integer table definition");
                };
                let id = db.table;
                let (key, expected_value) = match id {
                    0 => (38, 36),
                    1 => (36, 38),
                    _ => bail!("unexpected installed integer table index {id}"),
                };
                assert_eq!(db.rows.len(), 43);
                registers.change(Change::MidoriDefineDb(db))?;
                let lookup = format!("{id},{key},0,1,1,2");
                registers.change(Change::MidoriGetDb(midori_state::DbLookup::parse(lookup.as_bytes())?))?;
                assert_eq!(registers.read(Value::Local(1))?, expected_value);
                assert_eq!(registers.read(Value::Local(2))?, 0);
            } else if matches!(op, 0x0b | 0xbf) {
                let selector = native_numbers::integer2011(first.context("missing selector")?
                    .split(|b| *b == b',').next().unwrap_or_default());
                let family = if op == 0x0b { &mut services } else { &mut actions };
                *family.entry(selector).or_insert(0) += 1;
                match (op, selector) {
                    (0x0b, 30) => {
                        let Action::MidoriEntryName(entry) = script.next()? else { bail!("expected entry name") };
                        names.set(entry);
                        *executed.entry((op, selector)).or_insert(0) += 1;
                    }
                    (0xbf, 342) => {
                        let Action::Change(change) = script.next()? else { bail!("expected ruby fade") };
                        registers.change(change)?;
                        assert_ne!(registers.read(Value::System(42))? & 16, 0);
                        *executed.entry((op, selector)).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
            if script.pc == pc { script.pc = end; script.remaining -= 1; }
            assert_eq!(script.pc, end);
            pc = end;
        }
        assert_eq!(script.bytes.len(), pc);
        assert!(names.count() > 0);
        assert_eq!(controls.notification_slots().keys().copied().collect::<Vec<_>>(), [1, 10, 11, 12, 13]);
        assert_eq!(controls.control_slots().len(), 13);
        assert_eq!(draw_slots.startup_layers(), [Some([144, 145]), Some([146, 147]), Some([148, 149])]);
        assert!(executed.contains_key(&(0xbf, 342)));
        println!("{expected:?} def records={records}, top={counts:x?}, services={services:?}, actions={actions:?}, dispatched in isolation={executed:?}");
        let mut start = storage.read(&script_path("start")?)?;
        // Both supplied launchers have neither RT_BITMAP/152 nor RCG/152.
        let before = start.clone();
        seeds.decode("start", &mut start)?;
        assert_eq!(before, start);
        let ordinary = Script::new(start, 3)?;
        assert!(word(&ordinary.bytes, ordinary.pc)? as u16 > 0x11e);

    }
    Ok(())
}
