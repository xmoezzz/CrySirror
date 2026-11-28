// SPDX-License-Identifier: MPL-2.0
//! Detached CharacterMgr record for standalone COM3D2 1.0.4.
use super::{player_1040::Player, status_1040::Status};
use crate::{binio::Writer, parts_color::PartsColor, tjs::retail};
use anyhow::{Context, Result, ensure};
use crysirror_asset::interchange::cm3d2::Reader;
use std::borrow::Cow;

pub(crate) type Properties<'a> = Vec<(String, Cow<'a, [u8]>)>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Misc<'a> {
    pub active_slot: i32,
    pub thumbnail: Cow<'a, [u8]>,
    pub thumbnail_time: String,
    pub color: [f32; 4],
    pub voice_pitch: i32,
}

impl<'a> Misc<'a> {
    fn read(r: &mut Reader<'a>) -> Result<Self> {
        read_header(r, "CM3D2_MAID_MISC")?;
        let active_slot = r.i32()?;
        let size = r.count(1)?;
        Ok(Self {
            active_slot,
            thumbnail: Cow::Borrowed(r.take(size)?),
            thumbnail_time: r.string()?,
            color: r.f32s()?,
            voice_pitch: r.i32()?,
        })
    }

    fn write(&self, w: &mut Writer) -> Result<()> {
        write_header(w, "CM3D2_MAID_MISC");
        w.i32(self.active_slot);
        w.i32(i32::try_from(self.thumbnail.len())?);
        w.bytes.extend_from_slice(&self.thumbnail);
        w.str(&self.thumbnail_time);
        for channel in self.color {
            w.f32(channel);
        }
        w.i32(self.voice_pitch);
        Ok(())
    }

    pub(crate) fn to_owned(&self) -> Misc<'static> {
        Misc {
            active_slot: self.active_slot,
            thumbnail: Cow::Owned(self.thumbnail.to_vec()),
            thumbnail_time: self.thumbnail_time.clone(),
            color: self.color,
            voice_pitch: self.voice_pitch,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Man<'a> {
    pub properties: Properties<'a>,
    pub misc: Misc<'a>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Employee<'a> {
    pub properties: Properties<'a>,
    pub colors: [PartsColor; 9],
    pub(crate) status: Status,
    pub misc: Misc<'a>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Roster<'a> {
    pub player: Player,
    pub men: Vec<Man<'a>>,
    pub employees: Vec<Employee<'a>>,
    /// Offset of ScriptManager's header in the original slot.
    pub script_offset: usize,
}

fn read_header(r: &mut Reader<'_>, section: &str) -> Result<()> {
    ensure!(
        r.header(section)? == 1040,
        "unsupported standalone {section} version"
    );
    Ok(())
}

fn write_header(w: &mut Writer, section: &str) {
    w.str(section);
    w.i32(1040);
}

fn write_props(w: &mut Writer, properties: &Properties<'_>) -> Result<()> {
    write_header(w, "CM3D2_MPROP_LIST");
    w.i32(i32::try_from(properties.len())?);
    for (key, bytes) in properties {
        w.str(key);
        w.bytes.extend_from_slice(bytes);
    }
    Ok(())
}

fn read_props<'a>(r: &mut Reader<'a>, source: &'a [u8]) -> Result<Properties<'a>> {
    Ok(retail::stock_props(r, source)?
        .into_iter()
        .map(|(key, bytes)| (key, Cow::Borrowed(bytes)))
        .collect())
}

fn write_colors(w: &mut Writer, colors: &[PartsColor; 9]) {
    write_header(w, "CM3D2_MULTI_COL");
    w.i32(9);
    for c in colors {
        w.bool(c.used);
        for value in [
            c.main_hue,
            c.main_chroma,
            c.main_brightness,
            c.main_contrast,
            c.shadow_rate,
            c.shadow_hue,
            c.shadow_chroma,
            c.shadow_brightness,
            c.shadow_contrast,
        ] {
            w.i32(value);
        }
    }
}

impl Roster<'_> {
    /// Encode CharacterMgr only. GameMain owns the outer header and bookmarks,
    /// and ScriptManager owns the following record.
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::default();
        write_header(&mut w, "CM3D2_CHR_MGR");
        w.bytes.extend(self.player.encode()?);
        w.i32(i32::try_from(self.men.len())?);
        for man in &self.men {
            write_props(&mut w, &man.properties)?;
            man.misc.write(&mut w)?;
        }
        w.i32(i32::try_from(self.employees.len())?);
        for employee in &self.employees {
            write_props(&mut w, &employee.properties)?;
            write_colors(&mut w, &employee.colors);
            employee.status.write(&mut w)?;
            employee.misc.write(&mut w)?;
            // Maid.SerializeBody writes only this versioned header.
            write_header(&mut w, "CM3D2_MAID_BODY");
        }
        Ok(w.bytes)
    }
}

pub(super) fn read(source: &[u8], stock_start: usize, player: Player) -> Result<Roster<'_>> {
    let mut r = Reader::new(source);
    // Even an empty man record has both versioned headers and fixed misc fields.
    let count = r.count(36)?;
    let mut men = Vec::new();
    for index in 0..count {
        let mut record = || -> Result<_> {
            Ok(Man {
                properties: read_props(&mut r, source)?,
                misc: Misc::read(&mut r)?,
            })
        };
        men.push(record().with_context(|| format!("stock man {index}"))?);
    }
    let count = r.count(333)?;
    let mut employees = Vec::new();
    for index in 0..count {
        let mut record = || -> Result<_> {
            let properties = read_props(&mut r, source)?;
            let mut colors_header = Reader::new(&source[r.position()..]);
            read_header(&mut colors_header, "CM3D2_MULTI_COL")?;
            let colors = retail::read_retail_colors(&mut r)?;
            let status = Status::read(&mut r)?;
            let misc = Misc::read(&mut r)?;
            read_header(&mut r, "CM3D2_MAID_BODY")?;
            Ok(Employee {
                properties,
                colors,
                status,
                misc,
            })
        };
        employees.push(record().with_context(|| format!("stock employee {index}"))?);
    }
    let script_offset = stock_start + r.position();
    read_header(&mut r, "CM3D2_SCRIPT")?;
    Ok(Roster {
        player,
        men,
        employees,
        script_offset,
    })
}

#[cfg(test)]
pub(crate) fn read_slot(bytes: &[u8]) -> Result<Roster<'_>> {
    let prefix = super::player_1040::read_slot_prefix(bytes)?;
    read(
        &bytes[prefix.stock_start..prefix.bookmarks[0]],
        prefix.stock_start,
        prefix.player,
    )
}
