// SPDX-License-Identifier: MPL-2.0
//! RLD 2011 cmdChangeScenario and its system-bank commit boundary.
use crate::{Script, branch::Condition, native_numbers::integer2011, registers::Registers};
use anyhow::{Result, ensure};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_tasks::Task;

#[derive(Debug, PartialEq)]
pub(super) struct Command {
    target: Vec<u8>,
    mode: u8,
    pub when: Condition,
    pub reset_scene_flags: bool,
}

impl Command {
    pub fn parse(raw: &[u8], condition: Option<&[u8]>, csv: bool) -> Result<Self> {
        let mut fields = raw.split(|&b| b == b',');
        let (target, when, mode) = if csv {
            (
                fields.next().unwrap_or_default(),
                fields.next().unwrap_or_default(),
                integer2011(fields.next().unwrap_or_default()) as u8 & 3,
            )
        } else {
            (raw, condition.unwrap_or_default(), 0)
        };
        Ok(Self {
            target: target.to_vec(),
            mode,
            when: Condition::for_abi(when, 3)?,
            reset_scene_flags: csv || condition.is_some(),
        })
    }

    pub fn destination(
        &self,
        tables: &Registers,
        registers: &Registers,
    ) -> Result<Option<Destination>> {
        let source = match self.mode {
            0 => self.target.as_slice(),
            1 => registers.text(integer2011(&self.target))?,
            2 => tables
                .first_transfer_route(integer2011(&self.target), registers)?
                .unwrap_or_default(),
            3 => &[],
            _ => unreachable!(),
        };
        if source.is_empty() {
            return Ok(None);
        }
        let name = crysirror_resource::text_stream::decode_shift_jis(source, 512)?;
        ensure!(
            !matches!(name.as_str(), "." | "..") && !name.contains(['/', '\\']),
            "invalid ExHibit scenario transfer name"
        );
        let path = VfsPath::new(Mount::Asset, &format!("rld/{name}.rld"))?;
        let mut native_path = br"rld\".to_vec();
        native_path.extend_from_slice(source);
        native_path.extend_from_slice(b".rld");
        Ok(Some(Destination {
            path,
            name,
            native_path,
        }))
    }
}

pub(super) struct Destination {
    pub path: VfsPath,
    pub name: String,
    pub native_path: Vec<u8>,
}

pub(super) struct Pending {
    pub task: Task<Option<(String, Script)>>,
    pub registers: Registers,
}
