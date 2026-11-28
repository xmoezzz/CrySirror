// SPDX-License-Identifier: MPL-2.0
//! Definitions owned by the native scenario, before a save/load, log or selection
//! screen is opened. Original resource and font names are CP932 byte strings.
use crate::{
    message_area::Font,
    native_numbers::{integer, integer2011, unsigned2011},
};
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;

const GOTHIC: &[u8] = b"\x82\x6c\x82\x72 \x83\x53\x83\x56\x83\x62\x83\x4e";
const P_GOTHIC: &[u8] = b"\x82\x6c\x82\x72 \x82\x6f\x83\x53\x83\x56\x83\x62\x83\x4e";
const LOG_COLORS: [i32; 4] = [0xa0ffa0, 0xffffff, 0xe0e0e0, 0xffe8e0];

struct Fields<'a>(Vec<&'a [u8]>, fn(&[u8]) -> i32);
impl<'a> Fields<'a> {
    fn split(bytes: &'a [u8], count: usize, remainder: bool, number: fn(&[u8]) -> i32) -> Self {
        let fields = if remainder {
            bytes.splitn(count, |b| *b == b',').collect()
        } else {
            bytes.split(|b| *b == b',').take(count).collect()
        };
        Self(fields, number)
    }
    fn raw(&self, field: usize) -> &'a [u8] {
        self.0.get(field).copied().unwrap_or_default()
    }
    fn n(&self, field: usize) -> i32 {
        (self.1)(self.raw(field))
    }
    fn words<const N: usize>(&self, start: usize) -> [i32; N] {
        std::array::from_fn(|i| self.n(start + i))
    }
    fn image<const N: usize>(&self, start: usize) -> ImagePlacement<N> {
        ImagePlacement {
            name: self.raw(start).to_vec(),
            placement: self.words(start + 1),
        }
    }
    fn rect(&self, start: usize) -> [i32; 4] {
        let [x, y, w, h] = self.words(start);
        let (right, bottom) = (x.wrapping_add(w), y.wrapping_add(h));
        [x.min(right), y.min(bottom), x.max(right), y.max(bottom)]
    }
}

fn view_layers<const N: usize>(abi: u32, mut layers: [i32; N]) -> Result<[i32; N]> {
    if abi == 3 {
        for layer in &mut layers {
            *layer = layer2011(*layer)?;
        }
    }
    Ok(layers)
}

/// RVA 0xdbec0. The compact IDs use a lookup table. The high literal
/// aliases are intervals interspersed with single reserved layer aliases.
pub(super) fn layer2011(id: i32) -> Result<i32> {
    const COMPACT: [i32; 87] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 24, 25, 26, 27, 28, 29, 30, 31, 36, 37, 34, 35, 36,
        37, 38, 39, 40, 41, 50, 51, 44, 45, 46, 47, 48, 49, 50, 51, 92, 93, 94, 95, 96, 97, 98, 99,
        100, 101, 125, 131, 132, 133, 134, 136, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157,
        158, 159, 160, 169, 170, 171, 173, 174, 175, 176, 177, 178, 183, 184, 185, 186, 187, 188,
        189, 190, 191,
    ];
    Ok(match id {
        0..=86 => COMPACT[id as usize],
        289 | 369 => 91,
        290 | 358 => 66,
        291 | 699 => 123,
        292 | 327 => 23,
        293 | 329 => 35,
        294 | 328 => 24,
        295 | 339 => 49,
        296 | 338 => 38,
        297 | 349 => 63,
        298 | 348 => 52,
        299 | 649 => 103,
        359 => 77,
        368 => 80,
        300..=326 => id - 296,
        330..=337 => id - 294,
        340..=347 => id - 290,
        350..=357 => id - 286,
        360..=399 => id - 282,
        400..=499 => id - 364,
        500..=549 => id - 450,
        550..=599 => id - 486,
        600..=648 => id - 508,
        650..=698 => id - 546,
        700..=749 => id - 575,
        750..=799 => id - 624,
        800..=809 => id - 663,
        810..=819 => id - 662,
        820..=839 => id - 667,
        840..=849 => id - 671,
        850..=869 => id - 677,
        870..=889 => id - 685,
        890..=899 => id - 703,
        900..=999 => id - 712,
        1000.. => anyhow::bail!("unresolved ExHibit 2011 live layer alias {id}"),
        _ => id,
    })
}

/// Late command layer aliases at 0x1826c0/0x1827d0. Slots 50..86
/// and aliases above 699 differ from the 2011 resident.
pub(super) fn layer_late(id: i32) -> Result<i32> {
    const COMPACT_TAIL: [i32; 37] = [
        131, 138, 139, 140, 141, 143, 155, 156, 157, 158, 159, 160, 161, 162, 163,
        164, 165, 166, 167, 182, 183, 184, 186, 187, 188, 189, 190, 191, 206, 207,
        210, 211, 212, 213, 214, 215, 216,
    ];
    Ok(match id {
        50..=86 => COMPACT_TAIL[(id - 50) as usize],
        710 => 132,
        700..=749 => id - 569,
        750..=799 => id - 617,
        800..=809 => id - 656,
        810..=819 => id - 655,
        820..=839 => id - 660,
        840..=849 => id - 658,
        850..=859 => id - 664,
        860..=869 => id - 654,
        870..=889 => id - 660,
        890..=899 => id - 678,
        910 => 214,
        920 => 215,
        930 => 217,
        900..=949 => id - 687,
        950..=999 => id - 764,
        1000..=3999 => anyhow::bail!("unresolved ExHibit late live layer alias {id}"),
        4000..=4019 => id - 3876,
        4020..=4059 => id - 3844,
        4060.. => id - 3852,
        _ => layer2011(id)?,
    })
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(bound(serialize = "[i32; N]: Serialize"))]
struct ImagePlacement<const N: usize> {
    name: Vec<u8>,
    placement: [i32; N],
}

fn font_tokens<'a>(bytes: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
    bytes.split(|b| *b == b',').filter(|s| !s.is_empty())
}

fn read_font<'a>(
    tokens: &mut impl Iterator<Item = &'a [u8]>,
    fallback: &[u8],
    number: fn(&[u8]) -> i32,
) -> Font {
    // sub_1009B820 leaves the receiver intact when the cursor is exhausted.
    // The scratch FCString is reused for the eight numeric conversions.
    let family = tokens.next().unwrap_or(fallback).to_vec();
    let mut last = 0;
    let words = std::array::from_fn(|_| {
        if let Some(token) = tokens.next() {
            last = number(token);
        }
        last
    });
    Font { family, words }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct SlotView {
    pages: i32,
    items_per_page: i32,
    layers: [i32; 5],
    background: ImagePlacement<2>,
    page: ImagePlacement<2>,
    memo: ImagePlacement<2>,
    mark: ImagePlacement<2>,
    item_bounds: [i32; 4],
    memo_length: i32,
    memo_bounds: [i32; 4],
    memo_step: [i32; 2],
    item_button: ImagePlacement<4>,
    page_button: ImagePlacement<4>,
    memo_button: ImagePlacement<4>,
    exit_button: ImagePlacement<2>,
    font: Font,
    overrides: BTreeMap<i32, [i32; 2]>,
}
impl SlotView {
    fn new(bytes: &[u8], extended: bool, previous: Option<&Self>, abi: u32) -> Result<Self> {
        let f = Fields::split(
            bytes,
            if extended { 49 } else { 48 },
            extended,
            if abi == 3 { integer2011 } else { integer },
        );
        let font = if extended {
            read_font(
                &mut font_tokens(f.raw(48)),
                previous.map_or(P_GOTHIC, |v| v.font.family.as_slice()),
                f.1,
            )
        } else {
            Font {
                family: GOTHIC.to_vec(),
                words: [256, 400, 20, 16, 128, 1, 0xffffff, 0],
            }
        };
        Ok(Self {
            pages: f.n(0),
            items_per_page: f.n(1),
            layers: view_layers(abi, f.words(2))?,
            background: f.image(7),
            page: f.image(10),
            memo: f.image(13),
            mark: f.image(16),
            item_bounds: f.rect(19),
            memo_length: f.n(23),
            memo_bounds: f.rect(24),
            memo_step: f.words(28),
            item_button: f.image(30),
            page_button: f.image(35),
            memo_button: f.image(40),
            exit_button: f.image(45),
            font,
            overrides: BTreeMap::new(),
        })
    }
    pub fn page_position(&self, index: i32) -> Option<[i32; 2]> {
        if !(0..self.pages).contains(&index) {
            return None;
        }
        let [x, y, dx, dy] = self.page_button.placement;
        Some(self.overrides.get(&index).copied().unwrap_or([
            x.wrapping_add(index.wrapping_mul(dx)),
            y.wrapping_add(index.wrapping_mul(dy)),
        ]))
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct LogView {
    background: ImagePlacement<2>,
    layers: [i32; 2],
    creation_parameter: i32,
    bounds: [i32; 4],
    selector: ImagePlacement<2>,
    colors: [i32; 4],
    fonts: [Font; 2],
}
impl LogView {
    fn new(bytes: &[u8], extended: bool, abi: u32) -> Result<Self> {
        let f = Fields::split(
            bytes,
            if extended { 17 } else { 12 },
            extended,
            if abi == 3 { integer2011 } else { integer },
        );
        let mut colors = LOG_COLORS;
        let fonts = if extended {
            for (i, c) in colors.iter_mut().enumerate() {
                let n = f.n(12 + i);
                if n >= 0 {
                    *c = n;
                }
            }
            let mut t = font_tokens(f.raw(16));
            [
                read_font(&mut t, P_GOTHIC, f.1),
                read_font(&mut t, P_GOTHIC, f.1),
            ]
        } else {
            [
                Font {
                    family: GOTHIC.to_vec(),
                    words: [256, 400, 20, 16, 128, 1, 0xf0f0f0, 0],
                },
                Font {
                    family: P_GOTHIC.to_vec(),
                    words: [0, 400, 20, 16, 128, 2, 0xf0f0f0, 0],
                },
            ]
        };
        Ok(Self {
            background: f.image(0),
            layers: view_layers(abi, f.words(3))?,
            creation_parameter: if abi == 3 { f.n(11) } else { 20 },
            bounds: f.rect(5),
            selector: f.image(9),
            colors,
            fonts,
        })
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct SelectView {
    background: ImagePlacement<2>,
    item_image: Vec<u8>,
    bounds: [i32; 4],
    step: [i32; 2],
    font: Font,
}
impl SelectView {
    fn new(bytes: &[u8]) -> Self {
        let f = Fields::split(bytes, 11, true, integer);
        Self {
            background: f.image(0),
            item_image: f.raw(3).to_vec(),
            bounds: f.rect(4),
            step: f.words(8),
            font: read_font(&mut font_tokens(f.raw(10)), P_GOTHIC, integer),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub(super) struct SelectView2011 {
    selected: Option<SelectView>,
    extra_images: [Option<Vec<u8>>; 8],
    uses_extra_images: bool,
    properties: [i32; 3],
}

impl SelectView2011 {
    pub fn install(&mut self, mode: u8, bytes: &[u8]) {
        let count = match mode {
            0 => 11,
            1 => 19,
            _ => 22,
        };
        let f = Fields::split(bytes, count, true, integer2011);
        let start = if mode == 0 { 4 } else { 12 };
        let num = |i| integer2011(f.raw(i));
        let x = num(start);
        let y = num(start + 1);
        let right = x.wrapping_add(num(start + 2));
        let bottom = y.wrapping_add(num(start + 3));
        let properties = if mode >= 2 {
            [unsigned2011(f.raw(18)) as i32, num(19), num(20)]
        } else {
            [0; 3]
        };
        let selected = SelectView {
            background: ImagePlacement {
                name: f.raw(0).to_vec(),
                placement: [num(1), num(2)],
            },
            item_image: f.raw(3).to_vec(),
            bounds: [x.min(right), y.min(bottom), x.max(right), y.max(bottom)],
            step: [num(start + 4), num(start + 5)],
            font: read_font(&mut font_tokens(f.raw(count - 1)), P_GOTHIC, integer2011),
        };
        if mode > 0 {
            for (i, dst) in self.extra_images.iter_mut().enumerate() {
                let name = f.raw(i + 4);
                if name != b"-1" {
                    *dst = Some(name.to_vec());
                    self.uses_extra_images = true;
                }
            }
        }
        self.selected = Some(selected);
        self.properties = properties;
    }
}

#[derive(Debug, Default, Serialize)]
pub(super) struct Views {
    pub save: Option<SlotView>,
    pub load: Option<SlotView>,
    log: Option<LogView>,
    selection: Option<SelectView>,
}
#[derive(Debug, PartialEq)]
pub(super) enum Definition {
    Save,
    Load,
    Log,
    Selection,
}
impl Views {
    pub fn install(
        &mut self,
        kind: Definition,
        extended: bool,
        bytes: &[u8],
        abi: u32,
    ) -> Result<()> {
        match kind {
            Definition::Save => {
                self.save = Some(SlotView::new(bytes, extended, self.save.as_ref(), abi)?)
            }
            Definition::Load => {
                self.load = Some(SlotView::new(bytes, extended, self.load.as_ref(), abi)?)
            }
            Definition::Log => self.log = Some(LogView::new(bytes, extended, abi)?),
            Definition::Selection => self.selection = Some(SelectView::new(bytes)),
        }
        Ok(())
    }
    pub fn place_load_page(&mut self, [page, x, y]: [i32; 3]) {
        if let Some(view) = self.load.as_mut() {
            if view.page_position(page).is_some() {
                view.overrides.insert(page, [x, y]);
            }
        }
    }
}
