#[cfg(feature = "no_std")]
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use serde::{Deserialize, Serialize};

pub use crysirror_text::slot_color::SlotColor as ColorItem;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ColorManager {
    // 256 entries in total
    colors: Vec<ColorItem>,
    /// Derived volatile window color, refreshed from this VM before drawing.
    #[serde(skip)]
    pub(crate) volatile_tile: [u8; 4],
}

impl ColorManager {
    pub fn new() -> Self {
        let mut s = Self {
            colors: vec![ColorItem::new(); 256],
            volatile_tile: [0; 4],
        };

        s.colors[1] = ColorItem::black();
        s.colors[2] = ColorItem::white();
        s
    }

    pub fn get_entry_mut(&mut self, id: u8) -> &mut ColorItem {
        &mut self.colors[id as usize]
    }

    pub fn get_entry(&self, id: u8) -> &ColorItem {
        &self.colors[id as usize]
    }

    /// The colour a Tile primitive draws with. Tile id -1 bypasses the palette: the original
    /// engine reads R, G, B, A from volatile globals 5..8 each frame (the message window
    /// colour the scripts' config screens edit), keeping the low byte of each.
    pub fn tile_color(&self, tile: i16) -> ColorItem {
        if tile == -1 {
            let channel = |i| self.volatile_tile[(i - 5) as usize];
            return ColorItem {
                r: channel(5),
                g: channel(6),
                b: channel(7),
                a: channel(8),
            };
        }
        self.colors[tile as u8 as usize].clone()
    }
}

impl Default for ColorManager {
    fn default() -> Self {
        Self::new()
    }
}
