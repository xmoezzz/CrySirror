// Portions adapted from xmoezzz/xp3-brute, revision c82cb14302138125640478556723705aa720007b.
// Copyright (c) xp3-brute contributors; modifications CrySirror contributors.
// MIT License: see NOTICE-MIT in this directory.
//! Static CXDEC parameter recovery from the user's own PE modules (upstream
//! `src/legacy_cxdec.rs`). Every recognizer reads code/data structurally;
//! nothing is executed. Recovered values are only *candidates*: the archive
//! layer accepts a profile solely after it restores real entries' `adlr`.
//!
//! Not ported: upstream's Unicorn-based DLL-initialization snapshot fallback
//! and SteamStub unwrapping. A module needing either yields no candidate and
//! the archive open fails visibly.
use super::engine::{CxdecProfile, GeneratorKind, Prefix8, CONTROL_BLOCK_SIZE};
use super::pe::*;
use std::collections::{BTreeMap, BTreeSet};

const CONTROL_SIGNATURE: &[u8] = b" Encryption control block";
const LCG_MUL: u32 = 0x41c6_4e6d;
const LCG_ADD: u32 = 0x3039;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub prolog: [u8; 3],
    pub even: [u8; 8],
    pub odd: [u8; 6],
    pub distance: u32,
}

/// Facts recovered from one PE image (disk, embedded or statically decoded).
#[derive(Clone, Debug, Default)]
pub struct ModuleFacts {
    pub label: String,
    pub mask_offset: Option<(u32, u32)>,
    pub control_blocks: Vec<Vec<u8>>,
    pub dispatch: Vec<Dispatch>,
    pub classic: bool,
    pub cabbage: bool,
    pub random_seeds: Vec<u32>,
    pub prefix8: Vec<Prefix8>,
    pub setup_generator: bool,
    pub startup_redirect: Option<(String, String)>,
    pub v2link: bool,
}

/// Analyze a module, any embedded `internal module` images and a statically
/// self-decoded image when the on-disk generator is encoded.
pub fn analyze(label: &str, raw: &[u8]) -> Vec<ModuleFacts> {
    let mut out = Vec::new();
    let mut images = vec![(label.to_owned(), raw.to_vec())];
    for (at, bytes) in embedded_modules(raw) {
        images.push((format!("{label}::internal@0x{at:x}"), bytes));
    }
    for (label, bytes) in images {
        let Some(fact) = collect(&label, &bytes) else {
            continue;
        };
        let needs = needs_self_decode(&fact);
        out.push(fact);
        if needs {
            if let Some((decoded, _seed)) = static_self_decode(&bytes) {
                if let Some(d) = collect(&format!("{label}[self-decode:bitperm-v1]"), &decoded) {
                    let base = out.last().unwrap();
                    if improves(base, &d) {
                        out.push(d);
                    }
                }
            }
        }
    }
    out
}

fn needs_self_decode(f: &ModuleFacts) -> bool {
    f.v2link
        && !f.control_blocks.is_empty()
        && (!f.classic || f.mask_offset.is_none() || f.dispatch.is_empty())
}
fn improves(a: &ModuleFacts, b: &ModuleFacts) -> bool {
    (!a.classic && b.classic)
        || (a.mask_offset.is_none() && b.mask_offset.is_some())
        || (a.dispatch.is_empty() && !b.dispatch.is_empty())
        || (!a.cabbage && b.cabbage)
        || (a.prefix8.is_empty() && !b.prefix8.is_empty())
}

pub fn collect(label: &str, bytes: &[u8]) -> Option<ModuleFacts> {
    let pe = PeImage::parse(bytes).ok()?;
    if !pe.i386() {
        return None;
    }
    let callback = find_callback_config(bytes, &pe);
    let dynamic = find_dynamic_xcode(bytes, &pe);
    let classic = contains_u32(bytes, LCG_MUL) && contains_u32(bytes, LCG_ADD);
    let cabbage_rva = find_cabbage_prng_window(bytes, &pe);
    let prefix8_rva = find_prefix8_window(bytes, &pe);
    let control_blocks: Vec<Vec<u8>> = control_block_offsets(
        bytes,
        &pe,
        callback.as_ref(),
        dynamic,
        cabbage_rva,
        prefix8_rva,
    )
    .into_iter()
    .filter_map(|off| pe.file_offset_to_rva(off))
    .filter_map(|rva| pe.slice_rva(bytes, rva, CONTROL_BLOCK_SIZE))
    .map(<[u8]>::to_vec)
    .collect();
    let boundary = find_boundary_params(bytes, &pe).or_else(|| {
        (classic && !control_blocks.is_empty())
            .then(|| find_direct_boundary_params(bytes, &pe))
            .flatten()
    });
    let builder = callback
        .as_ref()
        .map(|c| c.builder_rva)
        .or(dynamic.map(|d| d.builder_rva));
    let dispatch = builder
        .or(cabbage_rva)
        .map(|anchor| find_dispatch_candidates(bytes, &pe, anchor))
        .unwrap_or_default();
    let random_seeds = cabbage_rva
        .map(|p| find_cabbage_seed_candidates(bytes, &pe, p, builder))
        .unwrap_or_default();
    let prefix8 = if prefix8_rva.is_some() {
        let found = recover_prefix8_constants(bytes, &pe);
        // The wrapper is proven but its key expansion was not decoded:
        // fall back to the algorithm default and let validation decide.
        if found.is_empty() {
            vec![Prefix8::DEFAULT]
        } else {
            found
        }
    } else {
        Vec::new()
    };
    if callback.is_none()
        && dynamic.is_none()
        && boundary.is_none()
        && control_blocks.is_empty()
        && dispatch.is_empty()
        && !classic
        && cabbage_rva.is_none()
        && prefix8_rva.is_none()
    {
        return None;
    }
    Some(ModuleFacts {
        label: label.to_owned(),
        mask_offset: callback.map(|c| (c.key0, c.key1)).or(boundary),
        control_blocks,
        dispatch,
        classic,
        cabbage: cabbage_rva.is_some(),
        random_seeds,
        prefix8,
        setup_generator: super::special::has_setup_generator(bytes),
        startup_redirect: super::special::startup_redirect(bytes),
        v2link: contains(bytes, b"V2Link\0"),
    })
}

/// Profiles whose generator grammar, Cabbage seed and Prefix8 wrapper all
/// come from one module tied to `setupArchiveData`; archive-specific
/// mask/offset/control come from the validated script-derived values.
pub fn coherent_candidates(
    facts: &[ModuleFacts],
    control: &[u8],
    mask_offset: (u32, u32),
) -> Vec<(CxdecProfile, String)> {
    let mut out = Vec::new();
    for f in facts {
        if !f.setup_generator
            || !f.cabbage
            || f.prefix8.is_empty()
            || f.dispatch.is_empty()
            || f.random_seeds.is_empty()
        {
            continue;
        }
        for d in &f.dispatch {
            for &seed in f.random_seeds.iter().take(8) {
                for &p in &f.prefix8 {
                    out.push((
                        CxdecProfile {
                            mask: mask_offset.0,
                            offset: mask_offset.1,
                            prolog_order: d.prolog,
                            even_branch_order: d.even,
                            odd_branch_order: d.odd,
                            control_block: control.to_vec(),
                            generator: GeneratorKind::Cabbage { random_seed: seed },
                            prefix8: Some(p),
                        },
                        f.label.clone(),
                    ));
                }
            }
        }
    }
    dedup(out)
}

/// Bounded cross-module combination for modules without the coherent
/// anchors. Each fact keeps its provenance; validation selects one profile.
pub fn generic_candidates(facts: &[ModuleFacts]) -> Vec<(CxdecProfile, String)> {
    const MAX: usize = 4096;
    let mask_offsets: BTreeMap<(u32, u32), &str> = facts
        .iter()
        .rev()
        .filter_map(|f| f.mask_offset.map(|m| (m, f.label.as_str())))
        .collect();
    let mut controls: BTreeMap<&[u8], &str> = BTreeMap::new();
    for f in facts.iter().rev() {
        for c in &f.control_blocks {
            controls.insert(c, &f.label);
        }
    }
    let mut dispatches: BTreeMap<([u8; 3], [u8; 8], [u8; 6]), (u32, &str)> = BTreeMap::new();
    for f in facts {
        for d in &f.dispatch {
            let e = dispatches
                .entry((d.prolog, d.even, d.odd))
                .or_insert((d.distance, &f.label));
            if d.distance < e.0 {
                *e = (d.distance, &f.label);
            }
        }
    }
    let mut dispatches: Vec<_> = dispatches.into_iter().collect();
    dispatches.sort_by_key(|(_, (d, l))| (*d, *l));
    let classic = facts.iter().any(|f| f.classic);
    let cabbage = facts.iter().any(|f| f.cabbage);
    let mut generators = Vec::new();
    if classic {
        generators.push(GeneratorKind::Classic);
    }
    if cabbage {
        let seeds: BTreeSet<u32> = facts
            .iter()
            .flat_map(|f| f.random_seeds.iter().copied())
            .collect();
        generators.extend(
            seeds
                .into_iter()
                .map(|random_seed| GeneratorKind::Cabbage { random_seed }),
        );
    }
    let mut wrappers: Vec<Option<Prefix8>> = vec![None];
    let p8: BTreeSet<Prefix8> = facts
        .iter()
        .flat_map(|f| f.prefix8.iter().copied())
        .collect();
    wrappers.extend(p8.into_iter().map(Some));
    if mask_offsets.is_empty()
        || controls.is_empty()
        || dispatches.is_empty()
        || generators.is_empty()
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    'outer: for (&(mask, offset), ml) in &mask_offsets {
        for (&control, cl) in &controls {
            for ((prolog, even, odd), (_, dl)) in &dispatches {
                for &generator in &generators {
                    for &prefix8 in &wrappers {
                        out.push((
                            CxdecProfile {
                                mask,
                                offset,
                                prolog_order: *prolog,
                                even_branch_order: *even,
                                odd_branch_order: *odd,
                                control_block: control.to_vec(),
                                generator,
                                prefix8,
                            },
                            format!("mask_offset={ml} control={cl} dispatch={dl}"),
                        ));
                        if out.len() >= MAX {
                            break 'outer;
                        }
                    }
                }
            }
        }
    }
    dedup(out)
}

fn dedup(mut v: Vec<(CxdecProfile, String)>) -> Vec<(CxdecProfile, String)> {
    let mut seen = BTreeSet::new();
    v.retain(|(p, _)| {
        p.validate().is_ok()
            && seen.insert((
                p.mask,
                p.offset,
                p.prolog_order,
                p.even_branch_order,
                p.odd_branch_order,
                p.control_block.clone(),
                p.generator,
                p.prefix8,
            ))
    });
    v
}

// ---------------------------------------------------------------- callback

struct CallbackConfig {
    key0: u32,
    key1: u32,
    builder_rva: u32,
}

/// `{ const char *name; u32 key[2]; builder }` referenced from code (or with
/// its builder inside `.decc`). Keys are title parameters, never constants.
fn find_callback_config(b: &[u8], pe: &PeImage) -> Option<CallbackConfig> {
    // Pass 1: structurally plausible descriptors.
    let mut found = Vec::new();
    for s in &pe.sections {
        if s.raw_size < 16 {
            continue;
        }
        let (start, end) = s.file_range(b.len());
        let mut off = start;
        while off + 16 <= end {
            let at = off;
            off += 4;
            let (name_va, key0, key1, builder_va) = (
                u32_opt(b, at)?,
                u32_opt(b, at + 4)?,
                u32_opt(b, at + 8)?,
                u32_opt(b, at + 12)?,
            );
            if key0 == 0 || key1 == 0 || key0 > 0x00ff_ffff || key1 > 0x00ff_ffff {
                continue;
            }
            let Some(builder_rva) = builder_va.checked_sub(pe.image_base) else {
                continue;
            };
            if !pe.is_executable_rva(builder_rva) {
                continue;
            }
            let Some(rva) = pe.file_offset_to_rva(at) else {
                continue;
            };
            let name_score = if name_va == 0 {
                1
            } else if pe
                .va_to_file_offset(name_va)
                .and_then(|o| c_string(b, o, 96))
                .is_some()
            {
                3
            } else {
                continue;
            };
            let in_decc = pe
                .section_for_rva(builder_rva)
                .is_some_and(|s| s.name.eq_ignore_ascii_case(".decc"));
            found.push((
                pe.image_base.wrapping_add(rva),
                name_score,
                in_decc,
                key0,
                key1,
                builder_rva,
            ));
        }
    }
    // Pass 2: one scan of executable bytes counts code references.
    let wanted: BTreeSet<u32> = found.iter().map(|f| f.0).collect();
    let mut xrefs: BTreeMap<u32, usize> = BTreeMap::new();
    if !wanted.is_empty() {
        for (s, e) in pe.exec_ranges(b.len()) {
            for w in b[s..e].windows(4) {
                let v = u32::from_le_bytes(w.try_into().unwrap());
                if wanted.contains(&v) {
                    *xrefs.entry(v).or_default() += 1;
                }
            }
        }
    }
    let mut best: Option<(u32, CallbackConfig)> = None;
    for (va, name_score, in_decc, key0, key1, builder_rva) in found {
        let refs = xrefs.get(&va).copied().unwrap_or(0);
        if refs == 0 && !in_decc {
            continue;
        }
        let score = 20 + refs.min(8) as u32 * 4 + name_score + if in_decc { 12 } else { 0 };
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((
                score,
                CallbackConfig {
                    key0,
                    key1,
                    builder_rva,
                },
            ));
        }
    }
    best.map(|(_, c)| c)
}
fn c_string(b: &[u8], off: usize, max: usize) -> Option<&str> {
    let tail = b.get(off..off.saturating_add(max).min(b.len()))?;
    let end = tail.iter().position(|&c| c == 0)?;
    let raw = &tail[..end];
    raw.iter()
        .all(|&c| c == b'\t' || (0x20..=0x7e).contains(&c))
        .then(|| std::str::from_utf8(raw).ok())
        .flatten()
}

// ------------------------------------------------------- dynamic manager

#[derive(Clone, Copy)]
struct DynamicXcode {
    manager_rva: u32,
    builder_rva: u32,
    control_rva: u32,
}

/// 128-lane runtime manager: `lane = key & 0x7f`, `seed = key >> 7`, a
/// bounded 0x80-byte builder call, the lane invoked for `seed` and `!seed`,
/// and a `mov eax, table; ret` getter for the control table.
fn find_dynamic_xcode(b: &[u8], pe: &PeImage) -> Option<DynamicXcode> {
    if !contains_u32(b, LCG_MUL) || !contains_u32(b, LCG_ADD) {
        return None;
    }
    let mut found: Option<DynamicXcode> = None;
    for (start, end) in pe.exec_ranges(b.len()).collect::<Vec<_>>() {
        if end <= start + 32 {
            continue;
        }
        for and_off in start..end.saturating_sub(3) {
            if b[and_off] != 0x83 || b[and_off + 2] != 0x7f || b[and_off + 1] & 0xf8 != 0xe0 {
                continue;
            }
            let lane_reg = b[and_off + 1] & 7;
            let split_end = and_off.saturating_add(24).min(end);
            let Some(seed_reg) = (and_off + 3..split_end.saturating_sub(2))
                .find(|&o| b[o] == 0xc1 && b[o + 1] & 0xf8 == 0xe8 && b[o + 2] == 7)
                .map(|o| b[o + 1] & 7)
            else {
                continue;
            };
            let ws = and_off.saturating_sub(32).max(start);
            let we = and_off.saturating_add(0x120).min(end);
            let mut push80 = ws;
            while push80 + 5 <= we {
                if b[push80..push80 + 5] != [0x68, 0x80, 0, 0, 0] {
                    push80 += 1;
                    continue;
                }
                let call_end = push80.saturating_add(24).min(we);
                for call in push80 + 5..call_end.saturating_sub(4) {
                    if b[call] != 0xe8 || !b[push80 + 5..call].contains(&(0x50 + lane_reg)) {
                        continue;
                    }
                    let Some(builder_rva) = rel32_target(b, pe, call) else {
                        continue;
                    };
                    if !pe.is_executable_rva(builder_rva)
                        || !seed_and_complement_calls(b, call + 5, we, seed_reg)
                    {
                        continue;
                    }
                    let back = push80.saturating_sub(48).max(ws);
                    for getter in (back..push80).rev() {
                        if b[getter] != 0xe8 {
                            continue;
                        }
                        let Some(control_va) = rel32_target(b, pe, getter)
                            .and_then(|r| pe.rva_to_file_offset(r))
                            .and_then(|o| tiny_pointer_return(b, o))
                        else {
                            continue;
                        };
                        let Some(control_rva) = control_va.checked_sub(pe.image_base) else {
                            continue;
                        };
                        if !pe
                            .slice_rva(b, control_rva, CONTROL_BLOCK_SIZE)
                            .is_some_and(looks_like_control_block)
                        {
                            continue;
                        }
                        let c = DynamicXcode {
                            manager_rva: pe.file_offset_to_rva(and_off)?,
                            builder_rva,
                            control_rva,
                        };
                        match found {
                            None => found = Some(c),
                            Some(p)
                                if p.builder_rva == c.builder_rva
                                    && p.control_rva == c.control_rva => {}
                            Some(_) => return None,
                        }
                    }
                }
                push80 += 1;
            }
        }
    }
    found
}
fn seed_and_complement_calls(b: &[u8], start: usize, end: usize, reg: u8) -> bool {
    let end = end.min(b.len());
    let (push, not) = (0x50 + reg, 0xd0 + reg);
    let indirect = |p: usize| p + 1 < end && b[p] == 0xff && (b[p + 1] >> 3) & 7 == 2;
    let Some(first) = (start..end.saturating_sub(1))
        .find(|&o| b[o] == push && (o + 1..o.saturating_add(8).min(end)).any(indirect))
    else {
        return false;
    };
    (first + 1..end.saturating_sub(2)).any(|o| {
        b[o] == 0xf7
            && b[o + 1] == not
            && (o + 2..(o + 14).min(end))
                .any(|p| b[p] == push && (p + 1..p.saturating_add(8).min(end)).any(indirect))
    })
}
fn rel32_target(b: &[u8], pe: &PeImage, call: usize) -> Option<u32> {
    if b.get(call) != Some(&0xe8) {
        return None;
    }
    let rva = pe.file_offset_to_rva(call)?;
    let rel = i32::from_le_bytes(b.get(call + 1..call + 5)?.try_into().ok()?);
    u32::try_from(i64::from(rva) + 5 + i64::from(rel)).ok()
}
fn tiny_pointer_return(b: &[u8], off: usize) -> Option<u32> {
    (b.get(off) == Some(&0xb8) && b.get(off + 5) == Some(&0xc3))
        .then(|| u32_opt(b, off + 1))
        .flatten()
}

// ---------------------------------------------------------- control table

fn control_block_offsets(
    b: &[u8],
    pe: &PeImage,
    callback: Option<&CallbackConfig>,
    dynamic: Option<DynamicXcode>,
    cabbage: Option<u32>,
    prefix8: Option<u32>,
) -> Vec<usize> {
    let mut out = BTreeSet::new();
    if b.len() >= CONTROL_BLOCK_SIZE {
        if let Some(o) = (0..=b.len() - CONTROL_BLOCK_SIZE)
            .step_by(4)
            .find(|&o| b[o..].starts_with(CONTROL_SIGNATURE))
        {
            out.insert(o);
        }
    }
    if let Some(o) = dynamic.and_then(|d| pe.rva_to_file_offset(d.control_rva)) {
        out.insert(o);
    }
    let mut anchors: Vec<u32> = callback.map(|c| c.builder_rva).into_iter().collect();
    if let Some(d) = dynamic {
        anchors.extend([d.manager_rva, d.builder_rva]);
    }
    anchors.extend(cabbage);
    anchors.extend(prefix8);
    anchors.sort_unstable();
    anchors.dedup();
    for a in anchors {
        let Some(s) = pe.section_for_rva(a).filter(|s| s.executable()) else {
            continue;
        };
        let Some(af) = pe.rva_to_file_offset(a) else {
            continue;
        };
        let (ss, se) = s.file_range(b.len());
        scan_control_pointers(
            b,
            pe,
            af.saturating_sub(0x8000).max(ss),
            af.saturating_add(0x8000).min(se),
            &mut out,
        );
    }
    out.into_iter().collect()
}
fn scan_control_pointers(
    b: &[u8],
    pe: &PeImage,
    start: usize,
    end: usize,
    out: &mut BTreeSet<usize>,
) {
    let mut consider = |v: u32| {
        consider_pointer(b, pe, v, out);
        if let Some(indirect) = pe.va_to_file_offset(v).and_then(|o| u32_opt(b, o)) {
            consider_pointer(b, pe, indirect, out);
        }
    };
    let mut i = start;
    while i < end {
        match b[i] {
            0x68 | 0xa1 | 0xa3 | 0xb8..=0xbf if i + 5 <= end => {
                consider(u32_opt(b, i + 1).unwrap());
                i += 5;
            }
            0x8b | 0x8d if i + 6 <= end && b[i + 1] & 0xc7 == 0x05 => {
                consider(u32_opt(b, i + 2).unwrap());
                i += 6;
            }
            0xc7 if i + 10 <= end && b[i + 1] == 0x05 => {
                consider(u32_opt(b, i + 2).unwrap());
                consider(u32_opt(b, i + 6).unwrap());
                i += 10;
            }
            _ => i += 1,
        }
    }
}
fn consider_pointer(b: &[u8], pe: &PeImage, va: u32, out: &mut BTreeSet<usize>) {
    let Some(s) = va
        .checked_sub(pe.image_base)
        .and_then(|r| pe.section_for_rva(r))
    else {
        return;
    };
    if s.executable() {
        return;
    }
    let Some(off) = pe.va_to_file_offset(va) else {
        return;
    };
    let end = off + CONTROL_BLOCK_SIZE;
    if end <= s.file_range(b.len()).1 && looks_like_control_block(&b[off..end]) {
        out.insert(off);
    }
}
fn looks_like_control_block(block: &[u8]) -> bool {
    if block.len() != CONTROL_BLOCK_SIZE {
        return false;
    }
    let mut unique = BTreeSet::new();
    let (mut zero, mut ff) = (0, 0);
    for c in block.chunks_exact(4) {
        let w = u32::from_le_bytes(c.try_into().unwrap());
        unique.insert(w);
        zero += usize::from(w == 0);
        ff += usize::from(w == u32::MAX);
    }
    unique.len() >= 512 && zero <= 64 && ff <= 64
}

// --------------------------------------------------------------- boundary

/// `mov [obj+4], r; and r, mask; add r, offset; mov [obj+8], r`.
fn find_boundary_params(b: &[u8], pe: &PeImage) -> Option<(u32, u32)> {
    boundary_scan(b, pe, 24, |b, start, and_off, add_off, end, reg| {
        let before = and_off.saturating_sub(16).max(start);
        let base = mov_store_disp(b, before, and_off, reg, 4)?;
        (mov_store_disp(b, add_off + 6, add_off.saturating_add(18).min(end), reg, 8) == Some(base))
            .then_some(())
    })
}
/// In-place `(hash & mask) + offset`; only consulted after classic generator
/// and control-table evidence exist in the same module.
fn find_direct_boundary_params(b: &[u8], pe: &PeImage) -> Option<(u32, u32)> {
    boundary_scan(b, pe, 40, |_, _, _, _, _, _| Some(()))
}
fn boundary_scan(
    b: &[u8],
    pe: &PeImage,
    reach: usize,
    accept: impl Fn(&[u8], usize, usize, usize, usize, u8) -> Option<()>,
) -> Option<(u32, u32)> {
    let mut found: Option<(usize, u32, u32)> = None;
    for (start, end) in pe.exec_ranges(b.len()).collect::<Vec<_>>() {
        if end <= start + 16 {
            continue;
        }
        for and_off in start..end.saturating_sub(12) {
            if b[and_off] != 0x81 || b[and_off + 1] & 0xf8 != 0xe0 {
                continue;
            }
            let reg = b[and_off + 1] & 7;
            let mask = u32_opt(b, and_off + 2)?;
            if mask == 0 || mask > 0x00ff_ffff {
                continue;
            }
            let search = and_off.saturating_add(reach).min(end);
            for add_off in and_off + 6..search.saturating_sub(5) {
                if b[add_off] != 0x81 || b[add_off + 1] != 0xc0 | reg {
                    continue;
                }
                let offset = u32_opt(b, add_off + 2)?;
                if offset == 0
                    || offset > 0x00ff_ffff
                    || accept(b, start, and_off, add_off, end, reg).is_none()
                {
                    continue;
                }
                match found {
                    None => found = Some((and_off, mask, offset)),
                    Some((_, m, o)) if m == mask && o == offset => {}
                    Some(_) => return None,
                }
            }
        }
    }
    found.map(|(_, m, o)| (m, o))
}
fn mov_store_disp(b: &[u8], start: usize, end: usize, reg: u8, disp: u8) -> Option<u8> {
    let end = end.min(b.len());
    (start..end.saturating_sub(2)).find_map(|o| {
        let m = b[o + 1];
        (b[o] == 0x89 && m >> 6 == 1 && (m >> 3) & 7 == reg && m & 7 != 4 && b[o + 2] == disp)
            .then_some(m & 7)
    })
}

// --------------------------------------------------------- Cabbage/Prefix8

fn has_shift(w: &[u8], left: bool, n: u8) -> bool {
    let g = if left { 4 } else { 5 };
    w.windows(3)
        .any(|i| i[0] == 0xc1 && i[1] & 0xc0 == 0xc0 && (i[1] >> 3) & 7 == g && i[2] == n)
}
fn xor_count(w: &[u8]) -> usize {
    w.windows(2)
        .filter(|i| matches!(i[0], 0x31 | 0x33) && i[1] & 0xc0 == 0xc0)
        .count()
}
fn has_not(w: &[u8]) -> bool {
    w.windows(2)
        .any(|i| i[0] == 0xf7 && i[1] & 0xc0 == 0xc0 && (i[1] >> 3) & 7 == 2)
}
fn has_cmp8(w: &[u8], v: u8) -> bool {
    w.windows(3)
        .any(|i| i[0] == 0x83 && i[1] & 0xc0 == 0xc0 && (i[1] >> 3) & 7 == 7 && i[2] == v)
}

/// Both CxProgramNana xorshift transitions in one compact code window.
fn find_cabbage_prng_window(b: &[u8], pe: &PeImage) -> Option<u32> {
    for (start, end) in pe.exec_ranges(b.len()) {
        for base in (start..end).step_by(128) {
            let w = &b[base..base.saturating_add(512).min(end)];
            if has_shift(w, true, 17)
                && has_shift(w, true, 18)
                && has_shift(w, false, 15)
                && has_shift(w, true, 13)
                && has_shift(w, false, 17)
                && has_shift(w, true, 5)
                && xor_count(w) >= 5
                && has_not(w)
            {
                return pe.file_offset_to_rva(base);
            }
        }
    }
    None
}

/// Hash-derived first-eight-byte wrapper: either the key expansion next to
/// its apply loop, or the object form XORing from `object+0x10+offset`.
fn find_prefix8_window(b: &[u8], pe: &PeImage) -> Option<u32> {
    for (start, end) in pe.exec_ranges(b.len()) {
        for base in (start..end).step_by(32) {
            let w = &b[base..base.saturating_add(256).min(end)];
            let old = contains_u32(w, 0x5555_5555)
                && contains_u32(w, 0xaaaa_aaaa)
                && has_shift(w, true, 13)
                && has_shift(w, false, 17)
                && has_shift(w, true, 5)
                && has_cmp8(w, 8)
                && xor_count(w) >= 4;
            let object = has_cmp8(w, 8)
                && w.windows(4)
                    .any(|i| i[0] == 0x8d && i[1] & 0xc7 == 0x44 && i[3] == 0x10)
                && w.windows(2).any(|i| matches!(i[0], 0x30 | 0x32));
            if old || object {
                return pe.file_offset_to_rva(base);
            }
        }
    }
    None
}

/// CrySirror extension: recover the two Prefix8 XOR words. The key
/// expansion is `t = h ^ h<<13; t ^= t>>17; hi = t ^ t<<5 ^ HI; lo = h ^ LO`.
/// `HI`/`LO` are either immediates or absolute globals; a global resolves to
/// its file-backed initial value and to any `.rdata` value a static
/// `mov r,[src]; mov [global],r` copies into it. Both role assignments are
/// returned; real-entry validation picks the correct pair.
fn recover_prefix8_constants(b: &[u8], pe: &PeImage) -> Vec<Prefix8> {
    let mut out = BTreeSet::new();
    for (start, end) in pe.exec_ranges(b.len()).collect::<Vec<_>>() {
        for shl5 in start..end.saturating_sub(3) {
            if b[shl5] != 0xc1 || b[shl5 + 1] & 0xf8 != 0xe0 || b[shl5 + 2] != 5 {
                continue;
            }
            let lo_win = shl5.saturating_sub(64).max(start);
            let pre = &b[lo_win..shl5];
            if !has_shift(pre, true, 13) || !has_shift(pre, false, 17) {
                continue;
            }
            let reg = b[shl5 + 1] & 7;
            let hi_end = (shl5 + 24).min(end);
            let mut his = Vec::new();
            let mut others = Vec::new();
            let mut i = lo_win;
            let stop = (shl5 + 40).min(end);
            while i < stop {
                if let Some((r, len, operand)) = xor_constant(b, i) {
                    let values = resolve_operand(b, pe, operand);
                    if r == reg && i > shl5 && i < hi_end && his.is_empty() {
                        his = values;
                    } else {
                        others.extend(values);
                    }
                    i += len;
                } else {
                    i += 1;
                }
            }
            for &hi in &his {
                for &lo in &others {
                    out.insert(Prefix8 { lo, hi });
                    out.insert(Prefix8 { lo: hi, hi: lo });
                }
            }
        }
    }
    out.into_iter().collect()
}
enum Operand {
    Imm(u32),
    Abs(u32),
}
/// `xor r32, imm32` (81 /6) or `xor r32, [abs32]` (33 /r, mod 00 rm 101).
fn xor_constant(b: &[u8], i: usize) -> Option<(u8, usize, Operand)> {
    let m = *b.get(i + 1)?;
    match b[i] {
        0x81 if m & 0xf8 == 0xf0 => Some((m & 7, 6, Operand::Imm(u32_opt(b, i + 2)?))),
        0x33 if m & 0xc7 == 0x05 => Some(((m >> 3) & 7, 6, Operand::Abs(u32_opt(b, i + 2)?))),
        _ => None,
    }
}
fn resolve_operand(b: &[u8], pe: &PeImage, op: Operand) -> Vec<u32> {
    let va = match op {
        Operand::Imm(v) => return vec![v],
        Operand::Abs(va) => va,
    };
    let mut values = BTreeSet::new();
    if let Some(v) = pe.va_to_file_offset(va).and_then(|o| u32_opt(b, o)) {
        values.insert(v);
    }
    let target = va.to_le_bytes();
    for (start, end) in pe.exec_ranges(b.len()).collect::<Vec<_>>() {
        for s in start..end.saturating_sub(6) {
            let reg = match (b[s], b[s + 1]) {
                (0xa3, _) if b[s + 1..s + 5] == target => 0,
                (0x89, m) if m & 0xc7 == 0x05 && b[s + 2..s + 6] == target => (m >> 3) & 7,
                _ => continue,
            };
            for l in s.saturating_sub(16).max(start)..s {
                let src = match b[l] {
                    0xa1 if reg == 0 => u32_opt(b, l + 1),
                    0x8b if b[l + 1] == (reg << 3) | 5 => u32_opt(b, l + 2),
                    _ => None,
                };
                let Some(src) = src else { continue };
                let readonly = src
                    .checked_sub(pe.image_base)
                    .and_then(|r| pe.section_for_rva(r))
                    .is_some_and(|s| s.characteristics & (SCN_WRITE | SCN_EXECUTE) == 0);
                if let Some(v) = readonly
                    .then(|| pe.va_to_file_offset(src).and_then(|o| u32_opt(b, o)))
                    .flatten()
                {
                    values.insert(v);
                }
            }
        }
    }
    values.into_iter().collect()
}

/// High-entropy second-seed candidates for the Cabbage generator, ranked by
/// structural evidence. Ambiguity is kept; archive validation decides.
fn find_cabbage_seed_candidates(
    b: &[u8],
    pe: &PeImage,
    prng: u32,
    builder: Option<u32>,
) -> Vec<u32> {
    let Some(prng_off) = pe.rva_to_file_offset(prng) else {
        return Vec::new();
    };
    let mut state = BTreeSet::new();
    let w = &b[prng_off..prng_off.saturating_add(768).min(b.len())];
    let mut i = 0;
    while i < w.len() {
        match w[i] {
            0xa1 | 0xa3 if i + 4 < w.len() => {
                state.insert(u32_opt(w, i + 1).unwrap());
                i += 5;
            }
            0x89 | 0x8b | 0x31 | 0x33 if i + 5 < w.len() => {
                if w[i + 1] & 0xc7 == 0x05 {
                    state.insert(u32_opt(w, i + 2).unwrap());
                    i += 6;
                } else {
                    i += 2;
                }
            }
            _ => i += 1,
        }
    }
    let mut scored: BTreeMap<u32, u32> = BTreeMap::new();
    let ranges: Vec<_> = pe.exec_ranges(b.len()).collect();
    // mov [esp+8],eax; mov [esp+10],imm32; mov [esp+14],edx; mov [esp+18],0; call
    for &(start, end) in &ranges {
        for o in start..end.saturating_sub(29) {
            if b[o..o + 8] == [0x89, 0x44, 0x24, 0x08, 0xc7, 0x44, 0x24, 0x10]
                && b[o + 12..o + 24] == [0x89, 0x54, 0x24, 0x14, 0xc7, 0x44, 0x24, 0x18, 0, 0, 0, 0]
                && b[o + 24] == 0xe8
            {
                let s = u32_opt(b, o + 8).unwrap();
                if seed_like(s, pe) {
                    *scored.entry(s).or_default() += 1_000_000;
                }
            }
        }
    }
    for a in &state {
        let a = a.to_le_bytes();
        for o in 0..b.len().saturating_sub(10) {
            if b[o] == 0xc7 && b[o + 1] == 0x05 && b[o + 2..o + 6] == a {
                let s = u32_opt(b, o + 6).unwrap();
                if seed_like(s, pe) {
                    *scored.entry(s).or_default() += 100;
                }
            }
        }
    }
    let immediate = |b: &[u8], j: usize, end: usize| -> Option<(u32, usize)> {
        match b[j] {
            0x68 | 0xb8..=0xbf => Some((u32_opt(b, j + 1)?, 5)),
            0xc7 if j + 8 <= end && b[j + 1] == 0x44 && b[j + 2] == 0x24 => {
                Some((u32_opt(b, j + 4)?, 8))
            }
            _ => None,
        }
    };
    for center in builder.into_iter().chain([prng]) {
        let Some(c) = pe.rva_to_file_offset(center) else {
            continue;
        };
        let (start, end) = (
            c.saturating_sub(0x2000),
            c.saturating_add(0x2000).min(b.len()),
        );
        let mut j = start;
        while j + 5 <= end {
            if let Some((s, n)) = immediate(b, j, end) {
                if seed_like(s, pe) {
                    let distance = c.abs_diff(j) as u32;
                    *scored.entry(s).or_default() +=
                        1 + 16_384u32.saturating_sub(distance.min(16_383)) / 1024;
                    if b[j + n..(j + n + 40).min(end)].contains(&0xe8) {
                        *scored.entry(s).or_default() += 48;
                    }
                }
            }
            j += 1;
        }
    }
    for &(start, end) in &ranges {
        let mut o = start;
        while o + 5 <= end {
            if let Some((s, n)) = immediate(b, o, end) {
                if seed_like(s, pe) && b[o + n..(o + n + 40).min(end)].contains(&0xe8) {
                    *scored.entry(s).or_default() += 20;
                }
            }
            o += 1;
        }
    }
    let mut v: Vec<_> = scored.into_iter().collect();
    v.sort_by_key(|&(s, score)| (std::cmp::Reverse(score), s));
    v.into_iter().take(256).map(|(s, _)| s).collect()
}
fn seed_like(v: u32, pe: &PeImage) -> bool {
    if v < 0x1_0000 || matches!(v, LCG_MUL | 0xaaaa_aaaa | 0x5555_5555 | 0xffff_ffff) {
        return false;
    }
    if pe.va_to_file_offset(v).is_some() {
        return false;
    }
    v.to_le_bytes().iter().collect::<BTreeSet<_>>().len() >= 3
}

// --------------------------------------------------------------- dispatch

#[derive(Clone, Copy)]
enum Kind {
    Prolog,
    Even,
    Odd,
}

/// 3/8/6 generator dispatch orders from compiler jump tables (or the MSVC
/// `% 3` branch chain); each target is classified by the x86 bytes it emits.
fn find_dispatch_candidates(b: &[u8], pe: &PeImage, anchor: u32) -> Vec<Dispatch> {
    let mut prologs = jump_tables(b, pe, anchor, 3, Kind::Prolog);
    prologs.extend(prolog_branches(b, pe, anchor));
    prologs.sort_by_key(|(_, r)| r.abs_diff(anchor));
    prologs.dedup_by(|a, b| a.0 == b.0);
    prologs.truncate(8);
    let evens = jump_tables(b, pe, anchor, 8, Kind::Even);
    let odds = jump_tables(b, pe, anchor, 6, Kind::Odd);
    let mut out = Vec::new();
    for (p, pr) in &prologs {
        for (e, er) in &evens {
            for (o, or) in &odds {
                out.push(Dispatch {
                    prolog: p.as_slice().try_into().unwrap(),
                    even: e.as_slice().try_into().unwrap(),
                    odd: o.as_slice().try_into().unwrap(),
                    distance: pr
                        .abs_diff(anchor)
                        .saturating_add(er.abs_diff(anchor))
                        .saturating_add(or.abs_diff(anchor)),
                });
            }
        }
    }
    out.sort_by_key(|d| d.distance);
    out.dedup_by(|a, b| (a.prolog, a.even, a.odd) == (b.prolog, b.even, b.odd));
    out.truncate(16);
    out
}
fn jump_tables(b: &[u8], pe: &PeImage, anchor: u32, n: usize, kind: Kind) -> Vec<(Vec<u8>, u32)> {
    let mut out = Vec::new();
    for (start, end) in pe.exec_ranges(b.len()).collect::<Vec<_>>() {
        if end <= start + 7 {
            continue;
        }
        for off in start..=end - 7 {
            // jmp [index*4 + disp32]
            let sib = b[off + 2];
            if b[off] != 0xff
                || b[off + 1] != 0x24
                || sib >> 6 != 2
                || sib & 7 != 5
                || (sib >> 3) & 7 == 4
            {
                continue;
            }
            let Some(table) = pe.va_to_file_offset(u32_opt(b, off + 3).unwrap()) else {
                continue;
            };
            if table.checked_add(n * 4).is_none_or(|e| e > b.len()) {
                continue;
            }
            let targets: Option<Vec<u32>> = (0..n)
                .map(|i| {
                    let rva = u32_opt(b, table + i * 4)?.checked_sub(pe.image_base)?;
                    pe.is_executable_rva(rva).then_some(rva)
                })
                .collect();
            let Some(targets) = targets else { continue };
            let order: Option<Vec<u8>> = targets
                .iter()
                .map(|&t| {
                    let to = pe.rva_to_file_offset(t)?;
                    let next = targets
                        .iter()
                        .copied()
                        .filter(|&c| c > t)
                        .min()
                        .and_then(|r| pe.rva_to_file_offset(r))
                        .filter(|&v| v > to)
                        .unwrap_or(to + 160);
                    let block_end = next.min(to + 160).min(b.len());
                    (block_end > to)
                        .then(|| classify_target(b, pe, to, block_end, kind))
                        .flatten()
                })
                .collect();
            let Some(order) = order.filter(|o| super::engine::permutation(o) && o.len() == n)
            else {
                continue;
            };
            let Some(rva) = pe.file_offset_to_rva(off) else {
                continue;
            };
            if rva.abs_diff(anchor) <= 0x20_000 {
                out.push((order, rva));
            }
        }
    }
    out.sort_by_key(|(_, r)| r.abs_diff(anchor));
    out.dedup_by(|a, b| a.0 == b.0);
    out.truncate(8);
    out
}
fn prolog_branches(b: &[u8], pe: &PeImage, anchor: u32) -> Vec<(Vec<u8>, u32)> {
    let mut out = Vec::new();
    for (start, end) in pe.exec_ranges(b.len()).collect::<Vec<_>>() {
        if end <= start + 32 {
            continue;
        }
        for off in start..end.saturating_sub(16) {
            // mov r32, 3 ; div r32
            let op = b[off];
            if !(0xb8..=0xbf).contains(&op) || b[off + 1..off + 5] != 3u32.to_le_bytes() {
                continue;
            }
            if b[off + 5] != 0xf7 || b[off + 6] != 0xf0 | (op - 0xb8) {
                continue;
            }
            let se = off.saturating_add(72).min(end);
            let mut jes = Vec::new();
            let mut c = off + 7;
            while c < se && jes.len() < 2 {
                let d = match b[c] {
                    0x74 if c + 1 < se => {
                        Some((2usize, (c as isize + 2 + b[c + 1] as i8 as isize) as usize))
                    }
                    0x0f if c + 5 < se && b[c + 1] == 0x84 => Some((
                        6,
                        (c as isize
                            + 6
                            + i32::from_le_bytes(b[c + 2..c + 6].try_into().unwrap()) as isize)
                            as usize,
                    )),
                    _ => None,
                };
                match d {
                    Some((len, t)) => {
                        jes.push((c, t));
                        c += len;
                    }
                    None => c += 1,
                }
            }
            let [(_, t0), (je1, t1)] = jes[..] else {
                continue;
            };
            let after = je1 + if b[je1] == 0x74 { 2 } else { 6 };
            let ge = after.saturating_add(24).min(se);
            let t2 = (after..ge).find_map(|q| {
                if b[q] == 0x75 && q + 1 < ge {
                    Some(q + 2)
                } else if b[q] == 0x0f && q + 5 < ge && b[q + 1] == 0x85 {
                    Some(q + 6)
                } else {
                    None
                }
            });
            let Some(t2) = t2 else { continue };
            let ts = [t0, t1, t2];
            if ts.iter().any(|&t| t < start || t >= end) {
                continue;
            }
            let order: Option<Vec<u8>> = ts
                .iter()
                .map(|&t| {
                    let next = ts.iter().copied().filter(|&c| c > t).min();
                    let be = next.unwrap_or(t + 160).min(t + 160).min(end);
                    classify_target(b, pe, t, be, Kind::Prolog)
                })
                .collect();
            let Some(order) = order.filter(|o| super::engine::permutation(o)) else {
                continue;
            };
            let Some(rva) = pe.file_offset_to_rva(off) else {
                continue;
            };
            if rva.abs_diff(anchor) <= 0x20_000 {
                out.push((order, rva));
            }
        }
    }
    out.sort_by_key(|(_, r)| r.abs_diff(anchor));
    out.dedup_by(|a, b| a.0 == b.0);
    out.truncate(8);
    out
}
fn classify_target(b: &[u8], pe: &PeImage, to: usize, end: usize, kind: Kind) -> Option<u8> {
    let end = end.min(b.len());
    if to >= end {
        return None;
    }
    let primary = &b[to..end];
    if let Some(v) = classify_block(primary, kind) {
        return Some(v);
    }
    // Follow short direct branches into shared MSVC emission tails.
    let origin = pe.file_offset_to_rva(to)?;
    let mut expanded = primary.to_vec();
    let scan_end = end.min(to + 160);
    let (mut off, mut followed) = (to, 0);
    while off < scan_end && followed < 8 {
        let d: Option<(usize, i32)> = match b[off] {
            0xeb | 0x70..=0x7f if off + 1 < scan_end => Some((2, b[off + 1] as i8 as i32)),
            0xe9 if off + 4 < scan_end => {
                Some((5, i32::from_le_bytes(b[off + 1..off + 5].try_into().ok()?)))
            }
            0x0f if off + 5 < scan_end && (0x80..=0x8f).contains(&b[off + 1]) => {
                Some((6, i32::from_le_bytes(b[off + 2..off + 6].try_into().ok()?)))
            }
            _ => None,
        };
        let Some((len, rel)) = d else {
            off += 1;
            continue;
        };
        let target = i64::from(pe.file_offset_to_rva(off)?) + len as i64 + i64::from(rel);
        let tf = u32::try_from(target)
            .ok()
            .filter(|t| t.abs_diff(origin) <= 0x4000)
            .and_then(|t| pe.rva_to_file_offset(t))
            .filter(|&t| !(t >= to && t < end));
        if let Some(tf) = tf {
            let te = (tf + 24).min(b.len());
            if tf < te {
                expanded.extend_from_slice(&b[tf..te]);
                followed += 1;
                if let Some(v) = classify_block(&expanded, kind) {
                    return Some(v);
                }
            }
        }
        off += len;
    }
    None
}
fn classify_block(b: &[u8], kind: Kind) -> Option<u8> {
    let imm = push_immediates(b);
    let has = |v: u32| imm.contains(&v);
    let emitted = |v: u8| {
        has(v as u32)
            || b.windows(3)
                .any(|w| w[0] == 0xc6 && matches!(w[1], 0 | 1) && w[2] == v)
    };
    let raw = |v: u32| contains_u32(b, v);
    match kind {
        Kind::Prolog => {
            if emitted(0xbe) && (emitted(0x86) || raw(0x3ff)) {
                Some(2)
            } else if emitted(0x8b) && emitted(0xc7) {
                Some(1)
            } else if emitted(0xb8) {
                Some(0)
            } else {
                None
            }
        }
        Kind::Even => {
            if (raw(0xaaaa_aaaa) && raw(0x5555_5555))
                || (imm.iter().filter(|&&v| v == 0xaa).count() >= 4
                    && imm.iter().filter(|&&v| v == 0x55).count() >= 4)
            {
                Some(5)
            } else if raw(0x3ff) && (has(0xbe) || has(0x86) || has(0x25)) {
                Some(4)
            } else if has(0x35) {
                Some(6)
            } else if has(0x05) || has(0x2d) {
                Some(7)
            } else if has(0xf7) && has(0xd0) {
                Some(0)
            } else if has(0xf7) && has(0xd8) {
                Some(2)
            } else if has(0x48) {
                Some(1)
            } else if has(0x40) {
                Some(3)
            } else {
                None
            }
        }
        Kind::Odd => {
            if has(0x0f) && has(0xaf) && has(0xc3) {
                Some(4)
            } else if has(0xf7) && has(0xd8) && has(0x01) {
                Some(3)
            } else if has(0x29) && has(0xd8) {
                Some(5)
            } else if has(0x01) && has(0xd8) {
                Some(2)
            } else if has(0xd3) && has(0xe8) {
                Some(0)
            } else if has(0xd3) && has(0xe0) {
                Some(1)
            } else {
                None
            }
        }
    }
}
fn push_immediates(b: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            0x6a if i + 1 < b.len() => {
                out.push(b[i + 1] as u32);
                i += 2;
            }
            0x68 if i + 4 < b.len() => {
                out.push(u32_opt(b, i + 1).unwrap());
                i += 5;
            }
            _ => i += 1,
        }
    }
    out
}

// --------------------------------------------------------- self-decoding

fn bitperm_state(old: u32) -> u32 {
    old.wrapping_mul(12_869).wrapping_add(0x1b01) ^ (old >> 3)
}
fn bitperm_byte(v: u8, mode: u32) -> u8 {
    match mode & 3 {
        0 => v.rotate_left(1),
        1 => ((v & 0x55) << 1) | ((v >> 1) & 0x55),
        2 => v.rotate_left(4),
        _ => ((v & 0x33) << 2) | ((v >> 2) & 0x33),
    }
}
pub(super) fn bitperm_decode(input: &[u8], seed: u32) -> Vec<u8> {
    let mut s = seed;
    input
        .iter()
        .map(|&v| {
            s = bitperm_state(s);
            bitperm_byte(v, s)
        })
        .collect()
}
/// Inverse used only by synthetic tests.
#[cfg(test)]
pub(super) fn bitperm_encode(input: &[u8], seed: u32) -> Vec<u8> {
    let mut s = seed;
    input
        .iter()
        .map(|&v| {
            s = bitperm_state(s);
            match s & 3 {
                0 => v.rotate_right(1),
                2 => v.rotate_right(4),
                _ => bitperm_byte(v, s),
            }
        })
        .collect()
}

/// Recover a bit-permuted executable section with seeds taken from the
/// module's own `mov r,imm32`/`push imm32` operands. Accepted only when the
/// decoded bytes reveal both classic generator LCG constants.
fn static_self_decode(raw: &[u8]) -> Option<(Vec<u8>, u32)> {
    const PROBE: usize = 256 * 1024;
    let pe = PeImage::parse(raw).ok()?;
    if !pe.i386() || (contains_u32(raw, LCG_MUL) && contains_u32(raw, LCG_ADD)) {
        return None;
    }
    let seeds = mov_push_immediates(raw, &pe);
    for require_write in [true, false] {
        for s in &pe.sections {
            if !s.executable() || (require_write && s.characteristics & SCN_WRITE == 0) {
                continue;
            }
            let start = s.raw_offset as usize;
            let semantic = if s.virtual_size == 0 {
                s.raw_size
            } else {
                s.virtual_size.min(s.raw_size)
            } as usize;
            if semantic == 0 || start >= raw.len() {
                continue;
            }
            let avail = semantic.min(raw.len() - start);
            let probe = &raw[start..start + avail.min(PROBE)];
            // Seed trials are independent; split them across worker threads.
            let workers = std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .clamp(1, 16);
            let chunk = seeds.len().div_ceil(workers).max(1);
            let hit = std::thread::scope(|scope| {
                let jobs: Vec<_> = seeds
                    .chunks(chunk)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter().copied().find(|&seed| {
                                let d = bitperm_decode(probe, seed);
                                contains_u32(&d, LCG_MUL) && contains_u32(&d, LCG_ADD)
                            })
                        })
                    })
                    .collect();
                // First proven seed in the original (ranked) order.
                jobs.into_iter()
                    .filter_map(|j| j.join().ok().flatten())
                    .next()
            });
            if let Some(seed) = hit {
                let mut image = raw.to_vec();
                image[start..start + avail]
                    .copy_from_slice(&bitperm_decode(&raw[start..start + avail], seed));
                return Some((image, seed));
            }
        }
    }
    None
}
fn mov_push_immediates(b: &[u8], pe: &PeImage) -> Vec<u32> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    'pass: for prefer_clean in [true, false] {
        for s in &pe.sections {
            if !s.executable() || (prefer_clean && s.characteristics & SCN_WRITE != 0) {
                continue;
            }
            let (start, end) = s.file_range(b.len());
            for i in start..end.saturating_sub(4) {
                if (0xb8..=0xbf).contains(&b[i]) || b[i] == 0x68 {
                    let v = u32_opt(b, i + 1).unwrap();
                    if seen.insert(v) {
                        out.push(v);
                        if out.len() >= 4096 {
                            break 'pass;
                        }
                    }
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out.sort_by_key(|&v| {
        if v <= 0xffff {
            0u8
        } else if v <= 0x00ff_ffff {
            1
        } else {
            2
        }
    });
    out
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    #[test]
    fn bitperm_round_trips() {
        let plain: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
        assert_eq!(
            bitperm_decode(&bitperm_encode(&plain, 0x1234), 0x1234),
            plain
        );
    }
    #[test]
    fn prefix8_constants_resolve_immediate_and_rdata_globals() {
        // Synthetic PE32: .text (RX) at 0x1000, .rdata (R) at 0x2000, .data (RW) at 0x3000.
        let mut img = synthetic_pe();
        let text = 0x400usize;
        // Key expansion with globals: shl ecx,0xd; shr edx,0x11; shl ecx,5;
        // xor ecx,[0x10003000]; xor edx,[0x10003004]
        let code: &[u8] = &[
            0x8b, 0xc8, 0xc1, 0xe1, 0x0d, 0x33, 0xc1, 0x8b, 0xd0, 0xc1, 0xea, 0x11, 0x33, 0xc2,
            0x8b, 0xc8, 0xc1, 0xe1, 0x05, 0x33, 0x0d, 0x00, 0x30, 0x00, 0x10, 0x33, 0xc8, 0x33,
            0x15, 0x04, 0x30, 0x00, 0x10, 0xc3,
            // link-time copy from .rdata: mov edx,[0x10002000]; mov eax,[0x10002004];
            // mov [0x10003000],edx; mov [0x10003004],eax
            0x8b, 0x15, 0x00, 0x20, 0x00, 0x10, 0xa1, 0x04, 0x20, 0x00, 0x10, 0x89, 0x15, 0x00,
            0x30, 0x00, 0x10, 0xa3, 0x04, 0x30, 0x00, 0x10, 0xc3,
        ];
        img[text..text + code.len()].copy_from_slice(code);
        img[0x600..0x604].copy_from_slice(&0x1111_2222u32.to_le_bytes());
        img[0x604..0x608].copy_from_slice(&0x3333_4444u32.to_le_bytes());
        img[0x800..0x804].copy_from_slice(&0xaaaa_aaaau32.to_le_bytes());
        img[0x804..0x808].copy_from_slice(&0x5555_5555u32.to_le_bytes());
        let pe = PeImage::parse(&img).unwrap();
        let got = recover_prefix8_constants(&img, &pe);
        assert!(got.contains(&Prefix8 {
            hi: 0x1111_2222,
            lo: 0x3333_4444
        }));
        assert!(got.contains(&Prefix8::DEFAULT));
        // Immediate form.
        let mut img2 = synthetic_pe();
        let code2: &[u8] = &[
            0xc1, 0xe1, 0x0d, 0xc1, 0xea, 0x11, 0xc1, 0xe1, 0x05, 0x33, 0xc8, 0x81, 0xf1, 0x78,
            0x56, 0x34, 0x12, 0x81, 0xf2, 0x21, 0x43, 0x65, 0x87, 0xc3,
        ];
        img2[text..text + code2.len()].copy_from_slice(code2);
        let pe2 = PeImage::parse(&img2).unwrap();
        assert!(recover_prefix8_constants(&img2, &pe2).contains(&Prefix8 {
            hi: 0x1234_5678,
            lo: 0x8765_4321
        }));
    }
    pub(crate) fn synthetic_pe() -> Vec<u8> {
        let mut b = vec![0u8; 0xa00];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x14cu16.to_le_bytes());
        b[0x86..0x88].copy_from_slice(&3u16.to_le_bytes());
        b[0x94..0x96].copy_from_slice(&0xe0u16.to_le_bytes());
        let opt = 0x98;
        b[opt..opt + 2].copy_from_slice(&0x10bu16.to_le_bytes());
        b[opt + 28..opt + 32].copy_from_slice(&0x1000_0000u32.to_le_bytes());
        b[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes());
        let table = opt + 0xe0;
        for (i, (name, va, raw, ch)) in [
            (b".text\0\0\0", 0x1000u32, 0x400u32, 0x6000_0020u32),
            (b".rdata\0\0", 0x2000, 0x600, 0x4000_0040),
            (b".data\0\0\0", 0x3000, 0x800, 0xc000_0040),
        ]
        .into_iter()
        .enumerate()
        {
            let p = table + i * 40;
            b[p..p + 8].copy_from_slice(name);
            b[p + 8..p + 12].copy_from_slice(&0x200u32.to_le_bytes());
            b[p + 12..p + 16].copy_from_slice(&va.to_le_bytes());
            b[p + 16..p + 20].copy_from_slice(&0x200u32.to_le_bytes());
            b[p + 20..p + 24].copy_from_slice(&raw.to_le_bytes());
            b[p + 36..p + 40].copy_from_slice(&ch.to_le_bytes());
        }
        b
    }
}
