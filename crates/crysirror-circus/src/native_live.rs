//! Native save-image state, separate from shared render resources. The fresh
//! bytes follow40EC50, sprite records follow444BC0/444180. Mutation coverage is
//! explicit: an unaudited accepted instruction prevents save-image admission.
use crate::native_save::{
    NativeReadRun, NativeSaveBody, BODY_BYTES, LIVE_BASE, READ_RUN_COUNT, SPRITE_BYTES,
    SPRITE_COUNT,
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
pub struct NativeLiveState {
    #[serde(deserialize_with = "deserialize_body")]
    body: Vec<u8>,
    #[serde(deserialize_with = "deserialize_sprites")]
    sprites: Vec<Vec<u8>>,
    image_bindings: BTreeMap<u8, usize>,
    gaps: BTreeSet<String>,
    completed: BTreeSet<u8>,
}
impl std::fmt::Debug for NativeLiveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeLiveState")
            .field("body_bytes", &self.body.len())
            .field("sprites", &self.sprites.len())
            .field("gaps", &self.gaps)
            .finish_non_exhaustive()
    }
}
fn deserialize_body<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<u8>, D::Error> {
    let bytes = Vec::<u8>::deserialize(d)?;
    if bytes.len() != BODY_BYTES {
        return Err(serde::de::Error::custom("native live body extent"));
    }
    Ok(bytes)
}
fn deserialize_sprites<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<Vec<u8>>, D::Error> {
    let bytes = Vec::<Vec<u8>>::deserialize(d)?;
    if bytes.len() != SPRITE_COUNT || bytes.iter().any(|s| s.len() != SPRITE_BYTES) {
        return Err(serde::de::Error::custom("native live sprite extent"));
    }
    Ok(bytes)
}
impl NativeLiveState {
    /// Native40EC50 zeroes the entire live body. These are actual initial VM
    /// bytes, never a serialization fallback for a partly known logical State.
    pub(crate) fn fresh() -> Self {
        Self {
            body: vec![0; BODY_BYTES],
            sprites: (0..SPRITE_COUNT).map(|_| fresh_sprite()).collect(),
            image_bindings: BTreeMap::new(),
            gaps: BTreeSet::new(),
            completed: BTreeSet::new(),
        }
    }
    pub(crate) fn unavailable() -> Self {
        let mut state = Self::fresh();
        state.gap("legacy logical snapshot has no native live image");
        state
    }
    pub fn gaps(&self) -> &BTreeSet<String> {
        &self.gaps
    }
    pub fn completed_opcodes(&self) -> &BTreeSet<u8> {
        &self.completed
    }
    pub(crate) fn gap(&mut self, reason: impl Into<String>) {
        self.gaps.insert(reason.into());
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.body.len() == BODY_BYTES
                && self.sprites.len() == SPRITE_COUNT
                && self.sprites.iter().all(|s| s.len() == SPRITE_BYTES),
            "Circus native live image extent"
        );
        Ok(())
    }
    fn write32(&mut self, absolute: usize, value: u32) {
        let at = absolute - LIVE_BASE;
        self.body[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn read32(&self, absolute: usize) -> u32 {
        let at = absolute - LIVE_BASE;
        u32::from_le_bytes(self.body[at..at + 4].try_into().unwrap())
    }
    fn read16(&self, absolute: usize) -> u16 {
        let at = absolute - LIVE_BASE;
        u16::from_le_bytes(self.body[at..at + 2].try_into().unwrap())
    }
    fn cstring(&mut self, absolute: usize, capacity: usize, text: &str) -> Result<()> {
        let (bytes, _, errors) = encoding_rs::SHIFT_JIS.encode(text);
        ensure!(
            !errors && bytes.len() < capacity && !bytes.contains(&0),
            "Circus native live string extent/encoding"
        );
        let at = absolute - LIVE_BASE;
        self.body[at..at + bytes.len()].copy_from_slice(&bytes);
        self.body[at + bytes.len()] = 0;
        Ok(())
    }
    pub(crate) fn byte_assignment(&mut self, bank: i16, index: i16, value: u8) {
        if (0..=10).contains(&bank) && (0..100).contains(&index) {
            self.body[118892 - LIVE_BASE + bank as usize * 100 + index as usize] = value;
        }
    }
    pub(crate) fn system_assignment(&mut self, index: i16, value: i32) {
        //40D240: all other currently accepted system setters write outside
        // this live body; preference and window state remain distinct.
        if let Some(at) = match index {
            233 => Some(115880),
            241 => Some(118888),
            _ => None,
        } {
            self.write32(at, value as u32);
        }
    }
    pub(crate) fn reset_script(&mut self, clear_banks: bool, names: &[String; 2]) -> Result<()> {
        self.validate()?;
        if clear_banks {
            self.body[118892 - LIVE_BASE..121092 - LIVE_BASE].fill(0);
        }
        self.cstring(265268, 20, &names[1])?;
        self.cstring(265288, 20, &names[0])?;
        self.body[106240 - LIVE_BASE..106242 - LIVE_BASE].copy_from_slice(&u16::MAX.to_le_bytes());
        for at in [
            106236, 111856, 111860, 111864, 111868, 111872, 115884, 115880,
        ] {
            self.write32(at, 0);
        }
        Ok(())
    }
    pub(crate) fn random_result(&mut self, value: i32) {
        let count = self.read32(111864) as usize;
        if count < 200 {
            self.write32(115080 + 4 * count, value as u32);
            self.write32(111864, count as u32 + 1);
        }
    }
    pub(crate) fn music(&mut self, id: i16, name: &str) -> Result<()> {
        self.cstring(21944, 32, name)?;
        self.write32(21976, id as u32);
        Ok(())
    }
    pub(crate) fn placement(&mut self, kind: i16, name: &str, position: [i16; 2]) -> Result<()> {
        ensure!((0..20).contains(&kind), "Circus native placement table");
        let count_at = 106156 + kind as usize * 4;
        let count = self.read32(count_at) as usize;
        ensure!(count <= 100, "Circus native placement capacity");
        let base = 26152 + kind as usize * 4000;
        let name = name.to_ascii_uppercase();
        let found = (0..count).find(|index| {
            let at = base + index * 40 - LIVE_BASE;
            let row = &self.body[at..at + 32];
            let end = row.iter().position(|b| *b == 0).unwrap_or(32);
            row[..end].eq_ignore_ascii_case(name.as_bytes())
        });
        let index = found.unwrap_or(count);
        ensure!(index < 100, "Circus native placement capacity");
        self.cstring(base + index * 40, 32, &name)?;
        self.write32(base + index * 40 + 32, position[0] as i32 as u32);
        self.write32(base + index * 40 + 36, position[1] as i32 as u32);
        if found.is_none() {
            self.write32(count_at, count as u32 + 1);
        }
        Ok(())
    }
    pub(crate) fn voice_timing(&mut self, mode: i16, name: &str) -> Result<()> {
        ensure!(
            mode == 255 || name.starts_with('-') || (name.is_ascii() && name.len() < 32),
            "Circus voice timing prefix extent/encoding"
        );
        self.write32(269848, 0);
        if mode == 255 || name.starts_with('-') {
            self.body[268008 - LIVE_BASE..269844 - LIVE_BASE].fill(0);
            self.write32(269844, 0);
            if mode == 255 {
                self.write32(269848, 1);
            }
            return Ok(());
        }
        let count = self.read32(269844) as usize;
        ensure!(count <= 50, "Circus voice timing table capacity");
        for index in 0..count {
            let at = 268012 + index * 36 - LIVE_BASE;
            let row = &self.body[at..at + 32];
            if row.len() >= name.len() && row[..name.len()].eq_ignore_ascii_case(name.as_bytes()) {
                return Ok(());
            }
        }
        ensure!(count < 50, "Circus voice timing table capacity");
        let gain = 100
            * if mode > 100 {
                i32::from(mode) - 256
            } else {
                i32::from(mode)
            };
        self.write32(268008 + count * 36, gain as u32);
        self.cstring(268012 + count * 36, 32, &name.to_ascii_uppercase())?;
        self.write32(269844, count as u32 + 1);
        Ok(())
    }
    pub(crate) fn read_label(
        &mut self,
        scene_index: i32,
        label: u16,
        metadata: u16,
        raw: u32,
        track: bool,
    ) -> Result<()> {
        self.validate()?;
        if track {
            let mut image = NativeSaveBody::from_exact(&self.body)?;
            let mut index = image.read_run_index() as usize;
            ensure!(index < READ_RUN_COUNT, "Circus native live read-run index");
            let old = image.read_run(index)?;
            let flags = self.read16(115884) | if self.read32(115880) != 0 { 0x8000 } else { 0 };
            let mut run = if old.scene_index == scene_index as u16
                && u32::from(old.last_label) + 1 == u32::from(label)
            {
                NativeReadRun {
                    last_label: label,
                    metadata_and_flags: flags,
                    ..old
                }
            } else {
                if index < 399 && old.scene_index != u16::MAX {
                    let duplicate = (0..index).rev().any(|n| {
                        image.read_run(n).is_ok_and(|p| {
                            p.scene_index == old.scene_index
                                && p.last_label == old.last_label
                                && p.first_label <= old.first_label
                        })
                    });
                    if !duplicate {
                        index += 1;
                        image.set_read_run_index(index)?;
                    }
                }
                NativeReadRun {
                    scene_index: scene_index as u16,
                    metadata_and_flags: flags,
                    first_label: label,
                    last_label: label,
                    source_word_55936: self.read16(111872),
                    source_word_55930: self.read16(111860),
                    source_word_55934: self.read16(111868),
                }
            };
            // The assignment above deliberately retains all metadata when an
            // existing contiguous run is extended, matching4036D0.
            run.metadata_and_flags = flags;
            image.set_read_run(index, run)?;
            self.body.copy_from_slice(image.bytes());
            self.write32(111848, u32::from(label));
            if (raw as i32) < 0 {
                self.write32(111872, self.read32(111872).wrapping_add(1));
            }
        }
        if self.read32(111844) == 0 {
            self.write32(111844, u32::from(label));
        }
        self.write32(22136, u32::from(metadata));
        self.write32(22140, u32::from(label));
        Ok(())
    }
    pub(crate) fn image_name(&mut self, slot: u8, text: &str) -> Result<()> {
        ensure!(slot < 51, "Circus native image-name slot");
        self.cstring(19496 + usize::from(slot) * 48, 48, text)
    }
    pub(crate) fn position(&mut self, slot: u8, position: [i16; 2]) {
        if let Some(index) = sprite_for_slot(slot) {
            let s = &mut self.sprites[index];
            for (at, value) in [
                (8, position[0] as f32),
                (12, position[1] as f32),
                (56, position[0] as f32),
                (60, position[1] as f32),
            ] {
                put_object_float(s, at, value);
            }
        }
    }
    pub(crate) fn scale(&mut self, slot: u8, percent: i16, pivot: [i16; 2]) {
        if let Some(index) = sprite_for_slot(slot) {
            let s = &mut self.sprites[index];
            for (at, value) in [
                (128, percent as f32),
                (132, percent as f32),
                (152, pivot[0] as f32),
                (156, pivot[1] as f32),
            ] {
                put_object_float(s, at, value);
            }
        }
    }
    ///443EA0 uses native texture-table allocation size and logical image size.
    /// These extents come from the completed engine-owned image resource.
    pub(crate) fn attach_image(
        &mut self,
        slot: u8,
        allocation: [u32; 2],
        logical: [u32; 2],
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            allocation.iter().chain(logical.iter()).all(|v| *v > 0),
            "Circus native texture dimensions"
        );
        let Some(index) = sprite_for_slot(slot) else {
            return Ok(());
        };
        let s = &mut self.sprites[index];
        for (at, value) in [
            (168, allocation[0]),
            (172, allocation[1]),
            (72, logical[0]),
            (76, logical[1]),
            (176, allocation[0]),
            (180, allocation[1]),
        ] {
            put_object_float(s, at, value as f32);
        }
        if object_float(s, 104) == object_float(s, 96)
            || object_float(s, 108) == object_float(s, 100)
        {
            for (at, value) in [
                (96, 0.),
                (100, 0.),
                (104, logical[0] as f32),
                (108, logical[1] as f32),
            ] {
                put_object_float(s, at, value);
            }
        }
        for (at, value) in [(64, allocation[0]), (68, allocation[1])] {
            if object_float(s, at) == 0. {
                put_object_float(s, at, value as f32);
            }
        }
        put_object_word(s, 184, 1);
        // Never retain a host/device identity in original native VM data.
        // The semantic binding is separate;41DD10 clears saved identity188.
        self.image_bindings.insert(slot, index);
        Ok(())
    }
    pub(crate) fn audit_completed(&mut self, opcode: u8, scene: &str, pc: u32) -> Result<()> {
        self.validate()?;
        self.completed.insert(opcode);
        self.cstring(19336, 128, scene)?;
        self.write32(22132, pc);
        // This deliberately narrow set has either a direct implementation
        // above or the original dispatch's explicit no-live-body branch.
        if !matches!(opcode,0x00..=0x04|0x06|0x08|0x0d|0x0e|0x11|0x12|0x13|0x14|0x19|0x1c|0x1f|0x20|0x23|0x25|0x27|
            0x29..=0x31|0x3a|0x3b|0x42|0x48|0x4b|0x53|0x54|0x57|0x5d|0x60|0x62|0x66|0x69|0x6c|0x6e|0x8b..=0x8d|
            0x46|0x47|0x70|0x91|0x92|0x93|0x97|0x98|0xa3|0xa6|0xaf|0xb3)
        {
            self.gap(format!(
                "opcode0x{opcode:02x} live-body writes not yet audited"
            ));
        }
        Ok(())
    }
    pub(crate) fn checkpoint(&mut self, resume: u32) -> Result<NativeSaveBody> {
        self.validate()?;
        self.write32(22132, resume);
        self.write32(22144, resume);
        let at = 121248 - LIVE_BASE;
        for (index, sprite) in self.sprites.iter().enumerate() {
            self.body[at + index * SPRITE_BYTES..at + (index + 1) * SPRITE_BYTES]
                .copy_from_slice(sprite);
        }
        let mut image = NativeSaveBody::from_exact(&self.body)?;
        image.apply_checkpoint_sprite_fixups();
        //41DD10 fixes the live records before its memcpy, not only the mirror.
        self.body.copy_from_slice(image.bytes());
        Ok(image)
    }
    pub fn admitted_checkpoint(&mut self, resume: u32) -> Result<NativeSaveBody> {
        ensure!(
            self.gaps.is_empty(),
            "Circus native save projection gaps: {:?}",
            self.gaps
        );
        self.checkpoint(resume)
    }
}
fn put_object_word(record: &mut [u8], object_offset: usize, value: u32) {
    record[object_offset - 8..object_offset - 4].copy_from_slice(&value.to_le_bytes());
}
fn put_object_float(record: &mut [u8], object_offset: usize, value: f32) {
    put_object_word(record, object_offset, value.to_bits());
}
fn object_float(record: &[u8], object_offset: usize) -> f32 {
    f32::from_bits(u32::from_le_bytes(
        record[object_offset - 8..object_offset - 4]
            .try_into()
            .unwrap(),
    ))
}
fn fresh_sprite() -> Vec<u8> {
    let mut record = vec![0; SPRITE_BYTES];
    for at in [112, 120, 116, 40, 36, 32] {
        put_object_float(&mut record, at, 1.);
    }
    for at in [132, 128, 736] {
        put_object_float(&mut record, at, 100.);
    }
    put_object_float(&mut record, 160, 1280.);
    put_object_float(&mut record, 164, 720.);
    put_object_word(&mut record, 184, 1);
    put_object_word(&mut record, 904, 50);
    //444180 writes integer clipping coordinates, not float bit patterns.
    for (at, value) in [(80, 0), (84, 0), (88, 1280), (92, 720)] {
        put_object_word(&mut record, at, value);
    }
    record
}
fn sprite_for_slot(slot: u8) -> Option<usize> {
    Some(match slot {
        1 => 0,
        2 => 1,
        3 => 2,
        4 => 10,
        5 => 20,
        6 => 30,
        7 => 40,
        8 => 50,
        9 => 12,
        10 => 22,
        11 => 32,
        12 => 42,
        13 => 52,
        25 => 63,
        26 => 65,
        31 => 75,
        32 => 76,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_slot_tables_match_original_music_placement_and_voice_memory() {
        let mut live = NativeLiveState::fresh();
        live.music(8, "BGM\\M008").unwrap();
        assert_eq!(live.read32(21976), 8);
        let old = live.body[21945 - LIVE_BASE..21976 - LIVE_BASE].to_vec();
        live.music(255, "").unwrap();
        assert_eq!(live.read32(21976), 255);
        assert_eq!(live.body[21944 - LIVE_BASE], 0);
        assert_eq!(&live.body[21945 - LIVE_BASE..21976 - LIVE_BASE], &old);
        live.placement(19, "CHAR_A", [-20, 300]).unwrap();
        live.placement(19, "char_a", [12, -40]).unwrap();
        assert_eq!(live.read32(106156 + 19 * 4), 1);
        assert_eq!(live.read32(26152 + 19 * 4000 + 32), 12);
        assert_eq!(live.read32(26152 + 19 * 4000 + 36), (-40i32) as u32);
        live.voice_timing(101, "hero_a").unwrap();
        live.voice_timing(10, "HERO").unwrap();
        assert_eq!(live.read32(269844), 1);
        assert_eq!(live.read32(268008), (-15500i32) as u32);
        assert_eq!(
            &live.body[268012 - LIVE_BASE..268019 - LIVE_BASE],
            b"HERO_A\0"
        );
        live.voice_timing(255, "").unwrap();
        assert_eq!(live.read32(269844), 0);
        assert_eq!(live.read32(269848), 1);
        assert!(live.body[268008 - LIVE_BASE..269844 - LIVE_BASE]
            .iter()
            .all(|b| *b == 0));
    }
    #[test]
    fn new_native_metadata_snapshot_roundtrip_rejects_wrong_extents() {
        let mut live = NativeLiveState::fresh();
        live.position(4, [1, 2]);
        live.gap("specific native operation pending");
        let encoded = serde_json::to_vec(&live).unwrap();
        let mut restored: NativeLiveState = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.gaps(), live.gaps());
        assert!(restored.admitted_checkpoint(4).is_err());
        assert_eq!(
            restored
                .checkpoint(4)
                .unwrap()
                .sprite(10)
                .unwrap()
                .position(),
            [1., 2.]
        );
        let mut corrupt = serde_json::to_value(&live).unwrap();
        corrupt["body"] = serde_json::json!([1, 2]);
        assert!(serde_json::from_value::<NativeLiveState>(corrupt).is_err());
    }
    #[test]
    fn fresh_native_records_contain_original_nonzero_defaults_and_integer_clip() {
        let mut live = NativeLiveState::fresh();
        let image = live.admitted_checkpoint(0x123).unwrap();
        let sprite = image.sprite(149).unwrap();
        assert_eq!(sprite.position(), [0., 0.]);
        assert_eq!(sprite.scale_percent(), [100., 100.]);
        assert_eq!(sprite.pivot(), [0., 0.]);
        for (at, value) in [(80, 0), (84, 0), (88, 1280), (92, 720), (184, 1), (904, 50)] {
            assert_eq!(
                u32::from_le_bytes(sprite.bytes()[at - 8..at - 4].try_into().unwrap()),
                value
            );
        }
        assert_eq!(image.pc_fields().decode_next, 0x123);
        assert_eq!(image.pc_fields().checkpoint_resume, 0x123);
        assert!(image.sprite(11).unwrap().bytes().iter().all(|b| *b == 0));
    }
    #[test]
    fn byte_banks_reset_both_arrays_and_names_follow_original_copy_order() {
        let mut live = NativeLiveState::fresh();
        live.body[118892 - LIVE_BASE..121092 - LIVE_BASE].fill(77);
        live.reset_script(true, &["常坂".into(), "一登".into()])
            .unwrap();
        assert!(live.body[118892 - LIVE_BASE..121092 - LIVE_BASE]
            .iter()
            .all(|b| *b == 0));
        assert_eq!(
            &live.body[265268 - LIVE_BASE..265273 - LIVE_BASE],
            b"\x88\xea\x93o\0"
        );
        assert_eq!(
            &live.body[265288 - LIVE_BASE..265293 - LIVE_BASE],
            b"\x8f\xed\x8d\xe2\0"
        );
        assert_eq!(live.read16(106240), u16::MAX);
        live.byte_assignment(10, 99, 255);
        assert_eq!(live.body[119991 - LIVE_BASE], 255);
        live.reset_script(false, &["常坂".into(), "一登".into()])
            .unwrap();
        assert_eq!(live.body[119991 - LIVE_BASE], 255);
    }
    #[test]
    fn read_history_metadata_contiguous_extension_and_tail_retention_are_native() {
        let mut live = NativeLiveState::fresh();
        live.reset_script(true, &["常坂".into(), "一登".into()])
            .unwrap();
        live.write32(111860, 7);
        live.write32(111868, 9);
        live.write32(111872, 11);
        live.write32(115884, 0x1234);
        live.read_label(3, 5, 17, 80, true).unwrap();
        live.system_assignment(233, 1);
        live.read_label(3, 6, 18, 90, true).unwrap();
        let body = NativeSaveBody::from_exact(&live.body).unwrap();
        assert_eq!(body.read_run_index(), 0);
        assert_eq!(
            body.read_run(0).unwrap(),
            NativeReadRun {
                scene_index: 3,
                metadata_and_flags: 0x9234,
                first_label: 5,
                last_label: 6,
                source_word_55936: 11,
                source_word_55930: 7,
                source_word_55934: 9
            }
        );
        live.read_label(4, 2, 19, 100, true).unwrap();
        let body = NativeSaveBody::from_exact(&live.body).unwrap();
        assert_eq!(body.read_run_index(), 1);
        let retained = body.read_run(1).unwrap();
        live.reset_script(false, &["常坂".into(), "一登".into()])
            .unwrap();
        assert_eq!(
            NativeSaveBody::from_exact(&live.body)
                .unwrap()
                .read_run(1)
                .unwrap(),
            retained
        );
    }
    #[test]
    fn transforms_texture_extents_and_random_history_are_synchronized_without_host_ids() {
        let mut live = NativeLiveState::fresh();
        live.attach_image(4, [2048, 1024], [1280, 720]).unwrap();
        live.position(4, [-20, 30]);
        live.scale(4, 150, [100, 200]);
        live.random_result(17);
        let body = live.admitted_checkpoint(9).unwrap();
        let sprite = body.sprite(10).unwrap();
        assert_eq!(sprite.position(), [-20., 30.]);
        assert_eq!(sprite.scale_percent(), [150., 150.]);
        assert_eq!(sprite.pivot(), [100., 200.]);
        assert_eq!(sprite.transient_texture_identity(), 0);
        assert_eq!(object_float(sprite.bytes(), 64), 2048.);
        assert_eq!(object_float(sprite.bytes(), 104), 1280.);
        assert_eq!(live.read32(111864), 1);
        assert_eq!(live.read32(115080), 17);
        live.audit_completed(0x49, "start", 10).unwrap();
        assert!(live.admitted_checkpoint(11).is_err());
        assert!(live
            .gaps()
            .contains("opcode0x49 live-body writes not yet audited"));
    }
}
