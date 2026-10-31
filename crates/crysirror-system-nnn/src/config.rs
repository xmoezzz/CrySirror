// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native byte-name tables and explicit variable-layout policy. Resource XOR
//! remains in the shared Resource transform; original encoded names stay bytes.
use crate::expression::{VariableProfile, Variables};
use anyhow::{ensure, Context, Result};
#[derive(Clone, Debug)]
pub struct Names {
    pub tokens: Vec<Vec<u8>>,
}
impl Names {
    pub fn parse(bytes: &[u8], ciphered: bool) -> Result<Self> {
        ensure!(
            bytes.len() <= 1024 * 1024,
            "System-NNN name table byte budget"
        );
        let mut plain = bytes.to_vec();
        if ciphered {
            crysirror_resource::transforms::xor_cycle_in_place(&mut plain, &[255], 0);
        }
        ensure!(!plain.contains(&0), "System-NNN embedded NUL name table");
        let mut at = 0;
        let mut tokens = Vec::new();
        while at < plain.len() {
            if plain[at] == b',' {
                at += 1;
            }
            while plain
                .get(at)
                .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            {
                at += 1;
            }
            if at == plain.len() {
                break;
            }
            let start = at;
            while plain
                .get(at)
                .is_some_and(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b','))
            {
                at += 1;
            }
            ensure!(at - start <= 128, "System-NNN name token length");
            ensure!(tokens.len() < 32768, "System-NNN name token count");
            tokens.push(plain[start..at].to_vec());
        }
        Ok(Self { tokens })
    }
    pub fn index(&self, name: &[u8]) -> Option<u16> {
        self.tokens
            .iter()
            .position(|s| s == name)
            .and_then(|i| u16::try_from(i).ok())
    }
    pub fn parameter(&self, name: &[u8]) -> Option<&[u8]> {
        self.tokens
            .chunks_exact(2)
            .find(|p| p[0].eq_ignore_ascii_case(name))
            .map(|p| p[1].as_slice())
    }
    pub fn integer(&self, name: &[u8]) -> Result<Option<i32>> {
        self.parameter(name).map(integer).transpose()
    }
}
pub fn integer(bytes: &[u8]) -> Result<i32> {
    let mut at = 0;
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    let negative = bytes.get(at) == Some(&b'-');
    if matches!(bytes.get(at), Some(b'-' | b'+')) {
        at += 1;
    }
    let mut value = 0i64;
    while let Some(d) = bytes.get(at).filter(|d| d.is_ascii_digit()) {
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(i64::from(*d - b'0')))
            .context("System-NNN integer overflow")?;
        at += 1;
    }
    i32::try_from(if negative { -value } else { value }).context("System-NNN integer range")
}
/// Exact original fresh-session bank and native calculated bindings. Saved
/// persistent values must be supplied separately rather than claimed restored.
pub fn fresh_variables(profile: VariableProfile, names: &Names) -> Result<Variables> {
    let limit = match profile {
        VariableProfile::Legacy1000 => 1001,
        VariableProfile::Expanded2200 => 2201,
    };
    ensure!(
        names.tokens.len() <= limit,
        "System-NNN variable-name profile mismatch"
    );
    let mut vars = Variables::fresh(profile);
    for name in [
        b"rnd".as_slice(),
        b"week",
        b"holiday",
        b"systemyear",
        b"systemmonth",
        b"systemday",
        b"systemhour",
        b"systemminute",
        b"systemsecond",
        b"systemweek",
        b"systemmillisecond",
        b"mode",
        b"direct3d",
        b"pixelshader",
        b"vertexshader",
        b"mousex",
        b"mousey",
        b"trig1",
        b"trig2",
        b"trig3",
        b"click1",
        b"click2",
        b"click3",
        b"wheel",
        b"cgpercent",
        b"scenepercent",
        b"expmode1",
        b"expmode2",
        b"expmode3",
        b"expmode4",
        b"expmode5",
        b"expcheck1",
        b"expcheck2",
        b"expcheck3",
        b"expcheck4",
        b"expcheck5",
    ] {
        if let Some(id) = names.index(name) {
            vars.mark_calculated(id, name.to_vec());
        }
    }
    Ok(vars)
}
pub fn configured_variables(
    profile: VariableProfile,
    names: &Names,
    setup: &Names,
) -> Result<Variables> {
    let mut vars = fresh_variables(profile, names)?;
    // Original reviewed legacy constructor 0x542180 binds date/year as ordinary
    // storage and writes these defaults. GetCalcuVarData 0x5335e0 falls through
    // to the bank for names without a real calculated binding, including older
    // time/year/month aliases; reserving every named id<100 changes native data.
    if let Some(id) = names.index(b"date") {
        vars.write_stored(id, 101);
    }
    if profile == VariableProfile::Legacy1000 || setup.integer(b"notUseYearVar")?.unwrap_or(0) == 0
    {
        if let Some(id) = names.index(b"year") {
            vars.write_stored(id, 2000);
        }
    }
    let layers = if profile==VariableProfile::Legacy1000{16}else{setup.integer(b"layerExpand")?.unwrap_or(16)};
    ensure!(
        (1..=32).contains(&layers),
        "unsupported System-NNN native layer count {layers}"
    );
    for layer in 0..layers {
        let key = format!("layerOffVar{layer}");
        if let Some(name) = setup.parameter(key.as_bytes()) {
            if let Some(id) = names.index(name) {
                vars.mark_write_effect(id, key.into_bytes());
            }
        }
    }
    Ok(vars)
}
/// Original InitNewGame clears only its nonpersistent ranges, then optionally
/// applies the configured native table. It never invents a script flag/PC.
pub fn initialize_new_game(
    vars: &mut Variables,
    profile: VariableProfile,
    initial: Option<&Names>,
) -> Result<()> {
    let tail = match profile {
        VariableProfile::Legacy1000 => 300..1000,
        VariableProfile::Expanded2200 => 1200..2200,
    };
    let mut candidate = vars.clone();
    for id in (100..200).chain(tail) {
        let value = if let Some(table) = initial {
            integer(
                table
                    .tokens
                    .get(id)
                    .context("System-NNN initial variable table bounds")?,
            )?
        } else {
            0
        };
        candidate.write_stored(id as u16, value);
    }
    *vars = candidate;
    Ok(())
}
