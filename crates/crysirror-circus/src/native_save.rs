//! Original DC4PH slot-body ABI. This codec preserves every imported byte;
//! it is not a live-State projection, slot restoration or fake autosave writer.
//! Sources: owner-authorized static 40C360/40C620/41DD10/444770/444BC0;
//! executable SHA256 and analysis provenance are retained in SOURCE.json.
use anyhow::{ensure, Context, Result};
use std::{fmt, ops::Range};
/// Native `_read` prefix. Original availability ignores trailing file data.
pub const BODY_BYTES: usize = 0x56fe4;
pub const LIVE_BASE: usize = 19328;
pub const IMAGE_NAME_BYTES: usize = 48;
pub const IMAGE_NAME_COUNT: usize = 51;
pub const SPRITE_BYTES: usize = 960;
pub const SPRITE_COUNT: usize = 150;
pub const MOVIE_BYTES: usize = 1668;
pub const READ_RUN_BYTES: usize = 14;
pub const READ_RUN_COUNT: usize = 400;
const IMAGE_NAMES: usize = 19496 - LIVE_BASE;
const SPRITES: usize = 121248 - LIVE_BASE;
const MOVIE: usize = 266340 - LIVE_BASE;
const BYTE_BANKS: usize = 118892 - LIVE_BASE;
const NATIVE_DATE: usize = 100154;
const READ_RUNS: usize = 106240 - LIVE_BASE;
const READ_RUN_INDEX: usize = 106236 - LIVE_BASE;
/// A complete native image admitted from bytes. There is intentionally no
/// default/zero constructor and no conversion from the partial logical VM.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeSaveBody {
    bytes: Box<[u8]>,
}
impl fmt::Debug for NativeSaveBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSaveBody")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativePcFields {
    pub decode_next: u32,
    pub label_metadata: u32,
    pub label: u32,
    pub checkpoint_resume: u32,
}
/// The seven original WORDs copied by4036D0. The last three are retained
/// native metadata, not reconstructed from the logical read-range set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeReadRun {
    pub scene_index: u16,
    pub metadata_and_flags: u16,
    pub first_label: u16,
    pub last_label: u16,
    pub source_word_55936: u16,
    pub source_word_55930: u16,
    pub source_word_55934: u16,
}
impl NativeReadRun {
    fn words(self) -> [u16; 7] {
        [
            self.scene_index,
            self.metadata_and_flags,
            self.first_label,
            self.last_label,
            self.source_word_55936,
            self.source_word_55930,
            self.source_word_55934,
        ]
    }
}
/// Original copied record bytes, not a GPU handle or executable pointer.
pub struct NativeSprite<'a> {
    bytes: &'a [u8],
}
impl NativeSprite<'_> {
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }
    pub fn position(&self) -> [f32; 2] {
        [float(self.bytes, 0), float(self.bytes, 4)]
    }
    /// Native4441A0 stores percentages at object+128/+132; the saved image
    /// starts at object+8. Do not convert these to normalized engine scales.
    pub fn scale_percent(&self) -> [f32; 2] {
        [float(self.bytes, 120), float(self.bytes, 124)]
    }
    /// Native444230 local pivot; no world-space/UI coordinate reinterpretation.
    pub fn pivot(&self) -> [f32; 2] {
        [float(self.bytes, 144), float(self.bytes, 148)]
    }
    /// Snapshot41DD10 explicitly zeroes this native texture identity. It is
    /// merely imported integer data and must never be dereferenced by a host.
    pub fn transient_texture_identity(&self) -> u32 {
        word(self.bytes, 180)
    }
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn float(bytes: &[u8], at: usize) -> f32 {
    f32::from_bits(word(bytes, at))
}
fn checked_record(start: usize, size: usize, count: usize, index: usize) -> Result<Range<usize>> {
    ensure!(index < count, "native save record index");
    let at = start
        .checked_add(
            index
                .checked_mul(size)
                .context("native save record overflow")?,
        )
        .context("native save record overflow")?;
    Ok(at..at + size)
}
impl NativeSaveBody {
    /// Admit exactly the native body without discarding or fabricating bytes.
    pub fn from_exact(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == BODY_BYTES,
            "native Circus body must be exactly {BODY_BYTES} bytes"
        );
        Ok(Self {
            bytes: bytes.into(),
        })
    }
    /// Match40C620's exact-prefix read, not a claim of semantic restoration.
    pub fn from_read_prefix(bytes: &[u8]) -> Result<Self> {
        Self::from_exact(
            bytes
                .get(..BODY_BYTES)
                .context("native Circus save prefix truncated")?,
        )
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn pc_fields(&self) -> NativePcFields {
        NativePcFields {
            decode_next: word(&self.bytes, 22132 - LIVE_BASE),
            label_metadata: word(&self.bytes, 22136 - LIVE_BASE),
            label: word(&self.bytes, 22140 - LIVE_BASE),
            checkpoint_resume: word(&self.bytes, 22144 - LIVE_BASE),
        }
    }
    pub fn set_pc_fields(&mut self, pc: NativePcFields) {
        for (at, value) in [
            (22132, pc.decode_next),
            (22136, pc.label_metadata),
            (22140, pc.label),
            (22144, pc.checkpoint_resume),
        ] {
            self.bytes[at - LIVE_BASE..at - LIVE_BASE + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    pub fn byte_bank(&self, bank: u8, index: u8) -> Result<u8> {
        ensure!(bank < 11 && index < 100, "native Circus byte bank index");
        Ok(self.bytes[BYTE_BANKS + usize::from(bank) * 100 + usize::from(index)])
    }
    pub fn set_byte_bank(&mut self, bank: u8, index: u8, value: u8) -> Result<()> {
        ensure!(bank < 11 && index < 100, "native Circus byte bank index");
        self.bytes[BYTE_BANKS + usize::from(bank) * 100 + usize::from(index)] = value;
        Ok(())
    }
    pub fn image_name_bytes(&self, slot: usize) -> Result<&[u8]> {
        let range = checked_record(IMAGE_NAMES, IMAGE_NAME_BYTES, IMAGE_NAME_COUNT, slot)?;
        let record = &self.bytes[range];
        let end = record
            .iter()
            .position(|b| *b == 0)
            .context("unterminated native Circus image name")?;
        Ok(&record[..end])
    }
    /// Native strcpy/clear changes bytes only through the terminator. Old
    /// trailing storage is retained, including bytes unknown to this reader.
    pub fn set_image_name_bytes(&mut self, slot: usize, name: &[u8]) -> Result<()> {
        let range = checked_record(IMAGE_NAMES, IMAGE_NAME_BYTES, IMAGE_NAME_COUNT, slot)?;
        ensure!(
            name.len() < IMAGE_NAME_BYTES && !name.contains(&0),
            "native Circus image name extent"
        );
        let record = &mut self.bytes[range];
        record[..name.len()].copy_from_slice(name);
        record[name.len()] = 0;
        Ok(())
    }
    pub fn sprite(&self, index: usize) -> Result<NativeSprite<'_>> {
        Ok(NativeSprite {
            bytes: &self.bytes[checked_record(SPRITES, SPRITE_BYTES, SPRITE_COUNT, index)?],
        })
    }
    /// Original41DD10's capture fixup only. Source resources still require a
    /// proper native projection and actual shared owner-completed capture.
    pub fn clear_captured_texture_identity(&mut self, index: usize) -> Result<()> {
        let range = checked_record(SPRITES, SPRITE_BYTES, SPRITE_COUNT, index)?;
        self.bytes[range.start + 180..range.start + 184].fill(0);
        Ok(())
    }
    /// Native41DD10 clears these five complete transient sprite records and
    /// texture identities before copying the live body to its save mirror.
    /// This operation requires an existing complete body; it cannot manufacture
    /// a live scene or certify that retained engine resources were captured.
    pub fn apply_checkpoint_sprite_fixups(&mut self) {
        for index in [11, 21, 31, 41, 51] {
            let start = SPRITES + index * SPRITE_BYTES;
            self.bytes[start..start + SPRITE_BYTES].fill(0);
        }
        for index in 0..SPRITE_COUNT {
            let start = SPRITES + index * SPRITE_BYTES + 180;
            self.bytes[start..start + 4].fill(0);
        }
    }
    pub fn read_run_index(&self) -> u32 {
        word(&self.bytes, READ_RUN_INDEX)
    }
    /// Bound only writes. Import/availability must preserve even an unknown
    /// or malformed index until a semantic restoration is explicitly attempted.
    pub fn set_read_run_index(&mut self, index: usize) -> Result<()> {
        ensure!(index < READ_RUN_COUNT, "native Circus read-run index");
        self.bytes[READ_RUN_INDEX..READ_RUN_INDEX + 4]
            .copy_from_slice(&(index as u32).to_le_bytes());
        Ok(())
    }
    pub fn read_run(&self, index: usize) -> Result<NativeReadRun> {
        let range = checked_record(READ_RUNS, READ_RUN_BYTES, READ_RUN_COUNT, index)?;
        let bytes = &self.bytes[range];
        let mut words = [0u16; 7];
        for (out, pair) in words.iter_mut().zip(bytes.chunks_exact(2)) {
            *out = u16::from_le_bytes(pair.try_into().unwrap());
        }
        Ok(NativeReadRun {
            scene_index: words[0],
            metadata_and_flags: words[1],
            first_label: words[2],
            last_label: words[3],
            source_word_55936: words[4],
            source_word_55930: words[5],
            source_word_55934: words[6],
        })
    }
    pub fn set_read_run(&mut self, index: usize, run: NativeReadRun) -> Result<()> {
        let range = checked_record(READ_RUNS, READ_RUN_BYTES, READ_RUN_COUNT, index)?;
        for (pair, value) in self.bytes[range].chunks_exact_mut(2).zip(run.words()) {
            pair.copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }
    pub fn movie_record(&self) -> &[u8] {
        &self.bytes[MOVIE..MOVIE + MOVIE_BYTES]
    }
    /// Pack already-obtained original civil-time fields. Obtaining local wall
    /// time is a host capability; this codec does not read OS or VM clocks.
    pub fn set_civil_time_fields(
        &mut self,
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<()> {
        ensure!(
            (1..=12).contains(&month)
                && (1..=31).contains(&day)
                && hour < 24
                && minute < 60
                && second < 60,
            "native Circus civil time fields"
        );
        self.bytes[NATIVE_DATE..NATIVE_DATE + 6].copy_from_slice(&[
            month,
            day,
            hour,
            minute,
            (year % 100) as u8,
            second,
        ]);
        Ok(())
    }
    pub fn civil_time_bytes(&self) -> [u8; 6] {
        self.bytes[NATIVE_DATE..NATIVE_DATE + 6].try_into().unwrap()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn patterned() -> Vec<u8> {
        (0..BODY_BYTES)
            .map(|i| ((i * 37 + 19) % 251) as u8)
            .collect()
    }
    #[test]
    fn exact_native_prefix_is_lossless_and_unknown_regions_survive() {
        let bytes = patterned();
        let body = NativeSaveBody::from_exact(&bytes).unwrap();
        assert_eq!(body.bytes(), bytes);
        let mut with_tail = bytes.clone();
        with_tail.extend(b"unread original trailer");
        assert_eq!(NativeSaveBody::from_read_prefix(&with_tail).unwrap(), body);
        assert!(NativeSaveBody::from_exact(&with_tail).is_err());
        assert!(NativeSaveBody::from_exact(&bytes[..BODY_BYTES - 1]).is_err());
        let mut patched = body.clone();
        patched.set_byte_bank(10, 99, 0xee).unwrap();
        let changed = patched
            .bytes()
            .iter()
            .zip(body.bytes())
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect::<Vec<_>>();
        assert_eq!(changed, [BYTE_BANKS + 1099]);
        let before = patched.clone();
        assert!(patched.set_byte_bank(11, 0, 1).is_err());
        assert_eq!(patched, before);
    }
    #[test]
    fn native_pc_names_and_capture_fixups_preserve_unrelated_bytes() {
        let mut body = NativeSaveBody::from_exact(&patterned()).unwrap();
        let before = body.clone();
        let pc = NativePcFields {
            decode_next: 0x83e,
            label_metadata: 17,
            label: 3,
            checkpoint_resume: 0x831,
        };
        body.set_pc_fields(pc);
        assert_eq!(body.pc_fields(), pc);
        body.set_image_name_bytes(50, b"BG\\room").unwrap();
        assert_eq!(body.image_name_bytes(50).unwrap(), b"BG\\room");
        let range = checked_record(IMAGE_NAMES, 48, 51, 50).unwrap();
        assert_eq!(
            &body.bytes()[range.start + 8..range.end],
            &before.bytes()[range.start + 8..range.end]
        );
        let before_name = body.clone();
        assert!(body.set_image_name_bytes(51, b"x").is_err());
        assert!(body.set_image_name_bytes(0, &[b'x'; 48]).is_err());
        assert_eq!(body, before_name);
        body.clear_captured_texture_identity(149).unwrap();
        assert_eq!(body.sprite(149).unwrap().transient_texture_identity(), 0);
        assert!(body.sprite(150).is_err());
        let changed = body
            .bytes()
            .iter()
            .zip(before.bytes())
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect::<Vec<_>>();
        assert!(changed.iter().all(|&i| (2804..2820).contains(&i)
            || (range.start..range.start + 8).contains(&i)
            || (SPRITES + 149 * 960 + 180..SPRITES + 149 * 960 + 184).contains(&i)));
    }
    #[test]
    fn original_civil_time_order_and_raw_float_bits_are_retained() {
        let mut body = NativeSaveBody::from_exact(&patterned()).unwrap();
        let before = body.clone();
        body.set_civil_time_fields(2026, 10, 31, 6, 40, 59).unwrap();
        assert_eq!(body.civil_time_bytes(), [10, 31, 6, 40, 26, 59]);
        let changed = body
            .bytes()
            .iter()
            .zip(before.bytes())
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect::<Vec<_>>();
        assert!(changed
            .iter()
            .all(|i| (NATIVE_DATE..NATIVE_DATE + 6).contains(i)));
        let before = body.clone();
        assert!(body.set_civil_time_fields(2026, 0, 1, 0, 0, 0).is_err());
        assert_eq!(body, before);
        let record = SPRITES;
        body.bytes[record..record + 4].copy_from_slice(&0x7fc01234u32.to_le_bytes());
        assert_eq!(body.sprite(0).unwrap().position()[0].to_bits(), 0x7fc01234);
    }
    #[test]
    fn checkpoint_fixups_match_all_150_native_records_only() {
        let mut body = NativeSaveBody::from_exact(&patterned()).unwrap();
        let before = body.clone();
        body.apply_checkpoint_sprite_fixups();
        for index in 0..SPRITE_COUNT {
            let sprite = body.sprite(index).unwrap();
            assert_eq!(sprite.transient_texture_identity(), 0);
            if [11, 21, 31, 41, 51].contains(&index) {
                assert!(sprite.bytes().iter().all(|b| *b == 0));
            } else {
                let old = before.sprite(index).unwrap();
                assert_eq!(&sprite.bytes()[..180], &old.bytes()[..180]);
                assert_eq!(&sprite.bytes()[184..], &old.bytes()[184..]);
            }
        }
        assert_eq!(&body.bytes()[..SPRITES], &before.bytes()[..SPRITES]);
        let end = SPRITES + SPRITE_COUNT * SPRITE_BYTES;
        assert_eq!(&body.bytes()[end..], &before.bytes()[end..]);
        let once = body.clone();
        body.apply_checkpoint_sprite_fixups();
        assert_eq!(body, once);
    }
    #[test]
    fn original_read_run_metadata_and_sentinel_survive_roundtrip() {
        let mut body = NativeSaveBody::from_exact(&patterned()).unwrap();
        let before = body.clone();
        let run = NativeReadRun {
            scene_index: u16::MAX,
            metadata_and_flags: 0x8123,
            first_label: 5,
            last_label: 7,
            source_word_55936: 11,
            source_word_55930: 13,
            source_word_55934: 17,
        };
        body.set_read_run(399, run).unwrap();
        body.set_read_run_index(399).unwrap();
        assert_eq!(body.read_run(399).unwrap(), run);
        assert_eq!(body.read_run_index(), 399);
        let record = READ_RUNS + 399 * READ_RUN_BYTES;
        assert_eq!(
            &body.bytes()[record..record + READ_RUN_BYTES],
            &[255, 255, 0x23, 0x81, 5, 0, 7, 0, 11, 0, 13, 0, 17, 0]
        );
        assert!(body
            .bytes()
            .iter()
            .zip(before.bytes())
            .enumerate()
            .all(|(i, (a, b))| a == b
                || (record..record + READ_RUN_BYTES).contains(&i)
                || (READ_RUN_INDEX..READ_RUN_INDEX + 4).contains(&i)));
        let before = body.clone();
        assert!(body.set_read_run(400, run).is_err());
        assert!(body.set_read_run_index(400).is_err());
        assert_eq!(body, before);
    }
}
