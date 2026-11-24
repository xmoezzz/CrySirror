// SPDX-License-Identifier: MPL-2.0
//! Midori native local-position work and its detached T7 text register.
use anyhow::{Result, bail};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PositionCommand {
    Reset,
    Global([i32; 2]),
}

impl PositionCommand {
    pub fn read(flags: u8, args: &[i32]) -> Result<Self> {
        match (flags, args) {
            // Native cmdLocalPosCorrect (0x10fa60). Mode 1's -12345
            // selector goes straight to the two global fields (0xbf9c0);
            // the second selector word is ignored in this branch.
            (0, [0]) => Ok(Self::Reset),
            (0, [1, -12345, _, x, y]) => Ok(Self::Global([*x, *y])),
            _ => bail!("unsupported Midori per-actor or execute local position correction"),
        }
    }
}

pub(super) struct Work {
    global: [i32; 2],
}
impl Work {
    pub fn begin_script() -> Self {
        // liteExec 0x145fe0 empties the local correction list, then
        // explicitly zeroes both offsets. No arbitrary camera modifier.
        Self { global: [0, 0] }
    }

    pub fn apply(&mut self, command: PositionCommand) {
        self.global = match command {
            PositionCommand::Reset => [0, 0],
            PositionCommand::Global(xy) => xy,
        };
    }

    pub fn capture(&self) -> (u32, String) {
        // blockEntry calls 0xdab80 with the actual LocalPosCorrector;
        // member rows (unadmitted) are empty, and field 1 is literal 0.
        (7, format!("0,0,{},{}", self.global[0], self.global[1]))
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
        work.apply(command(&[1, -12345, 999, i32::MIN, 7])?);
        assert_eq!(work.capture(), (7, "0,0,-2147483648,7".into()));
        assert!(command(&[1, 2, 999, 3, 4]).is_err());
        assert!(command(&[2, -12345, 0]).is_err());
        assert!(command(&[3]).is_err());
        assert_eq!(work.capture(), (7, "0,0,-2147483648,7".into()));
        work.apply(command(&[0])?);
        assert_eq!(work.capture(), (7, "0,0,0,0".into()));
        work.apply(command(&[1, -12345, 0, 5, -11])?);
        assert_eq!(work.capture(), (7, "0,0,5,-11".into()));
        let next = Work::begin_script();
        assert_eq!(next.capture(), (7, "0,0,0,0".into()));
        Ok(())
    }
}
