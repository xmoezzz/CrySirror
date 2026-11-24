// SPDX-License-Identifier: MPL-2.0
// Nana name stream adapted from xmoezzz/xp3-brute
// 7b5fdeec318de4977787359399370927946e9795, src/cxdec_names.rs (MIT).
// Copyright (c) xp3-brute contributors. See NOTICE-MIT in this directory.
use super::{
    SetupData, Special,
    engine::{CONTROL_BLOCK_SIZE, CxdecProfile, GeneratorKind},
    plain_member, special,
};
use crate::common::{EntryPlan, ParsedArchive, bytes};
use anyhow::{Context, Result, ensure};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_resource::{DecodeContext, read_all};
use flate2::{Decompress, FlushDecompress, Status};
use serde::Deserialize;
use sha3::digest::{ExtendableOutput, Update, XofReader};
use siphasher::sip::SipHasher24;
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::Hasher,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    name_state: [u32; 6],
    content: Content,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    mask: u32,
    offset: u32,
    prolog_order: [u8; 3],
    even_branch_order: [u8; 8],
    odd_branch_order: [u8; 6],
    random_seed: u32,
    control_block_file: String,
}

pub(super) fn apply(
    archive: &mut ParsedArchive,
    descriptor: &Special,
    companion: &str,
    ctx: &DecodeContext,
) -> Result<(CxdecProfile, Option<SetupData>)> {
    ensure!(
        descriptor.tag == *b"dls:",
        "Nana profile requires a dls: descriptor"
    );
    let input = ctx
        .open_sibling(companion)
        .context("open local Nana profile")?;
    let profile: Profile =
        serde_json::from_slice(&read_all(input.as_ref(), 16 * 1024)?).map_err(|e| {
            anyhow::anyhow!(
                "invalid Nana profile at line {}, column {}",
                e.line(),
                e.column()
            )
        })?;
    let table = profile.content.control_block_file;
    let table_path = companion.rsplit_once('/').map_or_else(
        || table.clone(),
        |(directory, _)| format!("{directory}/{table}"),
    );
    let table_file = ctx
        .open_sibling(&table_path)
        .context("open Nana content table")?;
    let engine = CxdecProfile {
        mask: profile.content.mask,
        offset: profile.content.offset,
        prolog_order: profile.content.prolog_order,
        even_branch_order: profile.content.even_branch_order,
        odd_branch_order: profile.content.odd_branch_order,
        generator: GeneratorKind::Cabbage {
            random_seed: profile.content.random_seed,
        },
        control_block: read_all(table_file.as_ref(), CONTROL_BLOCK_SIZE as u64)?,
        prefix8: None,
    };
    engine.validate()?;

    ensure!(
        u64::from(descriptor.original) <= ctx.limits.max_index_size,
        "dls: decoded name table byte limit"
    );
    let mut stored = bytes(
        archive.source.as_ref(),
        descriptor.offset,
        u64::from(descriptor.stored),
        ctx.limits.max_index_size,
    )?;
    decode_prefix(&mut stored, profile.name_state);
    let size = descriptor.original as usize;
    let mut decoded = vec![0; size.checked_add(1).context("dls: decoded size overflow")?];
    let mut inflate = Decompress::new(true);
    let status = inflate
        .decompress(&stored, &mut decoded, FlushDecompress::Finish)
        .context("dls: name table zlib")?;
    ensure!(
        status == Status::StreamEnd
            && inflate.total_out() == u64::from(descriptor.original)
            && inflate.total_in() == u64::from(descriptor.stored),
        "dls: name table does not match its zlib descriptor"
    );
    decoded.truncate(size);
    let records = special::parse_name_records(&decoded, Some(b"hnfn"), archive.entries.len())?;
    let mut by_token = BTreeMap::new();
    let mut logical_paths = BTreeSet::new();
    for (adler, name) in records {
        let normalized = name.replace('\\', "/");
        let path = VfsPath::new(Mount::Asset, &normalized)?;
        ensure!(
            normalized == path.relative()
                && logical_paths.insert(path.relative().to_ascii_lowercase()),
            "invalid or duplicate dls: filename"
        );
        ensure!(
            by_token
                .insert(special::token("dls:", &name), (adler, normalized))
                .is_none(),
            "duplicate dls: filename token"
        );
    }
    let startup = if archive.entries.iter().any(|entry| entry.path == "$") {
        plain_member(archive, "$", ctx).context("validate the native dls: startup member")?;
        ctx.parameters["kirikiri/xp3"]["cxdec_nana_setup_file"]
            .as_str()
            .map(|path| validate_setup(ctx, path, &engine, profile.name_state))
            .transpose()?
    } else {
        None
    };
    for (entry, plan) in archive.entries.iter_mut().zip(&mut archive.plans) {
        let EntryPlan::Segments { facts, .. } = plan else {
            anyhow::bail!("dls: File entry has no segment plan");
        };
        let name = match entry.path.as_str() {
            // The native Nana name namespace aliases the literal $ entry to
            // startup.tjs. The ordinary same-named entry can be a decoy.
            "$" => "startup.tjs".into(),
            "startup.tjs" => continue,
            token => {
                let (adler, name) = by_token
                    .remove(&token.to_ascii_lowercase())
                    .with_context(|| format!("dls: File {} has no filename record", entry.id))?;
                ensure!(
                    facts.adler32 == adler,
                    "dls: filename Adler-32 mismatch for File {}",
                    entry.id
                );
                name
            }
        };
        let original = std::mem::replace(&mut entry.path, name);
        facts.info_name = Some(original.into_boxed_str());
    }
    ensure!(
        by_token.is_empty(),
        "dls: filename record has no File entry"
    );
    Ok((engine, startup))
}

fn validate_setup(
    ctx: &DecodeContext,
    path: &str,
    profile: &CxdecProfile,
    name_state: [u32; 6],
) -> Result<SetupData> {
    let input = ctx.open_sibling(path).context("open Nana startup input")?;
    let bytes = read_all(input.as_ref(), 16 * 1024)?;
    let text = crysirror_resource::text_stream::decode_auto(&bytes, 16 * 1024)?;
    // TJS Array.load recognizes CR, LF and CRLF. The script uses item zero.
    let line = text.split(['\r', '\n']).next().unwrap_or_default();
    let (control, mask, offset, state) = derive_setup(line)?;
    ensure!(
        control == profile.control_block.as_slice()
            && mask == profile.mask
            && offset == profile.offset
            && state == name_state,
        "Nana startup input does not derive the verified archive profile"
    );
    Ok(SetupData {
        text: line.to_owned(),
        startup_archive: true,
    })
}

/// Native Nana initializer in the inspected 32-bit DLsite module. This is
/// separate from the older ChaCha/BLAKE2s Special index derivation.
pub(super) fn derive_setup(text: &str) -> Result<([u8; CONTROL_BLOCK_SIZE], u32, u32, [u32; 6])> {
    let bytes = text
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    ensure!(
        !bytes.is_empty() && bytes.len() <= 8192,
        "Nana startup text length"
    );
    let mut shake = sha3::Shake256::default();
    shake.update(&bytes);
    let mut control = [0; CONTROL_BLOCK_SIZE];
    XofReader::read(&mut shake.finalize_xof(), &mut control);
    let mut sip = SipHasher24::new_with_key(control[..16].try_into().unwrap());
    sip.write(&bytes);
    let hash = sip.finish();
    let digest = [hash as u32, (hash >> 32) as u32];
    let mask = (digest[0] & 0x1ff) + 0x100;
    let offset = ((digest[0] >> 9) & 0x7f) + ((digest[0] >> 16) & 0xff) + 0x20;
    let name_state = std::array::from_fn(|i| {
        let word = u32::from_le_bytes(control[i * 4..i * 4 + 4].try_into().unwrap());
        word ^ digest[i % 2]
    });
    Ok((control, mask, offset, name_state))
}

pub(super) fn decode_prefix(data: &mut [u8], state: [u32; 6]) {
    let [mut seed, a, b, c, counter_low, counter_high] = state;
    let mut lanes = [a, b, c];
    let mut schedule = [seed; 27];
    for (round, word) in schedule[1..].iter_mut().enumerate() {
        let lane = &mut lanes[round % 3];
        *lane = round as u32 ^ lane.rotate_right(8).wrapping_add(seed);
        seed = seed.rotate_left(3) ^ *lane;
        *word = seed;
    }
    let len = data.len().min(256);
    for (block, chunk) in data[..len].chunks_mut(8).enumerate() {
        let mut lo = (block as u32 + 1) ^ counter_low;
        let mut hi = counter_high;
        for word in schedule {
            hi = hi.rotate_right(8).wrapping_add(lo) ^ word;
            lo = lo.rotate_left(3) ^ hi;
        }
        for (byte, key) in chunk
            .iter_mut()
            .zip(lo.to_le_bytes().into_iter().chain(hi.to_le_bytes()))
        {
            *byte ^= key;
        }
    }
}
