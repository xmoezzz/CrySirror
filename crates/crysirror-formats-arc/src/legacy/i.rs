// SPDX-License-Identifier: GPL-3.0-or-later
// Safe format/codec translation from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
mod s25;
mod warc;
pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "shiina-rio/s25",
        probe: Some(|s| magic(s, 0, b"S25\0")),
        open: s25::open,
    },
    Legacy {
        id: "shiina-rio/warc",
        probe: Some(|s| magic(s, 0, b"WARC ")),
        open: warc::open,
    },
];
