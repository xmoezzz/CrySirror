// SPDX-License-Identifier: MPL-2.0
pub fn restore_persistent_channel(
    _: &mut crate::runtime::CommandContext,
    _: usize,
    state: crate::runtime::globals::PcmChPersistentState,
) -> anyhow::Result<()> {
    // Match the original restore predicate; persistent sources require an audio backend.
    if (state.loop_flag || state.ready_flag || state.delay_time > 0)
        && (!state.pcm_name.is_empty()
            || !state.bgm_name.is_empty()
            || state.koe_no >= 0
            || state.se_no >= 0)
    {
        anyhow::bail!("native persistent PCM restore unsupported")
    }
    Ok(())
}
