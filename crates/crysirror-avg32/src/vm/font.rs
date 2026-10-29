//! `FN.DAT` (NVL_SYSTEM) bitmap font shipped by the game. CrySirror: the
//! upstream TrueType rasteriser is removed; outline text is retained glyph
//! layers rendered by the engine (see `text`).

/// `FN.DAT`: the `NVL_SYSTEM` bitmap font.
#[derive(Debug, Clone)]
pub struct NovelFont {
    data: Vec<u8>,
}

const NOVEL_FONT_BYTES: usize = 12 * 24;
const NOVEL_FONT_SIZE: usize = 2_544_768;

impl NovelFont {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Shift-JIS to JIS X 0208 row/cell code.
    fn sjis_to_jis(sjis: u16) -> Option<u16> {
        let mut low = i32::from(sjis & 0xff);
        let mut high = (i32::from(sjis) - 0x8100) & 0xff00;
        let mut base = 0x2100;
        if low > 0x7f {
            if low >= 0x9f {
                if low > 0xfc {
                    return None;
                }
                base += 0x100;
                low -= 0x5f;
            } else {
                low -= 1;
            }
        } else if low == 0x7f || low < 0x40 {
            return None;
        }
        low -= 0x1f;
        if high >= 0x1f00 {
            if !(0x5f00..=0x6e00).contains(&high) {
                return None;
            }
            high -= 0x5f00;
            base += 0x5f00 - 0x2100;
        }
        Some(((high << 1) + base + low) as u16)
    }

    /// The 12x24-byte (24x24 nibble) glyph for a Shift-JIS code.
    pub fn glyph(&self, sjis: u16) -> Option<&[u8]> {
        let jis = i32::from(Self::sjis_to_jis(sjis)?) - 0x2121;
        let (high, low) = (jis >> 8, jis & 0xff);
        let mut position = ((high * 0x5e + low) * NOVEL_FONT_BYTES as i32).max(0) as usize;
        if position > NOVEL_FONT_SIZE - NOVEL_FONT_BYTES
            || position + NOVEL_FONT_BYTES > self.data.len()
        {
            position = 0;
        }
        self.data.get(position..position + NOVEL_FONT_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_sjis_to_jis_like_the_reference() {
        // "あ" is SJIS 0x82A0, JIS 0x2422.
        assert_eq!(NovelFont::sjis_to_jis(0x82a0), Some(0x2422));
        // "亜" is SJIS 0x889F, JIS 0x3021.
        assert_eq!(NovelFont::sjis_to_jis(0x889f), Some(0x3021));
    }
}
