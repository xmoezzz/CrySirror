// SPDX-License-Identifier: MPL-2.0
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct AudioHub;
pub use crate::bgm_bridge::BgmEngine;
pub use crate::koe_bridge::KoeEngine;
pub use crate::sfx_bridge::{PcmEngine, SeEngine};
