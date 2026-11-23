// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! KiriKiri executable-embedded options (`XOPT_EMBED_AREA_`): the release
//! configuration a shipped krkr2 executable carries, e.g. `debugwin="no"`,
//! which `System.getArgument("-debugwin")` reports. The area is the marker,
//! a 32-bit capacity, then `name=value` lines up to a NUL; values are TJS
//! string literals. What follows the NUL (the configurable option names) is
//! not an option and is ignored. KiriKiri Z stores its defaults in the PE
//! TEXT/IDR_OPTION resource instead. Only data is read.
use anyhow::{Context, Result, ensure};
use crysirror_resource::pe::{PeData, PeResourceName};

const MARKER: &[u8] = b"XOPT_EMBED_AREA_";
/// The area is fixed-size in the executable; anything longer is not an area.
const MAX_AREA: usize = 64 * 1024;
const MAX_OPTIONS: usize = 256;

/// `(name, value)` pairs, names without the leading `-`.
pub fn parse(exe: &[u8]) -> Result<Vec<(String, String)>> {
    let Some(at) = exe.windows(MARKER.len()).position(|w| w == MARKER) else {
        // krkrz TVPGetEmbeddedOptions calls FindResource(TEXT, IDR_OPTION).
        // Older executables may have no PE resource directory at all.
        if let Some(options) = PeData::parse(exe).ok()
            .and_then(|pe| pe.raw_resources(&PeResourceName::Name("TEXT".into()), &|| Ok(())).ok())
            .and_then(|resources| resources.into_iter().find(|r| r.name == PeResourceName::Id(139)))
        {
            ensure!(options.bytes.len() <= MAX_AREA, "embedded option resource limit");
            let end = options.bytes.iter().position(|b| *b == 0).unwrap_or(options.bytes.len());
            return parse_config(&options.bytes[..end]);
        }
        return Ok(Vec::new());
    };
    let body = exe
        .get(at + MARKER.len() + 4..)
        .context("truncated embedded option area")?;
    let text = &body[..body.len().min(MAX_AREA)];
    let end = text.iter().position(|b| *b == 0).context("unterminated embedded option area")?;
    let text = std::str::from_utf8(&text[..end]).context("embedded options are not text")?;
    let mut out = Vec::new();
    for line in text.split(['\n', '\r']).map(str::trim).filter(|l| !l.is_empty()) {
        ensure!(out.len() < MAX_OPTIONS, "embedded option limit");
        let (name, value) = line.split_once('=').unwrap_or((line, "yes"));
        out.push((name.trim().trim_start_matches('-').to_owned(), unquote(value.trim())?));
    }
    Ok(out)
}

/// `System.exeBits` of the original process: krkrz reports the pointer width of
/// the running executable, so it follows the PE machine and never the host. Yuzusoft
/// and other scripts branch on it to select plugins and the motion title path.
pub fn pe_bits(exe: &[u8]) -> Option<i64> {
    let lfanew = u32::from_le_bytes(exe.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if exe.get(..2)? != b"MZ" || exe.get(lfanew..lfanew.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    match u16::from_le_bytes(exe.get(lfanew + 4..lfanew + 6)?.try_into().ok()?) {
        0x014c => Some(32),
        0x8664 | 0xaa64 => Some(64),
        _ => None,
    }
}

/// Lines of a `<exe>.cf` / `<datapath>/<exe>.cfu` configuration file
/// (krkrz `TVPGetConfigFileOptions` + `PushConfigFileOptions`): each line
/// not starting with `;` is one `name=value` option, parsed like a command
/// line option (`TVPParseCommandLineOne`); a bare name means `yes`.
/// `(name, value)` pairs, names without the leading `-`, in file order.
pub fn parse_config(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let text = crate::storage::decode_text(bytes)?;
    let mut out = Vec::new();
    for line in text.split(['\r', '\n']) {
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        ensure!(out.len() < MAX_OPTIONS, "configuration option limit");
        let (name, value) = line.split_once('=').unwrap_or((line, "yes"));
        let name = name.trim_start_matches('-');
        // TVPGetCommandLine answers the first stocked match.
        if !out.iter().any(|(n, _): &(String, String)| n == name) {
            out.push((name.to_owned(), unquote(value)?));
        }
    }
    Ok(out)
}

/// A TJS string literal (`"..."` or `'...'` with backslash escapes), or a
/// bare value as written.
fn unquote(v: &str) -> Result<String> {
    let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') else {
        return Ok(v.to_owned());
    };
    ensure!(v.len() >= 2 && v.ends_with(q), "unterminated embedded option string");
    let inner = &v[1..v.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next().context("dangling escape in embedded option")? {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'x' | 'X' => {
                let mut code = 0u32;
                let mut digits = 0;
                while digits < 4 && let Some(d) = chars.peek().and_then(|c| c.to_digit(16)) {
                    code = code * 16 + d;
                    chars.next();
                    digits += 1;
                }
                ensure!(digits > 0, "empty hex escape in embedded option");
                out.push(char::from_u32(code).context("invalid escaped character")?);
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quoted_and_escaped_values_before_the_name_list() {
        let mut exe = b"MZ....".to_vec();
        exe.extend(MARKER);
        exe.extend(0x0fecu32.to_le_bytes());
        exe.extend(b"debugwin=\"\\x6E\\x6F\"\nhkupdaterect=\"\"\n\0forcelog,debugwin\0\0\0");
        assert_eq!(
            parse(&exe).unwrap(),
            [("debugwin".into(), "no".into()), ("hkupdaterect".into(), String::new())]
        );
    }

    #[test]
    fn exe_bits_follow_the_pe_machine() {
        let pe = |machine: u16| {
            let mut exe = vec![0; 0x80];
            exe[..2].copy_from_slice(b"MZ");
            exe[0x3c] = 0x40;
            exe[0x40..0x44].copy_from_slice(b"PE\0\0");
            exe[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
            exe
        };
        assert_eq!(pe_bits(&pe(0x014c)), Some(32));
        assert_eq!(pe_bits(&pe(0x8664)), Some(64));
        assert_eq!(pe_bits(&pe(0x01c0)), None);
        assert_eq!(pe_bits(b"MZ truncated"), None);
    }

    #[test]
    fn configuration_files_skip_comments_and_unquote_values() {
        let options = parse_config(b"; comment\r\nfullscreenmode=\"\\x31\"\r\nforcelog\r\nvomstyle=layer\n").unwrap();
        assert_eq!(
            options,
            [
                ("fullscreenmode".into(), "1".into()),
                ("forcelog".into(), "yes".into()),
                ("vomstyle".into(), "layer".into())
            ]
        );
    }

    #[test]
    fn kirikiri_z_uses_named_text_resource_139_as_defaults() {
        // One PE32 resource section with the tree TEXT / 139 / Japanese.
        let mut exe = vec![0; 1024];
        exe[..2].copy_from_slice(b"MZ");
        exe[128..132].copy_from_slice(b"PE\0\0");
        for (at, value) in [(132, 0x14cu16), (134, 1), (148, 224), (152, 0x10b),
            (524, 1), (550, 1), (574, 1), (608, 4)] {
            exe[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        let options = b"debugwin=\"no\"\rfixlangs=\"cn\"\0ignored=yes";
        for (at, value) in [(60, 128u32), (184, 4096), (188, 512), (208, 8192),
            (212, 512), (244, 16), (264, 4096), (268, 512),
            (384, 512), (388, 4096), (392, 512), (396, 512),
            (528, 0x8000_0060), (532, 0x8000_0018), (552, 139), (556, 0x8000_0030),
            (576, 1041), (580, 72), (584, 4096 + 128), (588, options.len() as u32)] {
            exe[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        exe[376..381].copy_from_slice(b".rsrc");
        exe[610..618].copy_from_slice(b"T\0E\0X\0T\0");
        exe[640..640 + options.len()].copy_from_slice(options);
        assert_eq!(parse(&exe).unwrap(), [("debugwin".into(), "no".into()), ("fixlangs".into(), "cn".into())]);
        exe[552..556].copy_from_slice(&140u32.to_le_bytes());
        assert!(parse(&exe).unwrap().is_empty());
    }

    #[test]
    fn executables_without_an_area_have_no_options() {
        assert!(parse(b"MZ no options here").unwrap().is_empty());
    }
}
