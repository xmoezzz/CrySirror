// SPDX-License-Identifier: GPL-3.0-or-later
// Archive policy adapted from arc_unpacker, Copyright (C) 2016 rr-.
// Deliberately non-executable Ren'Py pickle subset, matching arc_unpacker.
use super::*;
pub(super) fn rpa(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, length(s)?.min(34), 34)?;
    ensure!(
        h.starts_with(b"RPA-2.0 ") || h.starts_with(b"RPA-3.0 "),
        "RPA version"
    );
    let version = h[4];
    let number = |d: &[u8]| -> Result<u64> {
        let t = std::str::from_utf8(d)?;
        ensure!(t.bytes().all(|b| b.is_ascii_hexdigit()), "RPA hex header");
        Ok(u64::from_str_radix(t, 16)?)
    };
    let offset = number(h.get(8..24).context("RPA short header")?)?;
    let key = if version == b'3' {
        number(h.get(25..33).context("RPA3 short header")?)? as u32
    } else {
        0
    };
    let data = bytes(
        s,
        offset,
        length(s)?.checked_sub(offset).context("RPA table offset")?,
        cx.limits.max_index_size,
    )?;
    let table = crate::files::inflate(&data, cx.limits.max_index_size as usize)?;
    let mut c = Cursor::new(&table);
    let mut strings = Vec::new();
    let mut nums = Vec::new();
    let mut stopped = false;
    while c.left() > 0 {
        ensure!(
            strings.len() <= cx.limits.max_entries * 2 && nums.len() <= cx.limits.max_entries * 2,
            "RPA entry count"
        );
        match c.u8()? {
            b'U' => {
                let n = c.u8()? as usize;
                strings.push(c.take(n)?.to_vec());
            }
            b'X' => {
                let n = c.le32()? as usize;
                strings.push(c.take(n)?.to_vec());
            }
            b'K' => nums.push(c.u8()? as u32),
            b'M' => nums.push(c.le16()? as u32),
            b'J' => nums.push(c.le32()?),
            0x8a => {
                let n = c.u8()? as usize;
                ensure!(n <= 5, "RPA integer width");
                let data = c.take(n)?;
                ensure!(n < 5 || data[4] == 0, "RPA offset exceeds u32");
                let mut v = 0;
                for (i, b) in data.iter().take(4).enumerate() {
                    v |= (*b as u32) << (i * 8);
                }
                nums.push(v);
            }
            0x80 => {
                ensure!(c.u8()? <= 2, "RPA pickle protocol");
            }
            b'q' => {
                c.take(1)?;
            }
            b'r' => {
                c.take(4)?;
            }
            b'a' | b'u' | b'(' | b']' | b'}' | 0x85 | 0x86 | 0x87 => {}
            b'.' => {
                stopped = true;
                break;
            }
            op => anyhow::bail!(
                "unsupported RPA pickle opcode {op:#x}; executable opcodes are forbidden"
            ),
        }
    }
    ensure!(
        stopped && strings.len() % 2 == 0 && nums.len() == strings.len(),
        "unsupported RPA table structure"
    );
    let mut rows = Vec::new();
    for i in 0..strings.len() / 2 {
        let path = String::from_utf8(strings[i * 2].clone())?;
        let prefix = strings[i * 2 + 1].clone();
        let at = (nums[i * 2] ^ key) as u64;
        let stored = (nums[i * 2 + 1] ^ key) as u64;
        let size = stored
            .checked_add(prefix.len() as u64)
            .context("RPA logical prefix size")?;
        rows.push(Row::packed(
            path,
            at,
            stored,
            size,
            decode(move |d, _| {
                let mut o = prefix.clone();
                o.extend_from_slice(d);
                Ok(o)
            }),
        ));
    }
    Ok(rows)
}
