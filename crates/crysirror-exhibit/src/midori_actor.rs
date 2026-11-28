// SPDX-License-Identifier: MPL-2.0
//! Native Midori offstage lane owners and ordinary immediate portrait placement.
use crate::{
    branch::number, character::Catalog, midori_actor_image::FacePlan,
    midori_color_filter::ColorFilter,
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState},
    service::HostServices,
    state::ClientId,
};
use crysirror_tasks::{Task, TaskPool};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, PartialEq)]
pub(super) struct Definition {
    slot: Option<usize>,
    layers: [i32; 2],
}

impl Definition {
    #[cfg(test)]
    pub fn parse(raw: &[u8]) -> Result<Self> {
        Self::parse_profile(raw, crate::profile::Resident::Midori2011)
    }

    pub fn parse_profile(raw: &[u8], resident: crate::profile::Resident) -> Result<Self> {
        use crate::{native_numbers::integer2011, view_definitions};
        let mut fields = raw.split(|&c| c == b',');
        let slot = integer2011(fields.next().unwrap_or_default());
        if !(0..=7).contains(&slot) {
            return Ok(Self { slot: None, layers: [-2; 2] });
        }
        let alias = integer2011(fields.next().unwrap_or_default());
        let layer = if resident.late() {
            view_definitions::layer_late(alias)?
        } else {
            view_definitions::layer2011(alias)?
        };
        let next = layer.wrapping_add(1);
        Ok(Self {
            slot: Some(slot as usize),
            layers: [layer.min(next), layer.max(next)],
        })
    }
}

const MIDORI_VIEW: [u32; 2] = [1024, 576];
const VIRGIN_PACKED_POS: i32 = 0x0101_0000;

// Native T205 has eight 12-word rows. The 2011 descriptor constructor
// initializes fields through the packed position only. Do not let a
// safe Rust initializer pretend its untouched raw X/Y words are known.
type DrawRow = [Option<i32>; 12];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Position {
    Absolute([i32; 2]),
    CenteredBottom(i32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Draw {
    slot: usize,
    actor: u8,
    body: u8,
    face: u8,
    position: Position,
    shown: bool,
}

impl Draw {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw
            .splitn(10, |&c| c == b',')
            .map(number)
            .collect::<Result<_>>()?;
        let [
            slot,
            actor,
            body,
            face,
            position,
            x,
            y,
            effect @ (253 | 255),
            0,
        ] = fields.as_slice()
        else {
            bail!("unsupported Midori actor position, effect, or duration")
        };
        ensure!(
            (0..=2).contains(slot)
                && (3..=36).contains(actor)
                && (0..36).contains(body)
                && (0..42).contains(face),
            "unsupported Midori actor slot, body, face, or geometry"
        );
        let position = match (*position, *x, *y, *effect) {
            (-12346, x, y, 255) if [x, y].into_iter().all(|n| (-8192..=8192).contains(&n)) => {
                Position::Absolute([x, y])
            }
            (
                p @ (0x0301_fe40 | 0x0301_fe60 | 0x0301_fe80 | 0x0301_fec0 | 0x0401_fe40
                | 0x0401_fec0),
                0,
                0,
                _,
            ) if *effect == 255 || p >> 24 == 4 => Position::CenteredBottom(p),
            _ => bail!("unsupported Midori actor position, effect, or duration"),
        };
        Ok(Self {
            slot: *slot as usize,
            actor: *actor as u8,
            body: *body as u8,
            face: *face as u8,
            position,
            shown: *effect == 255,
        })
    }

    fn authored_position(self) -> (i32, [i32; 2]) {
        match self.position {
            Position::Absolute(point) => (-12346, point),
            Position::CenteredBottom(word) => (word, [0, 0]),
        }
    }

    fn preflight(self, view: [u32; 2]) -> Result<()> {
        ensure!(
            matches!(self.position, Position::Absolute(_)) || view == MIDORI_VIEW,
            "unsupported Midori packed portrait valid rectangle"
        );
        Ok(())
    }

    pub fn origin(self, size: [u32; 2], correction: [i32; 2], view: [u32; 2]) -> Result<[i32; 2]> {
        self.preflight(view)?;
        // 0x38340 centered x and bottom y. The caller disables camera offsets;
        // p[15:0] is signed and size is the allocated body, not the face cell.
        Ok(match self.position {
            Position::Absolute(point) => point,
            Position::CenteredBottom(word) => [
                view[0] as i32 / 2 - size[0] as i32 / 2 + i32::from(word as i16) + correction[0],
                view[1] as i32 - size[1] as i32 + correction[1],
            ],
        })
    }

    #[cfg(test)]
    pub fn is_packed(self) -> bool {
        matches!(self.position, Position::CenteredBottom(_))
    }

    pub fn face_plan(self, catalog: &Catalog) -> Result<FacePlan> {
        catalog
            .body_2011(self.actor, self.body)
            .context("undefined Midori actor body")?
            .immediate_face(self.face)
    }
}

pub(super) fn clear_slot(raw: &[u8]) -> Result<usize> {
    let Some((slot, rest)) = raw.split_first() else {
        bail!("empty Midori actor clear")
    };
    ensure!(
        matches!(slot, b'0'..=b'2') && rest == b",0,255,-1,0,0,*",
        "unsupported Midori actor clear effect or flags"
    );
    Ok((slot - b'0') as usize)
}

struct Visible {
    index: usize,
    asset: String,
    description: Draw,
    origin: [i32; 2],
}

struct DrawSlot {
    layers: [i32; 2],
    active: Option<Visible>,
    last_authored_point: Option<[i32; 2]>,
}

#[derive(Clone, Copy)]
struct Prepared {
    draw: Draw,
    correction: [i32; 2],
}

enum Pending {
    Read(Prepared, Task<ImageInput>),
    Compose(Prepared, ImageJob),
}

#[derive(Default)]
pub(super) struct Slots {
    slots: [Option<DrawSlot>; 8],
    pending: Option<Pending>,
    view: [u32; 2],
    color_filter: ColorFilter,
}

fn layer_id(native: i32) -> u32 {
    256 + native as u32
}

fn submit(h: &mut dyn HostServices, c: ClientId, updates: Vec<Value>) -> Result<()> {
    h.submit_engine(c, 1, &serde_json::to_vec(&json!({"layers":updates}))?)?;
    Ok(())
}

impl Slots {
    pub fn for_view(view: [u32; 2]) -> Self {
        Self {
            view,
            ..Default::default()
        }
    }

    pub fn set_filter(&mut self, filter: ColorFilter) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "Midori character filter while a draw is pending"
        );
        // Mode zero does not recolor any published actor surface.
        self.color_filter = filter;
        Ok(())
    }

    pub fn capture_filter(&self, prior: &[u8]) -> Option<(u32, String)> {
        self.color_filter.capture(prior)
    }

    pub fn define(&mut self, definition: Definition) -> Result<()> {
        let Some(slot) = definition.slot.map(|index| &mut self.slots[index]) else {
            return Ok(());
        };
        ensure!(
            self.pending.is_none() && slot.as_ref().is_none_or(|s| s.active.is_none()),
            "unsupported Midori active actor draw-slot redefinition"
        );
        let last_authored_point = slot.as_ref().and_then(|slot| slot.last_authored_point);
        *slot = Some(DrawSlot {
            layers: definition.layers,
            active: None,
            last_authored_point,
        });
        Ok(())
    }

    #[cfg(test)]
    pub fn startup_layers(&self) -> [Option<[i32; 2]>; 3] {
        std::array::from_fn(|i| self.slots[i].as_ref().map(|slot| slot.layers))
    }

    /// 0x5c9a0/0x366e0: native lane/descriptor words followed by actual
    /// presented coordinates. Clear/redefine leave the authored X/Y pair
    /// unchanged. Never serialize the whole T205 register from these rows
    /// while any native words remain unknown.
    pub fn checkpoint_rows(&self) -> Option<[DrawRow; 8]> {
        if self.pending() {
            return None;
        }
        Some(std::array::from_fn(|i| {
            let slot = self.slots.get(i).and_then(Option::as_ref);
            let layers = slot.map_or([-2, -2], |s| s.layers);
            let raw = slot.and_then(|s| s.last_authored_point);
            let (mut row, origin) = match slot.and_then(|s| s.active.as_ref()) {
                Some(visible) => {
                    let draw = visible.description;
                    let (word, _) = draw.authored_position();
                    (
                        [
                            Some(0),
                            Some(layers[0]),
                            Some(layers[1]),
                            Some(visible.index as i32),
                            Some(i32::from(draw.actor)),
                            Some(i32::from(draw.body)),
                            Some(i32::from(draw.face)),
                            Some(word),
                            None,
                            None,
                            None,
                            None,
                        ],
                        visible.origin,
                    )
                }
                None => (
                    [
                        Some(0),
                        Some(layers[0]),
                        Some(layers[1]),
                        Some(-1),
                        Some(-1),
                        Some(-1),
                        Some(-1),
                        Some(VIRGIN_PACKED_POS),
                        None,
                        None,
                        None,
                        None,
                    ],
                    [0, 0],
                ),
            };
            [row[8], row[9]] = raw.map_or([None, None], |p| p.map(Some));
            [row[10], row[11]] = origin.map(Some);
            row
        }))
    }

    pub fn poses(&self) -> [Option<Value>; 3] {
        std::array::from_fn(|i| {
            let slot = self.slots[i].as_ref()?;
            let v = slot.active.as_ref()?;
            let (position_word, _) = v.description.authored_position();
            Some(json!({
                "lane":slot.layers[v.index], "origin":v.origin, "shown":v.description.shown,
                "actor":v.description.actor, "body":v.description.body, "face":v.description.face,
                "position_word": position_word,
            }))
        })
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn draw(
        &mut self,
        draw: Draw,
        catalog: &Catalog,
        h: &mut dyn HostServices,
        c: ClientId,
        worker: &TaskPool,
    ) -> Result<()> {
        ensure!(self.pending.is_none(), "overlapping Midori portrait jobs");
        let slot = self.slots[draw.slot]
            .as_ref()
            .context("undefined Midori draw slot")?;
        ensure!(slot.layers[0] >= 0, "invalid Midori portrait layer ID");
        draw.preflight(self.view)?;
        let plan = draw.face_plan(catalog)?;
        let prepared = Prepared {
            draw,
            correction: plan.correction,
        };
        let storage = h.storage(c)?;
        let filter = self.color_filter;
        self.pending = Some(Pending::Read(
            prepared,
            worker.spawn(move |cancel| {
                cancel.check()?;
                let input = plan.read(storage.as_ref(), filter)?;
                cancel.check()?;
                Ok(input)
            })?,
        ));
        Ok(())
    }

    pub fn present(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        let (prepared, image) = match self.pending.take() {
            None => return Ok(true),
            Some(Pending::Read(draw, mut task)) => {
                let Some(input) = task.try_take()? else {
                    self.pending = Some(Pending::Read(draw, task));
                    return Ok(false);
                };
                let job = h
                    .image_metadata(c)?
                    .begin_process(input, Arc::from(&b"[]"[..]))?;
                self.pending = Some(Pending::Compose(draw, job));
                return Ok(false);
            }
            Some(Pending::Compose(draw, mut job)) => match job.poll()? {
                ImageProcessState::Pending => {
                    self.pending = Some(Pending::Compose(draw, job));
                    return Ok(false);
                }
                ImageProcessState::Failed(error) => bail!("Midori actor composition: {error}"),
                ImageProcessState::Ready(image) => (draw, image),
            },
        };
        let draw = prepared.draw;
        let size = image.size();
        let origin = draw.origin(size, prepared.correction, self.view)?;
        let slot = self.slots[draw.slot]
            .as_mut()
            .context("missing Midori draw slot")?;
        // Native 0x36a40 selects the inactive member; mode255 disposes the old one.
        let index = slot.active.as_ref().map_or(0, |old| 1 - old.index);
        let native = slot.layers[index];
        let upload = h.publish_image_resource(c, layer_id(native), image)?;
        let asset = upload.path.relative().to_owned();
        let old = slot.active.as_ref();
        let mut ops = Vec::with_capacity(2);
        if let Some(old) = old {
            ops.push(json!({"op":"remove","id":layer_id(slot.layers[old.index])}));
        }
        ops.push(json!({"op":"put","layer":{
            "id":layer_id(native),"order":[2,native],
            "rect":[origin[0],origin[1],size[0],size[1]],
            "opacity":if draw.shown {255} else {0},
            "paint":{"kind":"image","asset":asset}
        }}));
        if let Err(error) = submit(h, c, ops) {
            let _ = h.release_image_resource(c, &asset);
            return Err(error);
        }
        slot.last_authored_point = Some(draw.authored_position().1);
        let old = slot.active.replace(Visible {
            index,
            asset,
            description: draw,
            origin,
        });
        if let Some(old) = old {
            h.release_image_resource(c, &old.asset)?;
        }
        Ok(true)
    }

    pub fn clear(&mut self, slot: usize, h: &mut dyn HostServices, c: ClientId) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "Midori actor clear while a draw is pending"
        );
        let slot_number = slot;
        let Some(slot) = self.slots[slot_number].as_mut() else {
            return Ok(());
        };
        if let Some(visible) = slot.active.as_ref() {
            debug_assert_eq!(visible.description.slot, slot_number);
            submit(
                h,
                c,
                vec![json!({"op":"remove","id":layer_id(slot.layers[visible.index])})],
            )?;
            let retired = slot
                .active
                .take()
                .context("missing Midori visible portrait")?;
            h.release_image_resource(c, &retired.asset)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Script};
    use std::{
        fs,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn bitmap(indices: &[u8], alpha: &[u8]) -> Vec<u8> {
        let stride = indices.len().next_multiple_of(4);
        let mut out = b"GYU\x1a".to_vec();
        out.extend((if alpha.is_empty() { 0u16 } else { 3u16 }).to_le_bytes());
        out.extend(0x100u16.to_le_bytes());
        for v in [
            u32::MAX,
            8,
            indices.len() as u32,
            1,
            stride as u32,
            if alpha.is_empty() { 0 } else { stride as u32 },
            256,
        ] {
            out.extend(v.to_le_bytes());
        }
        for n in 0..=255u8 {
            out.extend([10, n, n.wrapping_mul(2), 0]);
        }
        out.extend(indices);
        out.resize(out.len() + stride - indices.len(), 0);
        if !alpha.is_empty() {
            out.extend(alpha);
            out.resize(out.len() + stride - alpha.len(), 0);
        }
        out
    }

    fn record(op: u8, csv: &[u8]) -> Result<Action> {
        let mut rld = b"\0DLR".to_vec();
        for word in [3u32, 12, 1] {
            rld.extend(word.to_le_bytes());
        }
        rld.extend([op, 0, 0, 1]);
        rld.extend(csv);
        rld.push(0);
        Script::new(rld, 3)?.next()
    }

    fn draw_at(csv: &[u8]) -> Result<Draw> {
        let Action::DrawMidoriActor(draw) = record(0xfc, csv)? else {
            bail!("expected actor draw")
        };
        Ok(draw)
    }

    #[test]
    fn actor_new_layer_uses_requested_face_and_releases_the_old_lane_and_resource() -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "crysirror-midori-actor-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let assets = dir.join("assets/res/g/ch/00060000");
        fs::create_dir_all(&assets)?;
        let run = (|| -> Result<()> {
            let body_path = assets.join("00060005P000.gyu");
            let face_path = assets.join("00060005F.gyu");
            fs::write(body_path, bitmap(&[1, 2, 3, 4], &[3, 63, 0, 253]))?;
            let original_atlas = bitmap(&[6, 7, 8, 9], &[]);
            fs::write(&face_path, &original_atlas)?;
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"actor-assets-only","content_revision":"1","canvas":[1024,576],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut rt = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let c = ClientId(1);
            let worker = TaskPool::new("actor-image-test", 1, 1)?;
            let mut catalog = Catalog::new(37);
            catalog.define(crate::character::Definition::parse_2011(
                b"6,8,1,NULL,A,B,C,D,E,F,G,H,I,J",
            )?)?;
            let raw = crate::character_body::tests::payload_2011(6, 5);
            let mut fields: Vec<_> = raw.split(|&b| b == b',').collect();
            fields[4] = b"-7";
            fields[5] = b"-9";
            fields[20] = b"2";
            fields[21] = b"1";
            fields[22] = b"0";
            catalog.define_body_2011(crate::character_body::Definition2011::parse(
                &fields.join(&b','),
            )?)?;
            let mut slots = Slots::for_view(MIDORI_VIEW);
            for csv in [b"0,800", b"1,802", b"2,804"] {
                let Action::DefineMidoriActorSlot(def) = record(0xfb, csv)? else {
                    bail!("expected actor pair")
                };
                slots.define(def)?;
            }
            assert_eq!(
                slots.startup_layers(),
                [Some([137, 138]), Some([139, 140]), Some([141, 142])]
            );
            assert_eq!(
                slots.checkpoint_rows().unwrap()[0],
                [
                    Some(0),
                    Some(137),
                    Some(138),
                    Some(-1),
                    Some(-1),
                    Some(-1),
                    Some(-1),
                    Some(0x0101_0000),
                    None,
                    None,
                    Some(0),
                    Some(0)
                ]
            );
            assert_eq!(
                slots.checkpoint_rows().unwrap()[7],
                [
                    Some(0),
                    Some(-2),
                    Some(-2),
                    Some(-1),
                    Some(-1),
                    Some(-1),
                    Some(-1),
                    Some(0x0101_0000),
                    None,
                    None,
                    Some(0),
                    Some(0)
                ]
            );
            let finish = |slots: &mut Slots, h: &mut dyn HostServices| -> Result<()> {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !slots.present(h, c)? {
                    ensure!(Instant::now() < deadline, "Midori actor worker timeout");
                    std::thread::yield_now();
                }
                Ok(())
            };
            slots.draw(
                draw_at(b"0,6,5,0,-12346,-4,5,255,0")?,
                &catalog,
                &mut rt.host,
                c,
                &worker,
            )?;
            assert!(slots.clear(0, &mut rt.host, c).is_err());
            assert!(slots.checkpoint_rows().is_none());
            finish(&mut slots, &mut rt.host)?;
            let first = slots.slots[0].as_ref().unwrap().active.as_ref().unwrap();
            assert_eq!((first.index, first.origin), (0, [-4, 5]));
            assert_eq!(
                slots.checkpoint_rows().unwrap()[0].map(Option::unwrap),
                [0, 137, 138, 0, 6, 5, 0, -12346, -4, 5, -4, 5]
            );
            let first_path = first.asset.clone();
            assert!(rt.host.engine.layers.get(c, 393).is_some());
            assert!(rt.host.engine.layers.get(c, 394).is_none());
            let first_image = rt
                .host
                .published_image_resource(c, &first_path)?
                .context("missing first portrait")?;
            assert_eq!(
                &*first_image.rgba(),
                &[2, 1, 10, 3, 12, 6, 10, 63, 14, 7, 10, 0, 8, 4, 10, 253]
            );
            assert!(slots.define(Definition::parse(b"0,800")?).is_err());
            let invalid_cell = draw_at(b"0,6,5,2,-12346,-4,5,255,0")?;
            assert!(
                slots
                    .draw(invalid_cell, &catalog, &mut rt.host, c, &worker)
                    .is_err()
            );
            let Action::MidoriColorFilter(filter) = record(0xa7, b"0,16416,48")? else {
                bail!("expected character filter");
            };
            slots.set_filter(filter)?;
            assert_eq!(
                slots.capture_filter(b"stale"),
                Some((4, "0,16416,48".into()))
            );
            assert_eq!(
                &*rt.host
                    .published_image_resource(c, &first_path)?
                    .unwrap()
                    .rgba(),
                &*first_image.rgba(),
                "mode zero must not recolor the current live actor"
            );
            assert!(record(0xa7, b"1,3145728,50").is_err());
            let second_draw = draw_at(b"0,6,5,1,-12346,9,0,255,0")?;
            slots.draw(second_draw, &catalog, &mut rt.host, c, &worker)?;
            assert_eq!(
                slots.slots[0]
                    .as_ref()
                    .unwrap()
                    .active
                    .as_ref()
                    .unwrap()
                    .asset,
                first_path
            );
            finish(&mut slots, &mut rt.host)?;
            let second = slots.slots[0].as_ref().unwrap().active.as_ref().unwrap();
            assert_eq!((second.index, second.origin), (1, [9, 0]));
            assert_eq!(
                slots.checkpoint_rows().unwrap()[0].map(Option::unwrap),
                [0, 137, 138, 1, 6, 5, 1, -12346, 9, 0, 9, 0]
            );
            let second_path = second.asset.clone();
            assert_ne!(first_path, second_path);
            assert!(rt.host.engine.layers.get(c, 393).is_none());
            assert!(rt.host.engine.layers.get(c, 394).is_some());
            assert!(rt.host.published_image_resource(c, &first_path)?.is_none());
            let second_image = rt
                .host
                .published_image_resource(c, &second_path)?
                .context("missing second portrait")?;
            assert_eq!(
                &*second_image.rgba(),
                &[1, 12, 14, 3, 13, 18, 14, 63, 14, 19, 14, 0, 6, 15, 14, 253]
            );
            slots.set_filter(ColorFilter::deferred(b"0,0,0")?)?;
            assert_eq!(slots.capture_filter(b"stale"), Some((4, String::new())));
            assert_eq!(slots.capture_filter(b""), None);
            assert_eq!(
                &*rt.host
                    .published_image_resource(c, &second_path)?
                    .unwrap()
                    .rgba(),
                &*second_image.rgba()
            );
            // A second slot keeps its own layer when the first is replaced,
            // rejects a mismatched palette, then is removed.
            slots.draw(
                draw_at(b"1,6,5,0,-12346,20,0,255,0")?,
                &catalog,
                &mut rt.host,
                c,
                &worker,
            )?;
            finish(&mut slots, &mut rt.host)?;
            let other = slots.slots[1]
                .as_ref()
                .unwrap()
                .active
                .as_ref()
                .unwrap()
                .asset
                .clone();
            assert_eq!(
                &*rt.host.published_image_resource(c, &other)?.unwrap().rgba(),
                &[2, 1, 10, 3, 12, 6, 10, 63, 14, 7, 10, 0, 8, 4, 10, 253]
            );
            let mut invalid_atlas = original_atlas.clone();
            invalid_atlas[36 + 4 * 8] ^= 127;
            fs::write(&face_path, invalid_atlas)?;
            let stable_capture = slots.checkpoint_rows();
            slots.draw(second_draw, &catalog, &mut rt.host, c, &worker)?;
            assert!(slots.checkpoint_rows().is_none());
            let error =
                finish(&mut slots, &mut rt.host).expect_err("bad indexed palette must fail");
            assert!(format!("{error:#}").contains("palette differs"));
            assert!(!slots.pending());
            assert_eq!(slots.checkpoint_rows(), stable_capture);
            assert_eq!(
                slots.slots[0]
                    .as_ref()
                    .unwrap()
                    .active
                    .as_ref()
                    .unwrap()
                    .asset,
                second_path
            );
            let Action::ClearMidoriActor(clear) = record(0xfd, b"0,0,255,-1,0,0,*")? else {
                bail!("expected clear")
            };
            slots.clear(clear, &mut rt.host, c)?;
            assert!(slots.slots[0].as_ref().unwrap().active.is_none());
            assert!(rt.host.engine.layers.get(c, 394).is_none());
            assert!(rt.host.engine.layers.get(c, 395).is_some());
            assert!(rt.host.published_image_resource(c, &second_path)?.is_none());
            assert!(rt.host.published_image_resource(c, &other)?.is_some());
            let after_clear = slots.checkpoint_rows().unwrap();
            assert_eq!(
                after_clear[0].map(Option::unwrap),
                [0, 137, 138, -1, -1, -1, -1, 0x0101_0000, 9, 0, 0, 0]
            );
            assert_eq!(after_clear[1], stable_capture.unwrap()[1]);
            slots.define(Definition::parse(b"0,800")?)?;
            slots.clear(clear, &mut rt.host, c)?;
            assert_eq!(slots.checkpoint_rows().unwrap(), after_clear);
            fs::write(&face_path, original_atlas)?;
            let hidden = draw_at(b"0,6,5,1,67239616,0,0,253,0")?;
            // Native first bottom placement uses signed -320 and current
            // body-specific corrections, independent of the prior absolute pose.
            assert!(hidden.origin([4, 1], [-7, -9], [800, 600]).is_err());
            slots.draw(hidden, &catalog, &mut rt.host, c, &worker)?;
            finish(&mut slots, &mut rt.host)?;
            let layer = rt
                .host
                .engine
                .layers
                .get(c, 393)
                .context("missing retained hidden lane")?;
            assert_eq!(layer.rect, [183.0, 566.0, 4.0, 1.0]);
            assert_eq!(layer.opacity, 0);
            assert_eq!(
                slots.checkpoint_rows().unwrap()[0].map(Option::unwrap),
                [0, 137, 138, 0, 6, 5, 1, 67239616, 0, 0, 183, 566]
            );
            assert_eq!(slots.poses()[0].as_ref().unwrap()["shown"], false);
            assert_eq!(
                slots.poses()[0].as_ref().unwrap()["position_word"],
                67239616
            );
            let shown = draw_at(b"0,6,5,0,50462336,0,0,255,0")?;
            // Odd widths must truncate before the negative offset is applied.
            assert_eq!(shown.origin([461, 847], [4, 666], MIDORI_VIEW)?, [-98, 395]);
            slots.draw(shown, &catalog, &mut rt.host, c, &worker)?;
            finish(&mut slots, &mut rt.host)?;
            assert!(rt.host.engine.layers.get(c, 393).is_none());
            let layer = rt
                .host
                .engine
                .layers
                .get(c, 394)
                .context("missing newly shown packed lane")?;
            assert_eq!(layer.rect, [119.0, 566.0, 4.0, 1.0]);
            assert_eq!(layer.opacity, 255);
            assert_eq!(
                slots.checkpoint_rows().unwrap()[0].map(Option::unwrap),
                [0, 137, 138, 1, 6, 5, 0, 50462336, 0, 0, 119, 566]
            );
            assert!(rt.host.engine.layers.get(c, 395).is_some());
            assert!(draw_at(b"0,6,5,0,50462336,1,0,255,0").is_err());
            assert!(draw_at(b"0,6,5,0,50462336,0,0,253,0").is_err());
            for bad in [
                b"0,6,5,0,-12345,-4,5,255,0".as_slice(),
                b"0,6,5,0,-12346,-4,5,9,0",
                b"0,6,5,0,-12346,-4,5,255,200",
            ] {
                assert!(record(0xfc, bad).is_err());
            }
            assert!(record(0xfd, b"0,0,9,200,0,0,*").is_err());
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        run
    }
}
