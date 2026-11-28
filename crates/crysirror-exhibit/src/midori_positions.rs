// SPDX-License-Identifier: MPL-2.0
//! Midori native local-position work and its detached T7 text register.
use anyhow::{Result, bail, ensure};
use std::fmt::Write;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PositionCommand {
    Reset,
    Global([i32; 2]),
    Set { key: [i32; 2], delta: [i32; 2] },
    Remove([i32; 2]),
    Execute,
    UnusedMode,
}

impl PositionCommand {
    pub fn read(flags: u8, args: &[i32]) -> Result<Self> {
        match (flags, args) {
            // Native cmdLocalPosCorrect (0x10fa60). Mode 1's -12345
            // selector goes straight to the two global fields (0xbf9c0);
            // the second selector word is ignored in this branch.
            (0, [0]) => Ok(Self::Reset),
            (0, [1, -12345, _, x, y]) => Ok(Self::Global([*x, *y])),
            (0, [1, actor, pose, x, y]) => Ok(Self::Set {
                key: [*actor, *pose],
                delta: [*x, *y],
            }),
            (0, [2, actor, pose]) => Ok(Self::Remove([*actor, *pose])),
            (0, [3]) => Ok(Self::Execute),
            (0, [mode, ..]) if !(0..=3).contains(mode) => Ok(Self::UnusedMode),
            _ => bail!("unsupported Midori local position command shape"),
        }
    }
}

pub(super) struct Work {
    global: [i32; 2],
    rows: Vec<([i32; 2], [i32; 2])>,
}
impl Work {
    pub fn begin_script() -> Self {
        // liteExec 0x145fe0 empties the local correction list, then
        // explicitly zeroes both offsets. No arbitrary camera modifier.
        Self {
            global: [0, 0],
            rows: Vec::new(),
        }
    }

    pub fn apply(&mut self, command: PositionCommand) -> Result<()> {
        match command {
            PositionCommand::Reset => {
                self.rows.clear();
                self.global = [0, 0];
            }
            PositionCommand::Global(xy) => self.global = xy,
            PositionCommand::Set { key, delta } => {
                // Lookup at 0xda330 compares both words exactly; 0x195220
                // appends new keys while an update retains list position.
                if let Some(row) = self.rows.iter_mut().find(|row| row.0 == key) {
                    row.1 = delta;
                } else {
                    ensure!(
                        self.rows.len() < 65_536,
                        "Midori correction list exceeds session budget"
                    );
                    self.rows.push((key, delta));
                }
            }
            PositionCommand::Remove(key) => {
                if let Some(index) = self.rows.iter().position(|row| row.0 == key) {
                    self.rows.remove(index);
                }
            }
            PositionCommand::Execute => {
                // 0x1005c0 returns before consulting the character manager
                // when there are no rows, including a global-only list.
                ensure!(
                    self.rows.is_empty(),
                    "Midori correction execution needs entered-character positions"
                );
            }
            PositionCommand::UnusedMode => {}
        }
        Ok(())
    }

    pub fn capture(&self) -> (u32, String) {
        // 0xdab80 walks in insertion order. Mode-1 records (0xbf9c0)
        // have the position bit and default secondary words.
        let mut text = format!(
            "{},0,{},{}",
            self.rows.len(),
            self.global[0],
            self.global[1]
        );
        for ([actor, pose], [x, y]) in &self.rows {
            write!(&mut text, ";1,{actor},{pose},{x},{y},-12345,-12345")
                .expect("writing to String");
        }
        (7, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Script};

    fn command(args: &[i32]) -> Result<PositionCommand> {
        let mut data = b"\0DLR".to_vec();
        for x in [3u32, 12, 1] {
            data.extend(x.to_le_bytes());
        }
        data.extend([0x6c, 0, args.len() as u8, 0]);
        for arg in args {
            data.extend(arg.to_le_bytes());
        }
        let Action::MidoriPosition(cmd) = Script::new(data, 3)?.next()? else {
            bail!("missing Midori native local positioning");
        };
        Ok(cmd)
    }

    #[test]
    fn position_capture_follows_native_set_reset_and_script_start() -> Result<()> {
        let mut work = Work::begin_script();
        assert_eq!(work.capture(), (7, "0,0,0,0".into()));
        work.apply(command(&[1, -12345, 999, i32::MIN, 7])?)?;
        assert_eq!(work.capture(), (7, "0,0,-2147483648,7".into()));
        work.apply(command(&[3])?)?;
        work.apply(command(&[1, 2, 999, 3, 4])?)?;
        work.apply(command(&[1, 2, -12345, 12, 24])?)?;
        work.apply(command(&[1, 2, 999, -9, -8])?)?;
        work.apply(command(&[2, -12345, 0])?)?;
        assert!(work.apply(command(&[3])?).is_err());
        assert_eq!(
            work.capture().1,
            "2,0,-2147483648,7;1,2,999,-9,-8,-12345,-12345;1,2,-12345,12,24,-12345,-12345"
        );
        work.apply(command(&[2, 2, 999])?)?;
        work.apply(command(&[4])?)?;
        assert_eq!(
            work.capture().1,
            "1,0,-2147483648,7;1,2,-12345,12,24,-12345,-12345"
        );
        work.apply(command(&[0])?)?;
        assert_eq!(work.capture(), (7, "0,0,0,0".into()));
        work.apply(command(&[1, -12345, 0, 5, -11])?)?;
        assert_eq!(work.capture(), (7, "0,0,5,-11".into()));
        let next = Work::begin_script();
        assert_eq!(next.capture(), (7, "0,0,0,0".into()));
        Ok(())
    }
}
