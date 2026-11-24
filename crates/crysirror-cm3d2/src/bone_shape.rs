// SPDX-License-Identifier: MPL-2.0
//! Retail BoneMorph.Init defines component-wise multipliers on the installed
//! skeleton. Read the literal registrations from that assembly once.
use crate::{binio::Reader, chara::Maid};
use anyhow::{Context, Result, bail, ensure};
use crysirror_asset::interchange::cm3d2::Bone;
use crysirror_dotnet::{Assembly, Token, table};
use glam::Vec3;
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
struct Rule {
    property: String,
    scale: bool,
    min: Vec3,
    max: Vec3,
}

pub(crate) struct Rules(BTreeMap<String, Arc<[Rule]>>);

fn expect(r: &mut Reader<'_>, instruction: u8) -> Result<()> {
    ensure!(
        r.take(1)?[0] == instruction,
        "unexpected BoneMorph.Init instruction at {}",
        r.at - 1
    );
    Ok(())
}

impl Rules {
    pub fn load(assembly: &Assembly) -> Result<Self> {
        let owner = assembly
            .find_type("BoneMorph")?
            .context("missing BoneMorph type")?;
        let method_name = |row| assembly.string(assembly.cell(table::METHOD_DEF, row, 3)?);
        let method = assembly
            .members(owner)?
            .1
            .find(|&m| method_name(m).ok() == Some("Init"))
            .context("missing BoneMorph.Init")?;
        let body = assembly.method_body(method)?;
        let mut r = Reader::new(body);
        expect(&mut r, 0x73)?; // newobj: lookup dictionary
        r.u32()?;
        expect(&mut r, 0x80)?; // stsfld
        r.u32()?;
        let mut rules: BTreeMap<String, Vec<Rule>> = BTreeMap::new();
        while body.get(r.at) != Some(&0x2a) {
            let mut string = || -> Result<String> {
                expect(&mut r, 0x72)?;
                let token = Token::from_u32(r.u32()?);
                ensure!(
                    token.table == 0x70,
                    "expected user string in BoneMorph.Init"
                );
                assembly.user_string(token.row)
            };
            let property = string()?;
            let bone = string()?;
            let mut values = [0.; 6];
            for v in &mut values {
                expect(&mut r, 0x22)?; // ldc.r4
                *v = r.f32()?;
                ensure!(v.is_finite(), "nonfinite bone shape coefficient");
            }
            expect(&mut r, 0x28)?;
            let called = Token::from_u32(r.u32()?);
            ensure!(
                called.table == table::METHOD_DEF && assembly.method_owner(called.row)? == owner,
                "foreign bone shape registration"
            );
            let scale = match method_name(called.row)? {
                "SetPosition" => false,
                "SetScale" => true,
                other => bail!("unknown bone shape registration {other}"),
            };
            let rule = Rule {
                property,
                scale,
                min: Vec3::from_slice(&values[..3]),
                max: Vec3::from_slice(&values[3..]),
            };
            if bone.contains('?') {
                for side in ["L", "R"] {
                    rules
                        .entry(bone.replace('?', side))
                        .or_default()
                        .push(rule.clone());
                }
            } else {
                rules.entry(bone).or_default().push(rule);
            }
        }
        ensure!(
            r.at + 1 == body.len() && !rules.is_empty(),
            "invalid BoneMorph.Init terminator"
        );
        Ok(Self(
            rules.into_iter().map(|(k, v)| (k, v.into())).collect(),
        ))
    }

    pub fn bind(&self, bones: &[Bone]) -> Vec<Binding> {
        let mut bound = Vec::new();
        for (index, bone) in bones.iter().enumerate() {
            let mut current = Some(index);
            let mut lower = false;
            // Some meshes attach the parent later in their serialized bone list.
            for _ in 0..bones.len() {
                let Some(node) = current.and_then(|i| bones.get(i)) else {
                    break;
                };
                if node.name.contains("Thigh") {
                    lower = true;
                    break;
                }
                current = node.parent;
            }
            let mut add = |name: &str, rest: Vec3| {
                if let Some(rules) = self.0.get(name) {
                    bound.push(Binding {
                        name: name.to_owned(),
                        rest,
                        lower,
                        rules: rules.clone(),
                    });
                }
            };
            let [x, y, z] = bone.translation;
            add(&bone.name, Vec3::new(-x, y, z));
            if bone.scale_node {
                add(&format!("{}_SCL_", bone.name), Vec3::ZERO);
            }
        }
        bound
    }
}

pub(crate) struct Binding {
    name: String,
    rest: Vec3,
    lower: bool,
    rules: Arc<[Rule]>,
}
impl Binding {
    pub fn apply(&self, maid: &Maid, nodes: &mut Map<String, Value>) {
        let mut pos = self.rest;
        let mut scale = Vec3::ONE;
        let (mut set_pos, mut set_scale) = (false, false);
        for rule in self.rules.iter() {
            let mut v = maid.value(&rule.property) as f32 / 100.;
            if rule.property == "DouPer" && !self.lower {
                v = 1. - v;
            }
            if rule.property == "EyePosX" {
                v *= 0.5 + maid.value("EyePosY") as f32 / 200.;
            }
            let multiplier = rule.min.lerp(rule.max, v.clamp(0., 1.));
            if rule.scale {
                scale *= multiplier;
                set_scale = true;
            } else {
                pos *= multiplier;
                set_pos = true;
            }
        }
        let entry = nodes.entry(self.name.clone()).or_insert_with(|| json!({}));
        if set_pos {
            entry["translation"] = json!(pos.to_array());
        }
        if set_scale {
            entry["scale"] = json!(scale.to_array());
        }
    }
}
