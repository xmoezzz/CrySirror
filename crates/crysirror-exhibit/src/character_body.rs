// SPDX-License-Identifier: MPL-2.0
//! Ordinary body descriptors. No image or scene is instantiated here.
use crate::branch::number;
use anyhow::{Result, ensure};

const FIELD_COUNT: usize = 213;
const FACE_VARIANTS: usize = 24;
const FACE_START: usize = 21;
const UNSET_PART: [&[u8]; 4] = [b"0", b"0", b"0", b"NULL"];
const DEFAULT_FACE: [&[u8]; 8] = [b"0", b"0", b"0", b"NULL", b"0", b"0", b"0", b"NULL"];
const FIELD_COUNT_2011: usize = 359;
const FACE_VARIANTS_2011: usize = 42;
const FACE_START_2011: usize = 23;

/// Preselected by native setFace (0x26690) for the admitted face mode 0.
/// Native nextESeq(0,-1) selects index 0 without touching the rotating index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaceProfile {
    NativeEye0AndMouthDefault,
}

#[derive(Debug, PartialEq)]
pub(super) struct Definition {
    pub character: u8,
    pub variant: u8,
    // Numeric body fields at +112/+116. Their later onstage consumers are
    // separate from defining a body. Keep the actual values, not defaults.
    body_words: [i32; 2],
    body_part: i32,
    corrected_origin: [i32; 2],
    face_origin: [i32; 2],
    faces: Box<[FaceProfile]>,
}

impl Definition {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() <= 4096, "oversized ExHibit v1 body definition");
        let fields: Vec<_> = raw.splitn(FIELD_COUNT + 1, |&b| b == b',').collect();
        ensure!(
            fields.len() == FIELD_COUNT,
            "unsupported ExHibit v1 body definition layout"
        );
        let n = |i: usize| number(fields[i]);
        let character = u8::try_from(n(0)?)?;
        let variant = u8::try_from(n(1)?)?;
        let body_words = [n(2)?, n(3)?];
        ensure!(
            (3..=34).contains(&character)
                && variant < 26
                && (0..=2).contains(&body_words[0])
                && body_words[1] == -1
                && fields[6..10] == [b"1", b"0", b"0", b"NULL".as_slice()]
                && fields[10..14] == UNSET_PART
                && fields[14..18] == UNSET_PART,
            "unsupported ExHibit v1 body/overlay profile"
        );
        let count = usize::try_from(n(18)?)?;
        ensure!(
            (1..=FACE_VARIANTS).contains(&count),
            "unsupported ExHibit v1 face count"
        );
        // In the pinned corpus, inactive tuples are also canonical. Keep
        // rejecting overrides, rather than guessing authored animation data.
        ensure!(
            fields[FACE_START..]
                .chunks_exact(8)
                .all(|f| f == DEFAULT_FACE),
            "unsupported ExHibit v1 face or sequence override"
        );
        let corrected_origin = [n(4)?, n(5)?];
        let face_origin = [n(19)?, n(20)?];
        ensure!(
            corrected_origin
                .into_iter()
                .chain(face_origin)
                .all(|v| (-8192..=8192).contains(&v)),
            "unsupported ExHibit v1 body origin"
        );
        Ok(Self {
            character,
            variant,
            body_words,
            body_part: 1,
            corrected_origin,
            face_origin,
            faces: vec![FaceProfile::NativeEye0AndMouthDefault; count].into_boxed_slice(),
        })
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Definition2011 {
    pub character: u8,
    pub variant: u8,
    body_words: [i32; 2],
    body_part: i32,
    corrected_origin: [i32; 2],
    secondary_correction: [i32; 2],
    face_origin: [i32; 2],
    faces: Box<[FaceProfile]>,
}

impl Definition2011 {
    pub fn immediate_face(&self, face: u8) -> Result<crate::midori_actor_image::FacePlan> {
        ensure!(
            usize::from(face) < self.faces.len(),
            "invalid Midori actor face"
        );
        Ok(crate::midori_actor_image::FacePlan {
            actor: self.character,
            body: self.variant,
            face,
            cells: self.faces.len() as u32,
            face_origin: self.face_origin,
            correction: self.corrected_origin,
        })
    }

    pub fn parse(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() <= 4096, "oversized ExHibit 2011 body definition");
        let fields: Vec<_> = raw.splitn(FIELD_COUNT_2011 + 1, |&b| b == b',').collect();
        ensure!(
            fields.len() == FIELD_COUNT_2011,
            "unsupported ExHibit 2011 body layout"
        );
        let n = |i: usize| number(fields[i]);
        let character = u8::try_from(n(0)?)?;
        let variant = u8::try_from(n(1)?)?;
        let body_words = [n(2)?, n(3)?];
        let secondary_correction = [n(6)?, n(7)?];
        ensure!(
            (3..=36).contains(&character)
                && variant < 36
                && (0..=4).contains(&body_words[0])
                && body_words[1] == -1
                && secondary_correction == [-12345; 2]
                && fields[8..12] == [b"1", b"0", b"0", b"NULL".as_slice()]
                && fields[12..16] == UNSET_PART
                && fields[16..20] == UNSET_PART,
            "unsupported ExHibit 2011 body or overlay profile"
        );
        let count = usize::try_from(n(20)?)?;
        ensure!(
            (1..=FACE_VARIANTS_2011).contains(&count),
            "invalid Midori face count"
        );
        ensure!(
            fields[FACE_START_2011..]
                .chunks_exact(8)
                .all(|f| f == DEFAULT_FACE),
            "unsupported ExHibit 2011 face or sequence override"
        );
        let corrected_origin = [n(4)?, n(5)?];
        let face_origin = [n(21)?, n(22)?];
        ensure!(
            corrected_origin
                .into_iter()
                .chain(face_origin)
                .all(|v| (-8192..=8192).contains(&v)),
            "unsupported ExHibit 2011 body origin"
        );
        // 0x37ac0 stores both explicit correction sentinels. The default
        // eye/mouth pair at 0x38d70 selects eye 0 without rotating it.
        Ok(Self {
            character,
            variant,
            body_words,
            body_part: 1,
            corrected_origin,
            secondary_correction,
            face_origin,
            faces: vec![FaceProfile::NativeEye0AndMouthDefault; count].into_boxed_slice(),
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub fn payload_2011(actor: u8, variant: u8) -> Vec<u8> {
        let mut fields = [
            actor.to_string(),
            variant.to_string(),
            "4".into(),
            "-1".into(),
            "-106".into(),
            "820".into(),
            "-12345".into(),
            "-12345".into(),
            "1".into(),
            "0".into(),
            "0".into(),
            "NULL".into(),
            "0".into(),
            "0".into(),
            "0".into(),
            "NULL".into(),
            "0".into(),
            "0".into(),
            "0".into(),
            "NULL".into(),
            "11".into(),
            "568".into(),
            "333".into(),
        ]
        .to_vec();
        for _ in 0..FACE_VARIANTS_2011 {
            for b in DEFAULT_FACE {
                fields.push(std::str::from_utf8(b).unwrap().into());
            }
        }
        fields.join(",").into_bytes()
    }

    #[test]
    fn midori_body_keeps_second_origin_and_default_face_count_without_v1_alias() -> Result<()> {
        let raw = payload_2011(9, 35);
        let def = Definition2011::parse(&raw)?;
        assert_eq!((def.character, def.variant, def.body_part), (9, 35, 1));
        assert_eq!(def.body_words, [4, -1]);
        assert_eq!(def.corrected_origin, [-106, 820]);
        assert_eq!(def.secondary_correction, [-12345; 2]);
        assert_eq!(def.face_origin, [568, 333]);
        assert_eq!(
            def.faces.as_ref(),
            [FaceProfile::NativeEye0AndMouthDefault; 11]
        );
        assert!(Definition::parse(&raw).is_err());
        let mut fields: Vec<_> = raw.split(|b| *b == b',').collect();
        fields[7] = b"0";
        assert!(Definition2011::parse(&fields.join(&b',')).is_err());
        fields[7] = b"-12345";
        fields[16] = b"1";
        assert!(Definition2011::parse(&fields.join(&b',')).is_err());
        fields[16] = b"0";
        fields[26] = b"other-sequence";
        assert!(Definition2011::parse(&fields.join(&b',')).is_err());
        Ok(())
    }

    pub fn payload(actor: u8, variant: u8) -> Vec<u8> {
        let mut fields = [
            actor.to_string(),
            variant.to_string(),
            "2".into(),
            "-1".into(),
            "-170".into(),
            "0".into(),
            "1".into(),
            "0".into(),
            "0".into(),
            "NULL".into(),
            "0".into(),
            "0".into(),
            "0".into(),
            "NULL".into(),
            "0".into(),
            "0".into(),
            "0".into(),
            "NULL".into(),
            "3".into(),
            "129".into(),
            "86".into(),
        ]
        .to_vec();
        for _ in 0..FACE_VARIANTS {
            for b in DEFAULT_FACE {
                fields.push(std::str::from_utf8(b).unwrap().into());
            }
        }
        fields.join(",").into_bytes()
    }

    #[test]
    fn ordinary_body_retains_signed_correction_capacity_and_default_face_programs() -> Result<()> {
        let raw = payload(5, 11);
        let def = Definition::parse(&raw)?;
        assert_eq!((def.character, def.variant, def.body_part), (5, 11, 1));
        assert_eq!(def.body_words, [2, -1]);
        assert_eq!(def.corrected_origin, [-170, 0]);
        assert_eq!(def.face_origin, [129, 86]);
        assert_eq!(
            def.faces.as_ref(),
            [FaceProfile::NativeEye0AndMouthDefault; 3]
        );
        let mut fields: Vec<_> = raw.split(|b| *b == b',').collect();
        fields[24] = b"A...";
        assert!(Definition::parse(&fields.join(&b',')).is_err());
        fields[24] = b"NULL";
        fields[10] = b"1";
        assert!(Definition::parse(&fields.join(&b',')).is_err());
        assert!(Definition::parse(&raw[..raw.len() - 1]).is_err());
        Ok(())
    }
}
