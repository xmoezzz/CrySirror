// SPDX-License-Identifier: MPL-2.0
//! Scoped character definitions and v1 unpositioned character-voice policy.
use crate::{
    branch::number,
    character_body,
    registers::{Registers, Value},
};
use anyhow::{Context, Result, ensure};
use crysirror_resource::text_stream::decode_shift_jis;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Profile {
    V1,
    Midori2011,
}

#[derive(Debug, PartialEq)]
pub(super) struct Definition {
    id: u8,
    variants: u8,
    voice_enabled: bool,
    expression: Vec<u8>,
    // Four names, three call-me forms and three call-you forms (0x265b0).
    forms: [Vec<u8>; 10],
    reg_base: i32,
    reg_span: u8,
    profile: Profile,
}

impl Definition {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        Self::parse_profile(raw, Profile::V1)
    }

    pub fn parse_2011(raw: &[u8]) -> Result<Self> {
        Self::parse_profile(raw, Profile::Midori2011)
    }

    fn parse_profile(raw: &[u8], profile: Profile) -> Result<Self> {
        ensure!(raw.len() <= 2048, "oversized ExHibit character definition");
        let fields: Vec<_> = raw.splitn(15, |&b| b == b',').collect();
        ensure!(
            fields.len() == 14,
            "unsupported ExHibit character definition layout"
        );
        let id = u8::try_from(number(fields[0])?)?;
        let variants = u8::try_from(number(fields[1])?)?;
        let voice_enabled = match number(fields[2])? {
            0 => false,
            1 => true,
            _ => anyhow::bail!("unsupported ExHibit character voice field"),
        };
        let (max_id, max_variants, base, span) = match profile {
            Profile::V1 => (34, 26, 100, 8),
            Profile::Midori2011 => (36, 36, 200, 4),
        };
        ensure!(
            (3..=max_id).contains(&id) && variants <= max_variants,
            "unsupported ExHibit ordinary character range"
        );
        for &field in &fields[3..] {
            let text = decode_shift_jis(field, 128).context("ExHibit character field encoding")?;
            ensure!(
                !text.chars().any(char::is_control),
                "unsupported control in ExHibit character definition"
            );
        }
        Ok(Self {
            id,
            variants,
            voice_enabled,
            expression: fields[3].to_vec(),
            forms: std::array::from_fn(|i| fields[4 + i].to_vec()),
            // setReg stores [base,len] as descriptor metadata; it does not clear R.
            reg_base: base + i32::from(span) * i32::from(id),
            reg_span: span,
            profile,
        })
    }
}

pub(super) struct Catalog {
    capacity: usize,
    definitions: BTreeMap<u8, Definition>,
    bodies: BTreeMap<(u8, u8), character_body::Definition>,
    bodies_2011: BTreeMap<(u8, u8), character_body::Definition2011>,
}
impl Catalog {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            definitions: BTreeMap::new(),
            bodies: BTreeMap::new(),
            bodies_2011: BTreeMap::new(),
        }
    }

    pub fn define(&mut self, definition: Definition) -> Result<()> {
        let id = definition.id;
        ensure!(
            usize::from(id) < self.capacity && !self.definitions.contains_key(&id),
            "unsupported ExHibit duplicate or out-of-range character {id}"
        );
        self.definitions.insert(id, definition);
        Ok(())
    }

    pub fn define_body(&mut self, definition: character_body::Definition) -> Result<()> {
        let key = (definition.character, definition.variant);
        let actor = self
            .definitions
            .get(&key.0)
            .context("undefined ExHibit body character")?;
        ensure!(
            actor.profile == Profile::V1
                && key.1 < actor.variants
                && !self.bodies.contains_key(&key),
            "unsupported ExHibit duplicate or out-of-range body variant"
        );
        self.bodies.insert(key, definition);
        Ok(())
    }

    pub fn define_body_2011(&mut self, definition: character_body::Definition2011) -> Result<()> {
        let key = (definition.character, definition.variant);
        let actor = self
            .definitions
            .get(&key.0)
            .context("undefined ExHibit 2011 body character")?;
        ensure!(
            actor.profile == Profile::Midori2011
                && key.1 < actor.variants
                && !self.bodies_2011.contains_key(&key),
            "unsupported ExHibit 2011 duplicate or out-of-range body variant"
        );
        self.bodies_2011.insert(key, definition);
        Ok(())
    }

    pub fn body_2011(&self, actor: u8, variant: u8) -> Option<&character_body::Definition2011> {
        self.bodies_2011.get(&(actor, variant))
    }

    pub fn body_count(&self) -> usize {
        self.bodies.len() + self.bodies_2011.len()
    }

    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    /// The 0x288a0 unpositioned, non-skip path only. The character has not
    /// entered a scene or acquired pan, lip-sync or loop-ducking state.
    pub fn unpositioned_voice_gain(&self, id: u8, regs: &Registers) -> Result<Option<[u32; 2]>> {
        let actor = self
            .definitions
            .get(&id)
            .context("undefined ExHibit voice character")?;
        ensure!(
            actor.profile == Profile::V1,
            "unsupported ExHibit 2011 character voice profile"
        );
        if !actor.voice_enabled {
            return Ok(None);
        }
        let s = |i| regs.read(Value::System(i));
        let gain = if s(30)? != 0 {
            let mute = 43 + 2 * i32::from(id);
            if s(mute)? != 0 {
                return Ok(None);
            }
            [s(mute + 1)?; 2]
        } else {
            if s(40)? != 0 {
                return Ok(None);
            }
            [s(41)?, s(42)?]
        };
        ensure!(
            gain.iter().all(|v| (0..=256).contains(v)),
            "unsupported ExHibit character voice gain"
        );
        // Native multiplies the unpositioned unity 256 by each Q8 register
        // and then divides by 256. Zero gain is distinct from the mute bit.
        Ok(Some(gain.map(|n| n as u32)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn definition(id: u8, voice: bool) -> Result<Definition> {
        let mut fields = vec![
            id.to_string().into_bytes(),
            b"24".to_vec(),
            u8::from(voice).to_string().into_bytes(),
            vec![0x93, 0xfa],
            b"Example".to_vec(),
        ];
        fields.resize(14, vec![]);
        Definition::parse(&fields.join(&b','))
    }

    #[test]
    fn authored_voice_and_system_modes_are_independent_of_effect_gain() -> Result<()> {
        let mut chars = Catalog::new(37);
        let character = definition(5, true)?;
        assert_eq!(character.variants, 24);
        assert_eq!(character.reg_base, 140);
        assert_eq!(character.expression, [0x93, 0xfa]);
        assert_eq!(character.forms[0], b"Example");
        chars.define(character)?;
        chars.define(definition(6, false)?)?;
        assert!(chars.define(definition(5, false)?).is_err());
        assert!(Catalog::new(5).define(definition(5, true)?).is_err());
        let mut r = crate::native_state_tests::fresh(1)?;
        r.set_system(38, 11)?;
        assert_eq!(chars.unpositioned_voice_gain(5, &r)?, Some([128, 128]));
        assert_eq!(chars.unpositioned_voice_gain(6, &r)?, None);
        assert!(chars.unpositioned_voice_gain(7, &r).is_err());
        r.set_system(54, 0)?;
        assert_eq!(chars.unpositioned_voice_gain(5, &r)?, Some([0, 0]));
        r.set_system(53, 1)?;
        assert_eq!(chars.unpositioned_voice_gain(5, &r)?, None);
        r.set_system(30, 0)?;
        r.set_system(41, 64)?;
        r.set_system(42, 224)?;
        assert_eq!(chars.unpositioned_voice_gain(5, &r)?, Some([64, 224]));
        r.set_system(42, -1)?;
        assert!(chars.unpositioned_voice_gain(5, &r).is_err());
        r.set_system(40, 1)?;
        assert_eq!(chars.unpositioned_voice_gain(5, &r)?, None);
        Ok(())
    }

    #[test]
    fn body_install_requires_its_actor_and_variant_slot_without_touching_voice() -> Result<()> {
        let parse =
            |id, body| character_body::Definition::parse(&character_body::tests::payload(id, body));
        let mut chars = Catalog::new(37);
        assert!(chars.define_body(parse(5, 11)?).is_err());
        chars.define(definition(5, true)?)?;
        assert!(chars.define_body(parse(5, 24)?).is_err());
        chars.define_body(parse(5, 11)?)?;
        assert!(chars.define_body(parse(5, 11)?).is_err());
        assert_eq!(chars.body_count(), 1);
        assert_eq!(
            chars.unpositioned_voice_gain(5, &crate::native_state_tests::fresh(1)?)?,
            Some([128, 128])
        );
        Ok(())
    }

    #[test]
    fn midori_authored_definition_and_body_dispatch_have_distinct_register_ownership() -> Result<()>
    {
        use crate::{Action, Script};
        let mut data = b"\0DLR".to_vec();
        for n in [3u32, 12, 3] {
            data.extend(n.to_le_bytes());
        }
        data.extend([0x30, 0, 0, 1]);
        let mut args = vec![b"9".as_slice(), b"36", b"1", b"example", b"Test"];
        args.resize(14, b"");
        data.extend(args.join(&b','));
        data.push(0);
        for variant in [26, 35] {
            data.extend([0x31, 0, 0, 0x11]);
            data.extend(character_body::tests::payload_2011(9, variant));
            data.push(0);
        }
        let mut script = Script::new(data, 3)?;
        let Action::DefineCharacter(actor) = script.next()? else {
            anyhow::bail!("expected Midori actor definition");
        };
        assert_eq!(
            (actor.reg_base, actor.reg_span, actor.variants),
            (236, 4, 36)
        );
        assert_eq!(actor.profile, Profile::Midori2011);
        assert!(Definition::parse(&args.join(&b',')).is_err());
        let mut actors = Catalog::new(37);
        actors.define(actor)?;
        for i in 0..2 {
            let Action::DefineBody2011(body) = script.next()? else {
                anyhow::bail!("expected Midori body definition");
            };
            assert_eq!(body.variant, [26, 35][i]);
            actors.define_body_2011(body)?;
        }
        assert_eq!(actors.body_count(), 2);
        let duplicate =
            character_body::Definition2011::parse(&character_body::tests::payload_2011(9, 26))?;
        assert!(actors.define_body_2011(duplicate).is_err());
        let v1 = character_body::Definition::parse(&character_body::tests::payload(9, 10))?;
        assert!(actors.define_body(v1).is_err());
        assert!(
            actors
                .unpositioned_voice_gain(9, &crate::native_state_tests::fresh(3)?)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn unexpected_schema_range_and_code_page_are_rejected() {
        assert!(Definition::parse(b"5,24,1").is_err());
        assert!(Definition::parse(b"5,24,2,k,,,,,,,,,,").is_err());
        assert!(Definition::parse(b"5,27,1,k,,,,,,,,,,").is_err());
        assert!(Definition::parse(b"35,24,1,k,,,,,,,,,,").is_err());
        assert!(Definition::parse(b"5,24,1,\x82,,,,,,,,,,").is_err());
    }
}
