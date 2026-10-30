// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native CUR resource boundary. Pixels/masks/animation definitions belong to
//! CrySirror's shared image decoder and UI cursor service, not a VM painter.
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub image: std::sync::Arc<crysirror_image::cursor::CursorImage>,
}
impl Cursor {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            image: std::sync::Arc::new(crysirror_image::cursor::decode_cur16m(bytes)?),
        })
    }
}
