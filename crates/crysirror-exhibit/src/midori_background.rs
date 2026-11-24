// SPDX-License-Identifier: MPL-2.0
//! Midori full-canvas background and its native draw register image.
use crate::registers::{Bank, Change, Registers};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    image_processing::{
        ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageJob, ImageProcessState,
        ImageResource,
    },
    path::{Mount, VfsPath},
    service::{ClientStorage, HostServices},
    state::ClientId,
};
use crysirror_tasks::{Task, TaskPool};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) const VIEW: [u32; 2] = [1024, 576];
const LAYER: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Draw {
    id: i32,
    pub duration: u64,
    pub crop: Option<[i32; 2]>,
}

impl Draw {
    pub fn full_canvas(flags: u8, args: &[i32]) -> Result<Self> {
        let [
            id,
            mode,
            0,
            ms,
            0,
            0,
            0,
            x,
            y,
            w,
            h,
            crop_flag,
            ratio_y,
            flags_word,
            0,
            0,
        ] = args
        else {
            bail!("unsupported Midori background effect or crop");
        };
        ensure!(
            flags == 0 && ((0..=9999).contains(id) || *id as u32 & 0xff00_0000 == 0x8000_0000),
            "unsupported Midori background operand"
        );
        ensure!(
            (*mode == 255 && *ms == 0) || (*mode == 9 && (1..=600_000).contains(ms)),
            "unsupported Midori background effect or duration"
        );
        ensure!(
            (*crop_flag, *ratio_y) == (-1, -1),
            "unsupported Midori background scale"
        );
        let crop = match (*x, *y, *w, *h, *flags_word) {
            (0, 0, 0, 0, 0) => None,
            (x, y, 1024, 576, 65536)
                if *id >= 0 && (0..=4096).contains(&x) && (0..=4096).contains(&y) =>
            {
                Some([x, y])
            }
            _ => bail!("unsupported Midori retained source rectangle"),
        };
        Ok(Self {
            crop,
            id: *id,
            duration: *ms as u64,
        })
    }

    pub fn asset(self) -> Option<String> {
        (self.id >= 0).then(|| format!("res/g/ev/{:02}/{:04}.gyu", self.id / 100, self.id))
    }

    fn solid(self) -> [u8; 4] {
        let [b, g, r, _] = self.id.to_le_bytes();
        [r, g, b, 255]
    }

    pub fn read(self, storage: &dyn ClientStorage) -> Result<ImageInput> {
        let name = self.asset().context("Midori background has no GYU file")?;
        let path = VfsPath::new(Mount::Asset, &name)?;
        let read = storage.open_read(&path)?;
        let bytes = crysirror_resource::read_all(read.as_ref(), 16 << 20)?;
        let size = self.inspect(&bytes)?;
        Ok(ImageInput::Encoded {
            name,
            bytes: Arc::from(bytes),
            size,
        })
    }

    pub fn inspect(self, bytes: &[u8]) -> Result<[u32; 2]> {
        let [size, depth] = raw_header(bytes)?;
        if let Some(point) = self.crop {
            ensure!(
                depth[0] == 24 && dims_contain(size, point),
                "unsupported Midori retained CG source geometry"
            );
        } else {
            ensure!(
                size == VIEW,
                "unsupported Midori full-canvas GYU background size"
            );
        }
        Ok(size)
    }

    fn checkpoint(self) -> [i32; 19] {
        // 0x505f0 updates the live background after draw completes. 0x57760
        // projects that descriptor into R10..R28; the materialized background
        // is not the native 16x16 sentinel. 0x50670 zeros the color state
        // that the other, still unimplemented modes can populate.
        let mut registers = [0; 19];
        registers[0] = self.id;
        registers[1] = 0x10000;
        if let Some([x, y]) = self.crop {
            registers[2..6].copy_from_slice(&[x, y, VIEW[0] as i32, VIEW[1] as i32]);
        }
        registers[8] = -1;
        registers[9] = -1;
        registers[11] = (VIEW[0] | (VIEW[1] << 16)) as i32;
        registers
    }
}

fn raw_header(bytes: &[u8]) -> Result<[[u32; 2]; 2]> {
    let header = bytes.get(..36).context("short Midori background GYU")?;
    let word = |i| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
    let mode = u16::from_le_bytes([header[6], header[7]]);
    let depth = word(12);
    let size = [word(16), word(20)];
    ensure!(
        &header[..4] == b"GYU\x1a"
            && header[4..6] == [0, 0]
            && matches!(mode, 0x0100 | 0x0400 | 0x0800)
            && word(8) != 0
            && size.into_iter().all(|side| (1..=4096).contains(&side))
            && word(24) > 0
            && word(28) == 0
            && matches!((depth, word(32)), (8, 256) | (24, 0)),
        "unsupported Midori full-canvas opaque GYU background"
    );
    Ok([size, [depth, word(32)]])
}

fn dims_contain(size: [u32; 2], origin: [i32; 2]) -> bool {
    (0..2).all(|axis| {
        origin[axis] >= 0
            && size[axis] >= VIEW[axis]
            && origin[axis] as u32 <= size[axis] - VIEW[axis]
    })
}

pub(super) fn crop_job(
    host: &mut dyn HostServices,
    client: ClientId,
    source: Image,
    origin: [i32; 2],
) -> Result<ImageJob> {
    ensure!(
        source.opaque() && dims_contain(source.size(), origin),
        "Midori crop outside retained background"
    );
    let [x, y] = origin;
    let input = ImageInput::Solid {
        size: VIEW,
        color: [0, 0, 0, 255],
    }
    .with_composite_layer(ImageCompositeLayer {
        input: Box::new(ImageInput::Resource(source)),
        source: [x, y, VIEW[0] as i32, VIEW[1] as i32],
        destination: [0, 0],
        clip: None,
        opacity: 255,
        math: ImageCompositeMath::Copy,
    });
    Ok(host
        .image_metadata(client)?
        .begin_process(input, Arc::from(&b"[]"[..]))?)
}

enum Pending {
    Reading(Draw, Task<ImageInput>),
    Decoding(Draw, ImageJob),
    Cropping(Draw, Image, ImageJob),
    Panning(Pan),
    Fading(Fade),
}

struct Live {
    asset: Option<String>,
    draw: Draw,
    frame: Frame,
    current: Option<[i32; 2]>,
}

impl Live {
    fn input(&self) -> ImageInput {
        input(self.draw, self.frame.shown.clone())
    }
}

type Image = Arc<dyn ImageResource>;

#[derive(Clone, Default)]
struct Frame {
    shown: Option<Image>,
    backing: Option<Image>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MoveCrop {
    to: [i32; 2],
    duration: u32,
    curve: i32,
}

impl MoveCrop {
    pub fn read(flags: u8, args: &[i32]) -> Result<Self> {
        let [x, y, 1024, 576, ms, 0, curve @ (0 | 2), 0] = args else {
            bail!("unsupported Midori magnification scale, blur or HQ pass");
        };
        ensure!(
            flags == 0
                && (1..=600_000).contains(ms)
                && [x, y].into_iter().all(|v| (0..=4096).contains(v)),
            "unsupported Midori pan operands"
        );
        Ok(Self {
            to: [*x, *y],
            duration: *ms as u32,
            curve: *curve,
        })
    }
}

struct Pan {
    motion: MoveCrop,
    start: u64,
    from: [i32; 2],
    shown: [i32; 2],
    source: Image,
    scratch: Option<String>,
    job: Option<([i32; 2], ImageJob)>,
}

impl Pan {
    fn state(&self, now: u64) -> (bool, [i32; 2]) {
        let elapsed = now
            .saturating_sub(self.start)
            .min(u64::from(self.motion.duration)) as u32;
        let at = std::array::from_fn(|axis| {
            self.from[axis]
                + crate::clip::offset(
                    self.motion.to[axis] - self.from[axis],
                    elapsed,
                    self.motion.duration,
                    self.motion.curve,
                )
        });
        (elapsed == self.motion.duration, at)
    }
}

fn input(draw: Draw, image: Option<Image>) -> ImageInput {
    image.map_or_else(
        || ImageInput::Solid {
            size: VIEW,
            color: draw.solid(),
        },
        ImageInput::Resource,
    )
}

struct Fade {
    draw: Draw,
    to: Frame,
    started: u64,
    step: u8,
    job: Option<(u8, ImageJob)>,
    scratch: Option<String>,
}

impl Fade {
    fn phase(&self, now: u64) -> u8 {
        let elapsed = now.saturating_sub(self.started).min(self.draw.duration);
        (elapsed * 16 / self.draw.duration) as u8
    }
    fn next_at(&self) -> u64 {
        self.started
            .saturating_add((u64::from(self.step + 1) * self.draw.duration).div_ceil(16))
    }
}

#[derive(Default)]
pub(super) struct Background {
    live: Option<Live>,
    pending: Option<Pending>,
}

impl Background {
    pub fn capture(&self) -> Option<(u32, [i32; 19])> {
        self.live.as_ref().map(|live| (10, live.draw.checkpoint()))
    }

    pub fn preparing(&self) -> bool {
        match &self.pending {
            None => false,
            Some(Pending::Fading(fade)) => fade.job.is_some(),
            Some(Pending::Panning(pan)) => pan.job.is_some(),
            Some(_) => true,
        }
    }

    pub fn wait_until(&self, now: u64) -> Option<u64> {
        match &self.pending {
            Some(Pending::Fading(fade)) if fade.job.is_none() && fade.phase(now) == fade.step => {
                Some(fade.next_at())
            }
            Some(Pending::Panning(pan))
                if pan.job.is_none() && !pan.state(now).0 && pan.state(now).1 == pan.shown =>
            {
                Some(now.saturating_add(1))
            }
            _ => None,
        }
    }

    pub fn pan(&mut self, motion: MoveCrop, now: u64) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "overlapping Midori background operations"
        );
        let live = self.live.as_ref().context("Midori pan has no live CG")?;
        let from = live.current.context("Midori pan has no retained CG crop")?;
        let source = live
            .frame
            .backing
            .clone()
            .context("Midori pan has no retained source")?;
        ensure!(
            dims_contain(source.size(), motion.to),
            "Midori pan destination outside retained CG"
        );
        self.pending = Some(Pending::Panning(Pan {
            motion,
            start: now,
            from,
            shown: from,
            source,
            scratch: None,
            job: None,
        }));
        Ok(())
    }

    pub fn current_crop(&self) -> Option<[i32; 2]> {
        self.live.as_ref().and_then(|live| live.current)
    }

    pub fn issue(
        &mut self,
        draw: Draw,
        host: &mut dyn HostServices,
        client: ClientId,
        worker: &TaskPool,
        registers: &mut Registers,
    ) -> Result<bool> {
        ensure!(self.pending.is_none(), "overlapping Midori backgrounds");
        ensure!(
            registers.configured(),
            "missing Midori background register dimensions"
        );
        if draw.duration != 0 {
            ensure!(
                self.live.is_some(),
                "Midori background fade has no prior live background"
            );
        }
        if draw.id < 0 {
            let now = host.now_ms();
            return self.prepared(draw, Frame::default(), host, client, registers, now);
        }
        let storage = host.storage(client)?;
        self.pending = Some(Pending::Reading(
            draw,
            worker.spawn(move |cancel| {
                cancel.check()?;
                let input = draw.read(storage.as_ref())?;
                cancel.check()?;
                Ok(input)
            })?,
        ));
        Ok(true)
    }

    pub fn poll(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        registers: &mut Registers,
    ) -> Result<bool> {
        let now = host.now_ms();
        self.poll_at(host, client, registers, now)
    }

    fn poll_at(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        registers: &mut Registers,
        now: u64,
    ) -> Result<bool> {
        match self.pending.take() {
            None => Ok(true),
            Some(Pending::Reading(draw, mut task)) => {
                match task.try_take()? {
                    Some(input) => {
                        let image = host
                            .image_metadata(client)?
                            .begin_process(input, Arc::from(&b"[]"[..]))?;
                        self.pending = Some(Pending::Decoding(draw, image));
                    }
                    None => self.pending = Some(Pending::Reading(draw, task)),
                }
                Ok(false)
            }
            Some(Pending::Decoding(draw, mut job)) => match job.poll()? {
                ImageProcessState::Pending => {
                    self.pending = Some(Pending::Decoding(draw, job));
                    Ok(false)
                }
                ImageProcessState::Failed(e) => bail!("Midori background decode: {e}"),
                ImageProcessState::Ready(image) => {
                    let backing = if let Some(origin) = draw.crop {
                        if origin != [0, 0] || image.size() != VIEW {
                            let crop = crop_job(host, client, image.clone(), origin)?;
                            self.pending = Some(Pending::Cropping(draw, image, crop));
                            return Ok(false);
                        }
                        Some(image.clone())
                    } else {
                        None
                    };
                    Ok(!self.prepared(
                        draw,
                        Frame {
                            shown: Some(image),
                            backing,
                        },
                        host,
                        client,
                        registers,
                        now,
                    )?)
                }
            },
            Some(Pending::Cropping(draw, backing, mut job)) => match job.poll()? {
                ImageProcessState::Pending => {
                    self.pending = Some(Pending::Cropping(draw, backing, job));
                    Ok(false)
                }
                ImageProcessState::Failed(e) => bail!("Midori retained background crop: {e}"),
                ImageProcessState::Ready(image) => Ok(!self.prepared(
                    draw,
                    Frame {
                        shown: Some(image),
                        backing: Some(backing),
                    },
                    host,
                    client,
                    registers,
                    now,
                )?),
            },
            Some(Pending::Panning(mut pan)) => match self.poll_pan(&mut pan, host, client, now) {
                Ok(true) => Ok(true),
                Ok(false) => {
                    self.pending = Some(Pending::Panning(pan));
                    Ok(false)
                }
                Err(e) => {
                    self.rollback(host, client, pan.scratch.take())?;
                    Err(e)
                }
            },
            Some(Pending::Fading(mut fade)) => {
                match self.poll_fade(&mut fade, host, client, registers, now) {
                    Ok(true) => Ok(true),
                    Ok(false) => {
                        self.pending = Some(Pending::Fading(fade));
                        Ok(false)
                    }
                    Err(error) => {
                        self.rollback(host, client, fade.scratch.take())?;
                        Err(error)
                    }
                }
            }
        }
    }

    fn rollback(
        &self,
        host: &mut dyn HostServices,
        client: ClientId,
        scratch: Option<String>,
    ) -> Result<()> {
        if scratch.is_some() {
            let old = self
                .live
                .as_ref()
                .context("missing stable Midori back layer")?;
            submit(host, client, paint(old.draw, old.asset.as_deref()))?;
        }
        release(host, client, scratch)
    }

    fn poll_pan(
        &mut self,
        pan: &mut Pan,
        host: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<bool> {
        let (end, target) = pan.state(now);
        let live = self
            .live
            .as_mut()
            .context("missing Midori retained background")?;
        if let Some((point, mut job)) = pan.job.take() {
            if point == target {
                match job.poll()? {
                    ImageProcessState::Pending => {
                        pan.job = Some((point, job));
                        return Ok(false);
                    }
                    ImageProcessState::Failed(e) => bail!("Midori background pan: {e}"),
                    ImageProcessState::Ready(image) => {
                        let path = publish(host, client, live.draw, image.clone())?;
                        pan.shown = target;
                        if end {
                            release(host, client, pan.scratch.take())?;
                            release(host, client, live.asset.replace(path))?;
                            live.frame.shown = Some(image);
                            live.current = Some(target);
                            return Ok(true);
                        }
                        release(host, client, pan.scratch.replace(path))?;
                        return Ok(false);
                    }
                }
            }
        }
        if target != pan.shown || end && pan.scratch.is_some() {
            // At the endpoint republish from immutable source and roll the
            // scratch into committed state. No source is recropped in place.
            pan.job = Some((target, crop_job(host, client, pan.source.clone(), target)?));
            return Ok(false);
        }
        Ok(end)
    }

    fn prepared(
        &mut self,
        draw: Draw,
        frame: Frame,
        host: &mut dyn HostServices,
        client: ClientId,
        registers: &mut Registers,
        now: u64,
    ) -> Result<bool> {
        if let Some(image) = frame.shown.as_ref() {
            ensure!(
                image.size() == VIEW && image.opaque(),
                "invalid Midori opaque background"
            );
        }
        if draw.duration == 0 {
            self.commit(draw, frame, host, client, registers)?;
            Ok(false)
        } else {
            self.pending = Some(Pending::Fading(Fade {
                draw,
                to: frame,
                started: now,
                step: 0,
                job: None,
                scratch: None,
            }));
            Ok(true)
        }
    }

    fn poll_fade(
        &mut self,
        fade: &mut Fade,
        host: &mut dyn HostServices,
        client: ClientId,
        registers: &mut Registers,
        now: u64,
    ) -> Result<bool> {
        let target = fade.phase(now);
        if target == 16 {
            self.commit(fade.draw, fade.to.clone(), host, client, registers)?;
            release(host, client, fade.scratch.take())?;
            return Ok(true);
        }
        if let Some((step, mut job)) = fade.job.take() {
            if step == target {
                match job.poll()? {
                    ImageProcessState::Pending => fade.job = Some((step, job)),
                    ImageProcessState::Failed(e) => bail!("Midori background fade: {e}"),
                    ImageProcessState::Ready(image) => {
                        let path = host
                            .publish_image_resource(client, LAYER, image)?
                            .path
                            .relative()
                            .to_owned();
                        if let Err(error) = submit(host, client, paint(fade.draw, Some(&path))) {
                            let _ = host.release_image_resource(client, &path);
                            return Err(error);
                        }
                        let old = fade.scratch.replace(path);
                        fade.step = step;
                        release(host, client, old)?;
                    }
                }
                return Ok(false);
            }
            // The engine clock advanced while a worker owned an old step.
            // Dropping its handle cancels it; never publish stale blend data.
        }
        if target > fade.step {
            let from = self
                .live
                .as_ref()
                .context("missing prior Midori background")?
                .input();
            let to = input(fade.draw, fade.to.shown.clone());
            // TmAsyncDraw mode9 computes floor(elapsed*16/duration). The
            // opaque 0x1651d0 kernel sums s*step + d*(16-step) then shifts 4;
            // weight=step*16 is exactly that kernel on the shared byte path.
            let blend = from.with_composite_layer(ImageCompositeLayer {
                input: Box::new(to),
                source: [0, 0, VIEW[0] as i32, VIEW[1] as i32],
                destination: [0, 0],
                clip: None,
                opacity: 255,
                math: ImageCompositeMath::MaskPiecewise256 {
                    weight: u16::from(target) * 16,
                },
            });
            let job = host
                .image_metadata(client)?
                .begin_process(blend, Arc::from(&b"[]"[..]))?;
            fade.job = Some((target, job));
        }
        Ok(false)
    }

    fn commit(
        &mut self,
        draw: Draw,
        frame: Frame,
        host: &mut dyn HostServices,
        client: ClientId,
        registers: &mut Registers,
    ) -> Result<()> {
        let asset = if let Some(image) = frame.shown.clone() {
            Some(publish(host, client, draw, image)?)
        } else {
            submit(host, client, paint(draw, None))?;
            None
        };
        let old = self.live.replace(Live {
            asset,
            draw,
            frame,
            current: draw.crop,
        });
        if draw.id >= 0 {
            registers.change(Change::Experience(Bank::Cg, vec![draw.id]))?;
        }
        release(host, client, old.and_then(|live| live.asset))?;
        Ok(())
    }
}

fn publish(
    host: &mut dyn HostServices,
    client: ClientId,
    draw: Draw,
    image: Image,
) -> Result<String> {
    let path = host
        .publish_image_resource(client, LAYER, image)?
        .path
        .relative()
        .to_owned();
    if let Err(error) = submit(host, client, paint(draw, Some(&path))) {
        let _ = host.release_image_resource(client, &path);
        return Err(error);
    }
    Ok(path)
}

fn release(host: &mut dyn HostServices, client: ClientId, path: Option<String>) -> Result<()> {
    if let Some(p) = path {
        host.release_image_resource(client, &p)?;
    }
    Ok(())
}

fn paint(draw: Draw, asset: Option<&str>) -> Value {
    if let Some(asset) = asset {
        json!({"kind":"image","asset":asset})
    } else {
        json!({"kind":"solid","color":draw.solid()})
    }
}

fn submit(host: &mut dyn HostServices, client: ClientId, paint: Value) -> Result<()> {
    host.submit_engine(
        client,
        1,
        &serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{
            "id": LAYER, "order": [0,0], "rect": [0,0,VIEW[0],VIEW[1]], "paint":paint,
        }}]}))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Script, native_state_tests};
    use std::{
        fs,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn opcode_with_duration(id: i32, mode: i32, width: i32, ms: i32) -> Result<Draw> {
        let mut script = b"\0DLR".to_vec();
        for word in [3u32, 12, 1] {
            script.extend(word.to_le_bytes());
        }
        script.extend([0x4a, 0, 16, 0]);
        for arg in [id, mode, 0, ms, 0, 0, 0, 0, 0, width, 0, -1, -1, 0, 0, 0] {
            script.extend(arg.to_le_bytes());
        }
        let Action::DrawMidoriBackground(draw) = Script::new(script, 3)?.next()? else {
            bail!("expected v3 immediate background");
        };
        Ok(draw)
    }

    fn opcode(id: i32, mode: i32, width: i32) -> Result<Draw> {
        opcode_with_duration(id, mode, width, 0)
    }

    fn bitmap() -> Vec<u8> {
        let mut out = b"GYU\x1a".to_vec();
        out.extend([0, 0, 0, 1]);
        let color_bytes = 3 * VIEW[0] * VIEW[1];
        for word in [u32::MAX, 24, VIEW[0], VIEW[1], color_bytes, 0, 0] {
            out.extend(word.to_le_bytes());
        }
        for _ in 0..color_bytes / 3 {
            out.extend([9, 77, 202]);
        }
        out
    }

    #[test]
    fn immediate_background_commits_render_layer_and_checkpoint_together() -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "exhibit-midori-background-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let assets = dir.join("assets/res/g/ev/00");
        fs::create_dir_all(&assets)?;
        let result = (|| -> Result<()> {
            let bitmap = bitmap();
            fs::write(assets.join("0007.gyu"), &bitmap)?;
            fs::write(assets.join("0008.gyu"), &bitmap[..36])?;
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"background-assets-only","content_revision":"1","canvas":[1024,576],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut runtime = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let mut owner = Background::default();
            let mut regs = native_state_tests::fresh(3)?;
            let worker = TaskPool::new("background-test", 1, 1)?;
            let client = ClientId(1);
            let deadline = Instant::now() + Duration::from_secs(10);
            let poll = |owner: &mut Background,
                        host: &mut dyn HostServices,
                        regs: &mut Registers|
             -> Result<()> {
                while !owner.poll(host, client, regs)? {
                    ensure!(Instant::now() < deadline, "Midori background test deadline");
                    std::thread::yield_now();
                }
                Ok(())
            };
            let solid = opcode(0x80cafe07u32 as i32, 255, 0)?;
            let image = opcode(7, 255, 0)?;
            assert!(opcode(8, 9, 0).is_err());
            assert!(opcode(8, 255, 1024).is_err());
            assert!(opcode(-1, 255, 0).is_err());
            assert!(owner.capture().is_none());
            assert!(!owner.issue(solid, &mut runtime.host, client, &worker, &mut regs)?);
            let layer = runtime
                .host
                .engine
                .layers
                .get(client, LAYER)
                .context("missing solid layer")?;
            assert_eq!(
                serde_json::to_value(&layer.paint)?,
                json!({"kind":"solid","color":[202,254,7,255]})
            );
            let solid_capture = owner.capture().unwrap();
            assert_eq!(solid_capture.1[0], 0x80cafe07u32 as i32);

            assert!(owner.issue(image, &mut runtime.host, client, &worker, &mut regs)?);
            assert_eq!(owner.capture(), Some(solid_capture));
            assert_eq!(regs.capture_experience()?.cell(4 + 7), Some(0));
            poll(&mut owner, &mut runtime.host, &mut regs)?;
            assert_eq!(
                owner.capture(),
                Some((
                    10,
                    [
                        7, 65536, 0, 0, 0, 0, 0, 0, -1, -1, 0, 37749760, 0, 0, 0, 0, 0, 0, 0
                    ]
                ))
            );
            let first_path = owner.live.as_ref().unwrap().asset.clone().unwrap();
            let pixels = runtime
                .host
                .published_image_resource(client, &first_path)?
                .unwrap();
            assert!(
                pixels
                    .rgba()
                    .chunks_exact(4)
                    .all(|c| c == [202, 77, 9, 255])
            );
            assert_eq!(regs.capture_experience()?.cell(4 + 7), Some(1));
            assert_eq!(regs.read(crate::registers::Value::Local(10))?, 0);

            assert!(owner.issue(
                opcode(8, 255, 0)?,
                &mut runtime.host,
                client,
                &worker,
                &mut regs
            )?);
            assert!(poll(&mut owner, &mut runtime.host, &mut regs).is_err());
            assert_eq!(owner.capture().unwrap().1[0], 7);
            assert!(
                runtime
                    .host
                    .published_image_resource(client, &first_path)?
                    .is_some()
            );
            assert_eq!(regs.capture_experience()?.cell(4 + 8), Some(0));

            assert!(owner.issue(image, &mut runtime.host, client, &worker, &mut regs)?);
            poll(&mut owner, &mut runtime.host, &mut regs)?;
            let second_path = owner.live.as_ref().unwrap().asset.clone().unwrap();
            assert_ne!(first_path, second_path);
            assert!(
                runtime
                    .host
                    .published_image_resource(client, &first_path)?
                    .is_none()
            );
            assert!(
                runtime
                    .host
                    .published_image_resource(client, &second_path)?
                    .is_some()
            );
            assert_eq!(regs.capture_experience()?.cell(4 + 7), Some(2));
            assert!(!owner.issue(solid, &mut runtime.host, client, &worker, &mut regs)?);
            assert!(
                runtime
                    .host
                    .published_image_resource(client, &second_path)?
                    .is_none()
            );
            assert_eq!(owner.capture(), Some(solid_capture));
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }

    #[test]
    fn timed_background_uses_native_steps_from_stable_source_without_touching_overlay() -> Result<()>
    {
        let dir = std::env::temp_dir().join(format!(
            "exhibit-midori-background-fade-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let assets = dir.join("assets/res/g/ev/00");
        fs::create_dir_all(&assets)?;
        let result = (|| -> Result<()> {
            fs::write(assets.join("0007.gyu"), bitmap())?;
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"background-fade-assets-only","content_revision":"1","canvas":[1024,576],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut runtime = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let mut owner = Background::default();
            let mut regs = native_state_tests::fresh(3)?;
            let worker = TaskPool::new("background-fade-test", 1, 1)?;
            let client = ClientId(1);
            let deadline = Instant::now() + Duration::from_secs(10);
            let fade = opcode_with_duration(7, 9, 0, 101)?;
            assert!(
                owner
                    .issue(fade, &mut runtime.host, client, &worker, &mut regs)
                    .is_err()
            );
            let solid = opcode(0x80cafe07u32 as i32, 255, 0)?;
            owner.issue(solid, &mut runtime.host, client, &worker, &mut regs)?;
            runtime.host.submit_engine(
                client,
                1,
                br#"{"layers":[{"op":"put","layer":{
                "id":256,"order":[2,5],"rect":[5,6,7,8],"paint":{"kind":"solid","color":[8,7,6,255]}
            }}]}"#,
            )?;
            let old = owner.capture();
            assert!(owner.issue(fade, &mut runtime.host, client, &worker, &mut regs)?);
            let start = 1700;
            while owner.wait_until(start).is_none() {
                assert!(!owner.poll_at(&mut runtime.host, client, &mut regs, start)?);
                ensure!(
                    Instant::now() < deadline,
                    "Midori background fade decode deadline"
                );
                std::thread::yield_now();
            }
            assert_eq!(owner.wait_until(start), Some(start + 7));
            assert!(!owner.preparing());
            assert_eq!(owner.capture(), old);
            assert_eq!(regs.capture_experience()?.cell(4 + 7), Some(0));

            let verify_step = |owner: &mut Background,
                               runtime: &mut crysirror_runtime::Runtime,
                               regs: &mut Registers,
                               elapsed: u64,
                               step: u8,
                               rgba: [u8; 4]|
             -> Result<String> {
                let now = start + elapsed;
                while owner.wait_until(now).is_none() {
                    assert!(!owner.poll_at(&mut runtime.host, client, regs, now)?);
                    ensure!(
                        Instant::now() < deadline,
                        "Midori background fade blend deadline"
                    );
                    std::thread::yield_now();
                }
                let Some(Pending::Fading(fade)) = &owner.pending else {
                    bail!("lost fade")
                };
                assert_eq!(fade.step, step);
                let path = fade
                    .scratch
                    .as_ref()
                    .context("missing published fade frame")?
                    .clone();
                let pixels = runtime
                    .host
                    .published_image_resource(client, &path)?
                    .unwrap();
                assert!(pixels.rgba().chunks_exact(4).all(|p| p == rgba));
                Ok(path)
            };
            // Queue step 1 then advance immediately. The worker output may
            // finish, but its old step must not be published after time moves on.
            assert!(!owner.poll_at(&mut runtime.host, client, &mut regs, start + 7)?);
            assert!(owner.preparing());
            let path = verify_step(
                &mut owner,
                &mut runtime,
                &mut regs,
                27,
                4,
                [202, 209, 7, 255],
            )?;
            assert_eq!(owner.wait_until(start + 27), Some(start + 32));
            let next = verify_step(
                &mut owner,
                &mut runtime,
                &mut regs,
                70,
                11,
                [202, 132, 8, 255],
            )?;
            assert!(
                runtime
                    .host
                    .published_image_resource(client, &path)?
                    .is_none()
            );
            assert_eq!(owner.capture(), old);
            assert_eq!(regs.capture_experience()?.cell(4 + 7), Some(0));
            assert!(owner.poll_at(&mut runtime.host, client, &mut regs, start + 101)?);
            assert!(
                runtime
                    .host
                    .published_image_resource(client, &next)?
                    .is_none()
            );
            assert_eq!(owner.capture().unwrap().1[0], 7);
            assert_eq!(regs.capture_experience()?.cell(4 + 7), Some(1));
            let final_path = owner.live.as_ref().unwrap().asset.as_ref().unwrap();
            assert!(
                runtime
                    .host
                    .published_image_resource(client, final_path)?
                    .unwrap()
                    .rgba()
                    .chunks_exact(4)
                    .all(|p| p == [202, 77, 9, 255])
            );
            let overlay = runtime
                .host
                .engine
                .layers
                .get(client, 256)
                .context("lost foreground overlay")?;
            assert_eq!(overlay.rect, [5.0, 6.0, 7.0, 8.0]);
            assert_eq!(
                serde_json::to_value(&overlay.paint)?,
                json!({"kind":"solid","color":[8,7,6,255]})
            );
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }

    #[test]
    fn retained_pan_copies_from_immutable_backing_and_preserves_authored_draw_capture() -> Result<()>
    {
        let dir = std::env::temp_dir().join(format!(
            "exhibit-midori-pan-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let assets = dir.join("assets/res/g/ev/00");
        fs::create_dir_all(&assets)?;
        let result = (|| -> Result<()> {
            let rgb = |x: u32, y: u32| [x as u8, y as u8, (3 * x + 5 * y) as u8, 255];
            let [w, h] = [1040u32, 592u32];
            let mut image = b"GYU\x1a".to_vec();
            image.extend([0, 0, 0, 1]);
            for v in [u32::MAX, 24, w, h, w * h * 3, 0, 0] {
                image.extend(v.to_le_bytes());
            }
            for y in (0..h).rev() {
                for x in 0..w {
                    let [r, g, b, _] = rgb(x, y);
                    image.extend([b, g, r]);
                }
            }
            fs::write(assets.join("0007.gyu"), &image)?;
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"background-pan-assets-only","content_revision":"1","canvas":[1024,576],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut runtime = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let c = ClientId(1);
            let worker = TaskPool::new("background-pan-test", 1, 1)?;
            let mut owner = Background::default();
            let mut regs = native_state_tests::fresh(3)?;
            let deadline = Instant::now() + Duration::from_secs(10);
            let move_record = |x: i32, y: i32, curve: i32| -> Result<MoveCrop> {
                let mut rld = b"\0DLR".to_vec();
                for word in [3u32, 12, 1] {
                    rld.extend(word.to_le_bytes());
                }
                rld.extend([0x50, 0, 8, 0]);
                for word in [x, y, 1024, 576, 100, 0, curve, 0] {
                    rld.extend(word.to_le_bytes());
                }
                let Action::PanMidoriBackground(pan) = Script::new(rld, 3)?.next()? else {
                    bail!("missing Midori pan")
                };
                Ok(pan)
            };
            assert!(owner.pan(move_record(13, 14, 2)?, 0).is_err());
            assert!(MoveCrop::read(0, &[13, 14, 1024, 576, 100, 1, 2, 0]).is_err());
            assert!(MoveCrop::read(0, &[13, 14, 1024, 576, 100, 0, 2, 1]).is_err());
            assert!(MoveCrop::read(0, &[13, 14, 1024, 578, 100, 0, 2, 0]).is_err());
            let draw = Draw::full_canvas(
                0,
                &[7, 255, 0, 0, 0, 0, 0, 3, 4, 1024, 576, -1, -1, 65536, 0, 0],
            )?;
            assert!(owner.issue(draw, &mut runtime.host, c, &worker, &mut regs)?);
            while !owner.poll_at(&mut runtime.host, c, &mut regs, 0)? {
                ensure!(Instant::now() < deadline, "Midori crop preparation timeout");
                std::thread::yield_now();
            }
            let capture = owner.capture();
            assert_eq!(&capture.unwrap().1[2..6], &[3, 4, 1024, 576]);
            assert_eq!(owner.current_crop(), Some([3, 4]));
            let verify = |runtime: &mut crysirror_runtime::Runtime,
                          path: &str,
                          point: [u32; 2]|
             -> Result<()> {
                let resource = runtime
                    .host
                    .published_image_resource(c, path)?
                    .context("missing Midori pan image")?;
                let actual = resource.rgba();
                for (i, pix) in actual.chunks_exact(4).enumerate() {
                    assert_eq!(
                        pix,
                        rgb(point[0] + (i % 1024) as u32, point[1] + (i / 1024) as u32)
                    );
                }
                Ok(())
            };
            let old = owner.live.as_ref().unwrap().asset.clone().unwrap();
            verify(&mut runtime, &old, [3, 4])?;
            assert!(owner.pan(move_record(17, 14, 2)?, 100).is_err());
            owner.pan(move_record(13, 14, 2)?, 100)?;
            while owner.wait_until(125).is_none() {
                assert!(!owner.poll_at(&mut runtime.host, c, &mut regs, 125)?);
                ensure!(Instant::now() < deadline, "Midori pan interim timeout");
                std::thread::yield_now();
            }
            let Some(Pending::Panning(pan)) = &owner.pending else {
                bail!("missing pending pan")
            };
            let scratch = pan.scratch.clone().unwrap();
            verify(&mut runtime, &scratch, [8, 9])?;
            assert_eq!(owner.current_crop(), Some([3, 4]));
            while !owner.poll_at(&mut runtime.host, c, &mut regs, 200)? {
                ensure!(Instant::now() < deadline, "Midori pan completion timeout");
                std::thread::yield_now();
            }
            assert!(runtime.host.published_image_resource(c, &old)?.is_none());
            assert!(
                runtime
                    .host
                    .published_image_resource(c, &scratch)?
                    .is_none()
            );
            verify(
                &mut runtime,
                owner.live.as_ref().unwrap().asset.as_ref().unwrap(),
                [13, 14],
            )?;
            assert_eq!(owner.current_crop(), Some([13, 14]));
            assert_eq!(capture, owner.capture());
            assert_eq!(regs.capture_experience()?.cell(11), Some(1));

            owner.pan(move_record(3, 4, 0)?, 250)?;
            while owner.wait_until(300).is_none() {
                assert!(!owner.poll_at(&mut runtime.host, c, &mut regs, 300)?);
                ensure!(Instant::now() < deadline, "Midori chained pan timeout");
                std::thread::yield_now();
            }
            let Some(Pending::Panning(pan)) = &owner.pending else {
                bail!("missing chained pan")
            };
            verify(&mut runtime, pan.scratch.as_ref().unwrap(), [8, 9])?;
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }
}
