// SPDX-License-Identifier: MPL-2.0
//! Original numeric/string substitution policy, never a host pointer or layout.
use crate::kcs::Machine;
use anyhow::{ensure, Context, Result};
/// Native sub6A48F0 integer syntax: no whitespace, optional sign and hex prefix,
/// wrapping 32-bit accumulation, and decimal fractions explicitly become zero.
pub fn integer(text: &str) -> u32 {
    let mut bytes = text.as_bytes();
    let negative = bytes.first() == Some(&b'-');
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        bytes = &bytes[1..];
    }
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return 0;
    }
    let hex = bytes.starts_with(b"0x") || bytes.starts_with(b"0X");
    if hex {
        bytes = &bytes[2..];
    }
    let radix = if hex { 16 } else { 10 };
    let mut value = 0u32;
    let mut count = 0;
    for b in bytes {
        let digit = match b {
            b'0'..=b'9' => u32::from(b - b'0'),
            b'a'..=b'f' if hex => u32::from(b - b'a' + 10),
            b'A'..=b'F' if hex => u32::from(b - b'A' + 10),
            _ => break,
        };
        value = value.wrapping_mul(radix).wrapping_add(digit);
        count += 1;
    }
    if !hex && bytes.get(count) == Some(&b'.') {
        return 0;
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}
pub fn format_bytes(machine: &Machine, address: u32) -> Result<Vec<u8>> {
    format_inner(machine, address, 0)
}
pub fn format(machine: &Machine, address: u32) -> Result<String> {
    let bytes = format_bytes(machine, address)?;
    String::from_utf8(bytes).context("unsupported non-UTF8 CatSystem2 native path")
}
/// Original native19/20 signed-char difference, or the reviewed module's
/// native21/22 CRT C-locale unsigned ASCII-folded difference. The raw forms
/// reverse their two arguments; callers retain that ABI order.
pub fn compare(left: &[u8], right: &[u8], insensitive: bool) -> u32 {
    let mut index = 0;
    loop {
        let mut a = left.get(index).copied().unwrap_or(0);
        let mut b = right.get(index).copied().unwrap_or(0);
        if insensitive {
            a = a.to_ascii_lowercase();
            b = b.to_ascii_lowercase();
        }
        if a != b || a == 0 {
            return if insensitive {
                i32::from(a) - i32::from(b)
            } else {
                i32::from(a as i8) - i32::from(b as i8)
            } as u32;
        }
        index += 1;
    }
}
/// Read native42's NUL-list at numeric VM addresses. A negative signed index
/// selects its first element, and crossing the first empty entry stops there.
/// This is VM memory layout policy; delimiter splitting belongs to shared Text.
pub fn nul_list_item(machine: &Machine, source: u32, index: u32) -> Result<Vec<u8>> {
    let mut at = source;
    let mut consumed = 0usize;
    let count = (index as i32).max(0) as usize;
    ensure!(count <= 65536, "CatSystem2 NUL-list index budget");
    for _ in 0..count {
        let bytes = machine.read_cstring(at, 65534usize.saturating_sub(consumed))?;
        if bytes.is_empty() {
            return Ok(bytes);
        }
        consumed = consumed
            .checked_add(bytes.len() + 1)
            .context("CatSystem2 NUL-list byte overflow")?;
        ensure!(consumed <= 65534, "CatSystem2 NUL-list byte budget");
        at = source
            .checked_add(consumed as u32)
            .context("CatSystem2 NUL-list address overflow")?;
    }
    machine.read_cstring(at, 65534usize.saturating_sub(consumed))
}
fn format_inner(machine: &Machine, address: u32, depth: u8) -> Result<Vec<u8>> {
    ensure!(depth < 8, "CatSystem2 native format recursion budget");
    let input = machine.read_cstring(address, 4096)?;
    let mut output = Vec::new();
    let mut i = 0;
    while i < input.len() {
        if input[i] != b'%' {
            ensure!(
                output.len() < 4096,
                "CatSystem2 native format output budget"
            );
            output.push(input[i]);
            i += 1;
            continue;
        }
        if input.get(i + 1) == Some(&b'%') {
            ensure!(
                output.len() < 4096,
                "CatSystem2 native format output budget"
            );
            output.push(b'%');
            i += 2;
            continue;
        }
        let begin = i;
        i += 1;
        let zero = input.get(i) == Some(&b'0');
        let mut width = 0usize;
        while input.get(i).is_some_and(u8::is_ascii_digit) {
            width = width
                .checked_mul(10)
                .and_then(|n| n.checked_add((input[i] - b'0') as usize))
                .context("native format width overflow")?;
            ensure!(width <= 64, "native format width budget");
            i += 1;
        }
        let kind = *input.get(i).context("incomplete native format")?;
        i += 1;
        ensure!(
            input.get(i) == Some(&b'['),
            "unsupported native format at byte {begin}"
        );
        i += 1;
        let region = *input.get(i).context("native format region")?;
        i += 1;
        let mut offset = 0u32;
        let digits = i;
        while input.get(i).is_some_and(u8::is_ascii_digit) {
            offset = offset
                .checked_mul(10)
                .and_then(|n| n.checked_add(u32::from(input[i] - b'0')))
                .context("native format offset overflow")?;
            i += 1;
        }
        ensure!(
            i > digits && input.get(i) == Some(&b']'),
            "native format address syntax"
        );
        i += 1;
        let at = match region {
            b'G' => offset,
            b'L' => machine
                .frame_base
                .checked_add(offset)
                .context("native local format overflow")?,
            b'H' => offset | 0x80000000,
            _ => anyhow::bail!("unsupported native format region"),
        };
        let value = match kind {
            b'd' | b'i' => format!("{}", machine.read_word(at)? as i32).into_bytes(),
            b'u' => format!("{}", machine.read_word(at)?).into_bytes(),
            b'x' => format!("{:x}", machine.read_word(at)?).into_bytes(),
            b'X' => format!("{:X}", machine.read_word(at)?).into_bytes(),
            b's' => format_inner(machine, at, depth + 1)?,
            _ => anyhow::bail!("unsupported native format conversion {}", kind as char),
        };
        let padding = width.saturating_sub(value.len());
        if zero && value.first() == Some(&b'-') {
            output.push(b'-');
            output.extend(std::iter::repeat_n(b'0', padding));
            output.extend_from_slice(&value[1..]);
        } else {
            output.extend(std::iter::repeat_n(if zero { b'0' } else { b' ' }, padding));
            output.extend(value);
        }
        ensure!(
            output.len() <= 4096,
            "CatSystem2 native format output budget"
        );
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    #[test]
    fn string_variables_keep_empty_presence_raw_bytes_and_replacement_dirty_state() {
        let mut vars=super::StringVariables::default();
        assert_eq!(vars.get(800),None);assert!(!vars.dirty);
        vars.assign(800,Vec::new()).unwrap();assert_eq!(vars.get(800),Some(b"".as_slice()));assert!(vars.dirty);
        vars.dirty=false;vars.assign(800,vec![0x81,0x5c]).unwrap();assert_eq!(vars.get(800),Some([0x81,0x5c].as_slice()));assert!(vars.dirty);
        assert!(vars.assign(800,vec![1,0,2]).is_err());assert_eq!(vars.get(800),Some([0x81,0x5c].as_slice()));
    }

    #[test]
    fn native_compare_returns_the_complete_original_difference() {
        assert_eq!(super::compare(b"A", b"z", false), (-57i32) as u32);
        assert_eq!(super::compare(b"KCS", b"kcs", true), 0);
        assert_eq!(super::compare(&[0x80], b"", false), (-128i32) as u32);
        assert_eq!(super::compare(&[0x80], b"", true), 128);
        assert_eq!(super::compare(b"ab", b"abc", true), (-99i32) as u32);
    }
    #[test]
    fn nul_list_addresses_preserve_negative_and_empty_terminator_policy() {
        let p = crate::kcs::Program {
            code: vec![0x3b],
            globals_bytes: 256,
            stack_bytes: 256,
            strings: vec![],
        };
        let mut m = crate::kcs::Machine::new(&p, 256).unwrap();
        m.write_bytes(0x80000000, b"first\0second\0\0").unwrap();
        assert_eq!(
            super::nul_list_item(&m, 0x80000000, u32::MAX).unwrap(),
            b"first"
        );
        assert_eq!(super::nul_list_item(&m, 0x80000000, 1).unwrap(), b"second");
        assert_eq!(super::nul_list_item(&m, 0x80000000, 2).unwrap(), b"");
        assert_eq!(super::nul_list_item(&m, 0x80000000, 25).unwrap(), b"");
        assert!(super::nul_list_item(&m, 0x80000000, 65537).is_err());
    }
    #[test]
    fn format_uses_owned_global_local_heap_addresses_and_checks_recursion() {
        let p = crate::kcs::Program {
            code: vec![0x3b],
            globals_bytes: 256,
            stack_bytes: 256,
            strings: vec![],
        };
        let mut m = crate::kcs::Machine::new(&p, 256).unwrap();
        m.write_bytes(0, b"plane/%02d[L12]/%X[H4]/%s[G128]/%%\0")
            .unwrap();
        m.write_bytes(m.frame_base + 12, &3u32.to_le_bytes())
            .unwrap();
        m.write_bytes(0x80000004, &0xabu32.to_le_bytes()).unwrap();
        m.write_bytes(128, b"name\0").unwrap();
        assert_eq!(super::format(&m, 0).unwrap(), "plane/03/AB/name/%");
        m.write_bytes(0, b"%s[G0]\0").unwrap();
        assert!(super::format(&m, 0)
            .unwrap_err()
            .to_string()
            .contains("recursion"));
    }
    #[test]
    fn native_integer_syntax_is_not_rust_parse_or_atoi() {
        for (source, want) in [
            ("12tail", 12),
            (" 12", 0),
            ("12.0", 0),
            ("-0x10", 0xfffffff0),
            ("0XfG", 15),
            ("4294967296", 0),
            ("+2", 2),
        ] {
            assert_eq!(super::integer(source), want, "{source}");
        }
    }
}

///73FE70/740450: original per-KCS numeric string variable bank, not a text
/// layout/formatting engine. Empty assigned strings remain present values.
#[derive(Default)]
pub(crate) struct StringVariables {
    values: std::collections::BTreeMap<u32,Vec<u8>>,
    pub dirty:bool,
}
impl StringVariables {
    pub fn assign(&mut self,id:u32,bytes:Vec<u8>)->Result<()> {
        ensure!(bytes.len()<=65534&&!bytes.contains(&0),"CatSystem2 string variable extent");
        ensure!(self.values.contains_key(&id)||self.values.len()<65536,"CatSystem2 string variable count");
        let resident=self.values.values().map(Vec::len).sum::<usize>()-self.values.get(&id).map_or(0,Vec::len)+bytes.len();
        ensure!(resident<=16*1024*1024,"CatSystem2 string variable byte budget");
        self.values.insert(id,bytes);self.dirty=true;Ok(())
    }
    pub fn get(&self,id:u32)->Option<&[u8]> {self.values.get(&id).map(Vec::as_slice)}
    pub fn diagnostics(&self)->serde_json::Value {serde_json::json!({"count":self.values.len(),"bytes":self.values.values().map(Vec::len).sum::<usize>(),"dirty":self.dirty})}
}
