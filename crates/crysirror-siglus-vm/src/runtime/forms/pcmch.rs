// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
pub fn restore_persistent_channel(
    ctx: &mut crate::runtime::CommandContext,
    channel: usize,
    state: crate::runtime::globals::PcmChPersistentState,
) -> anyhow::Result<()> {
    crate::pcmch_bridge::restore(ctx, channel, state)
}
