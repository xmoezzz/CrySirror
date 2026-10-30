// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only CatSystem2 archive profile from caller-owned PE resources.
//! Resource names and key policy independently implemented from the MIT
//! TriggersTools.CatSystem2 VCodes.cs, commit 47fdcb605c16a5d93a97cb2894684326f21885c8.
use anyhow::{ensure, Context, Result};
use crysirror_resource::DecodeContext;
use pelite::pe32::Pe as _;
use pelite::pe64::Pe as _;
use pelite::{resources::Name, PeFile};

pub(super) fn game_id(cx: &DecodeContext, path: &str) -> Result<Vec<u8>> {
    let reader = cx.open_dependency(path)?;
    let bytes = crysirror_resource::read_all(reader.as_ref(), 64 * 1024 * 1024)?;
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Result<Vec<u8>> {
    let pe = PeFile::from_bytes(bytes).context("CatSystem2 executable PE header")?;
    let resources = match pe {
        pelite::Wrap::T32(pe) => pe.resources().context("CatSystem2 PE resources")?,
        pelite::Wrap::T64(pe) => pe.resources().context("CatSystem2 PE resources")?,
    };
    let key = resources
        .find_resource(&[Name::Str("KEY_CODE"), Name::Str("KEY")])
        .context("CatSystem2 KEY_CODE/KEY resource")?;
    ensure!((4..=56).contains(&key.len()), "CatSystem2 KEY_CODE length");
    let key = key.iter().map(|b| b ^ 0xcd).collect::<Vec<_>>();
    let code = resources
        .find_resource(&[Name::Str("V_CODE2"), Name::Str("DATA")])
        .context("CatSystem2 V_CODE2/DATA resource")?;
    ensure!(
        !code.is_empty() && code.len() <= 4096 && code.len() % 8 == 0,
        "CatSystem2 V_CODE2 length"
    );
    let decoded = crysirror_resource::blowfish_le_decrypt(code, &key, code.len())?;
    let end = decoded
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(decoded.len());
    ensure!(
        end > 0 && decoded[end..].iter().all(|b| *b == 0),
        "CatSystem2 V_CODE2 termination"
    );
    Ok(decoded[..end].to_vec())
}
